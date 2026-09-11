# Design — the restore's recovery code gets a non-interactive path

- **Date:** 2026-09-11
- **Closes:** [#572](https://github.com/cairn-ehr/cairn-ehr/issues/572) — *restore's recovery-code
  prompt has no non-interactive path, so a DR drill cannot be scripted or run unattended.* Filed from
  the #512 measurement run and confirmed empirically there.
- **Then closes:** [#570](https://github.com/cairn-ehr/cairn-ehr/issues/570) — *the restore CLI
  surface is untested.* The two are the same wall: a CLI test cannot drive an `rpassword` prompt
  without a pseudo-terminal either, which is why that surface has no tests. #572 is what makes #570
  writable, so they ride one branch in that order.
- **Produces:** one optional flag on `Cmd::Restore`; a new pure module
  `crates/cairn-node/src/restore/recovery_code.rs`; one ADR; **no migration, no `SCHEMA_GENERATION`
  bump, no wire-format change.** The event core is untouched.
- **Touches one crate** (`cairn-node`) plus `scripts/measure_dr_restore.py`. The gate is still the
  full workspace, because the branch changes a binary whose `restore --help` surface an integration
  test pins (`restore_needs_nothing_about_the_dead_node.rs`).
- **Predecessors, not re-derived here:** [ADR-0026](../../spec/decisions/0026-node-durability-and-disaster-recovery.md)
  (the restore ceremony), [ADR-0066](../../spec/decisions/0066-identity-dies-with-the-disk-custody-must-not.md)
  (the local-state export carries the dead node's unwrap secret, which is what the recovery code
  opens), [ADR-0067](../../spec/decisions/0067-a-restore-reads-the-clinical-plane.md) and
  [ADR-0068](../../spec/decisions/0068-provenance-warns-never-gates-on-the-restore-path.md)
  (provenance warns, never gates — the reason there is no other interactive read anywhere in the
  restore arm, which §2.2 below depends on).

---

## 1. Why this piece exists

The one command whose correctness matters most is the one that cannot be exercised.

`restore` needs two secrets and treats them unalike. The passphrase for the **new** sealed key has
`--passphrase` and `CAIRN_KEY_PASSPHRASE`. The **old** node's recovery code, which unseals the
local-state export and is therefore the thing that returns the dead node's custody, has no flag and no
environment variable at all. It is read through `rpassword::prompt_password`, which opens `/dev/tty`
and fails on any non-tty.

A piped code does not merely get ignored. The read errors, the export never opens, and the restore
finishes having recovered **zero patients** while exiting non-zero — #500's own signature, arriving
inside the mechanism built to prevent it. Under a pseudo-terminal the identical inputs restore
everything and the bodies open.

The consequence is that a clinic cannot rehearse. A quarterly drill that confirms the medium still
restores has to be done by hand every time, and the clinics that will not do it by hand are the ones
that most need the answer. The `scripts/measure_dr_restore.py` rig allocates a pseudo-terminal purely
to work around this, which is scaffolding that exists only because of the gap.

**Scope, decided with the maintainer (2026-09-11).** The automation this serves is a **rehearsal
drill into a throwaway node**, not an unattended production restore. That is what sets §4's boundary.

---

## 2. Two findings that shaped the design

Both came out of reading the restore arm before designing against it. The second reversed a decision
that had already been taken in conversation.

### 2.1 The prompt lands after the door is fenced, and the retry loop exists because of it

`unseal_local_state_with_retries`'s own doc says it plainly: the prompt lands **after**
`finalize_identity` has written `local_node`, and a second restore into an enrolled database is
refused. A single mistyped character therefore used to cost the node its custody key outright.

The retry budget is the mitigation, not a cure. It is why this design reads a supplied code at
**step 0**, in the pre-flight block whose stated purpose is exactly this — two checks already live
there because "by the time that code runs `finalize_identity` has fenced the restore door closed
[…] there is no free second attempt to notice a problem on."

A drill script pointed at a path that does not exist should cost nothing. Today the equivalent
mistake costs an identity and a database.

### 2.2 `print_recovery_code` is ALREADY reachable unattended, so a refusal keyed on this flag buys nothing

The first version of this design refused `--old-recovery-code-file` together with a sealed new key, on
the reasoning that it would mechanically preserve #527/#562's triage note — *"no cron-run command
reaches `print_recovery_code`"* — rather than let that note expire silently.

That reasoning is wrong, and the note is **already false**:

- `restore` calls `print_recovery_code` at `main.rs` step 4 whenever it mints a sealed key.
- A medium with **no local-state export sibling** never reaches the recovery-code prompt at all,
  because `apply_local_state_export` is only entered when `read_optional_sibling` found bytes.
- Every other read in the restore arm between its start and that print is **print-only**. That is not
  an accident of the current code; it is ADR-0068's ruling that provenance warns and never gates.

So today, with `CAIRN_KEY_PASSPHRASE` set and `--insecure-plaintext` omitted, a restore of a
federation-only medium runs start to finish with no tty, mints a sealed key, and prints a fresh
recovery code to stderr. Cron can already reach it.

Three consequences, and they are why the refusal was dropped:

1. **It would guard one path and leave the open one open.** The exposure it names arrives by a route
   the flag does not appear on.
2. **It keys on the wrong signal.** The flag is not a proxy for *unattended*. An operator standing at
   the terminal reading a code off a USB stick would be refused, while the genuinely unattended path
   above is untouched.
3. **It would cost the measurement rig the ceremony it exists to measure.** The rig restores into a
   **sealed** key deliberately, and its stated subject is a restore "as an operator would run it".
   Under the refusal it would have to keep the pseudo-terminal that #572 exists to remove, or add
   `--insecure-plaintext` and stop measuring the real ceremony.

**What replaces it** is §4's warning plus a separately-filed issue for the actual fix, which must cover
both paths rather than one.

---

## 3. The decision

`cairn-node restore` gains one optional argument:

```
--old-recovery-code-file <PATH>
```

Read the OLD node's recovery code from a file instead of prompting for it. Nothing else changes: with
the flag absent, the ceremony is bit-for-bit what it is today.

### 3.1 Why a file, and not a flag value or an environment variable

The issue's own table is the argument. The two secrets are not alike:

| | Passphrase for the NEW key | The OLD node's recovery code |
|---|---|---|
| Invented at restore time | yes | **no** |
| Retained off-node | no | **yes, it is the only such artifact** |
| What it opens, with the medium beside it | the new node's own key | **the clinic's whole clinical record** |

`CAIRN_KEY_PASSPHRASE` is a defensible exposure for a secret the operator makes up at the keyboard and
which protects a key that has not existed for ten seconds. The recovery code plus the medium sitting
next to it is the whole record in the clear. Consistency is not a strong enough reason to give both the
same treatment.

A path keeps the secret:

- off the **process table** — `ps auxww` shows the path, never the code;
- out of **shell history**;
- out of the **environment**, so it is not in `/proc/<pid>/environ`, not inherited by children, and not
  in a crash dump.

It also composes for free. A tmpfs path, a named pipe, and `/dev/stdin` all work with no extra code,
which covers the drill, the measurement rig and #570's tests without a `-` sentinel or a second
mechanism. File permissions become the control, and an off-site recovery code is already a written
artifact, so a file is its natural shape.

### 3.2 Why the name says "old"

This command **also mints and prints a new recovery code**. A flag called `--recovery-code-file` on
such a command invites the reading "this is where the new code goes", which is a footgun on the one
ceremony with no second attempt.

This is not hypothetical. The #512 rig matched the string `"recovery code"` to find the prompt, and the
**new** node's shown-once banner satisfied it two steps early, so the rig typed its answer into the
terminal minutes before the real prompt and worked only as type-ahead. The word `old` is what already
disambiguates in this CLI, in `rpassword`'s prompt strings, and in neither banner.

It also leaves `--new-recovery-code-file` free as the consistent name for the sink §4's follow-up issue
proposes, rather than stranding that fix with a name that does not pair.

---

## 4. The exposure this does not fix, said plainly

When a sealed key is minted and **stderr is not a terminal**, `restore` prints one additional line
saying that a secret has just been written to this stream and that whatever captured it must be treated
as secret.

Keying on `std::io::IsTerminal` rather than on the presence of the flag is deliberate, and it follows
directly from §2.2. The honest question is *"will a human see this code?"*, not *"which flags were
passed?"*, and the honest question also covers the federation-only path that has been open all along.
`IsTerminal` is std, stable, and adds no dependency.

**This warning is not a guarantee and the ADR must say so.** It tells an operator what happened; it
does not stop it. The real fix is a place to put the new code that is not the process's stderr — a
`--new-recovery-code-file <PATH>` sink, or a refusal to mint a sealed key when nothing can show its
code to a human. That is a second secret-handling decision, it is not what #572 asks for, and folding
it in here would put two of them in one ADR. **It is filed as its own issue by this slice** (house
rule 5: fix it or file it, never let a known defect pass silently), and #527/#562's triage note is
corrected in the same breath, because it is already false rather than about to become false.

---

## 5. The shape

### 5.1 A new module, because every neighbour is already over the limit

`crates/cairn-node/src/restore/recovery_code.rs`. The alternatives are all full:
`main.rs` is 6405 lines, `restore.rs` is 601, `restore/clinical.rs` is 620. House rule 4 says a file
this design touches should not be the one that grows.

Putting the pure functions in the **library** rather than in a `#[cfg(test)]` block inside the binary
also makes their tests real integration tests. `Cmd` is defined in a binary crate, which is the wall
this crate keeps hitting: an integration test cannot import it, which is why `--help` gets driven as a
subprocess. Pure functions in the lib have no such problem.

### 5.2 What is pure, and therefore directly testable

```rust
/// Read and validate an operator-supplied recovery code.
pub fn read_recovery_code_file(path: &Path) -> Result<Zeroizing<String>, RecoveryCodeError>

/// How many times the unseal loop may ask, given where the code comes from.
pub fn recovery_code_attempts(supplied: bool) -> usize

/// The warning printed when a freshly-minted code lands somewhere no human is reading.
pub fn minted_code_exposure_warning() -> String
```

`read_recovery_code_file` refuses two inputs and the distinction matters:

- an **unreadable path** (missing, permissions, a mount that went away) — the operator's most likely
  scripting error;
- content that is **empty or whitespace-only**. This one is not cosmetic. `normalize_recovery_code`
  strips all spacing and case before the unwrap, so a file holding `"   "` normalizes to the empty
  string and would attempt an unseal under an effectively empty secret. `establish-local-state-key`
  already guards exactly this, for exactly this reason, and its comment is the precedent to follow.

A trailing newline is **tolerated**, not an error: `printf '%s\n' "$CODE" > file` is how anyone would
write one, and normalization strips it anyway. Refusing it would be a trap with no upside.

The return is `Zeroizing<String>` all the way, matching every other secret in this file (issue #46).

### 5.3 Where each piece is wired

**Step 0, the pre-flight block.** If the flag is present, read and validate the file there, before a
byte is minted. Two further honest touches at the same site:

- The flag with **no export sibling present** is inert. Warn, do not fail: a drill script pointed at
  the wrong medium would otherwise pass while exercising none of the path it exists to exercise.
- The flag together with an export that is present but **unreadable** already bails on the existing
  `SiblingRead::Unreadable` arm, and that ordering is kept.

**The unseal loop.** `unseal_local_state_with_retries` already takes an injected
`ask: impl FnMut(usize) -> Result<Zeroizing<String>>`, so no new machinery is needed. A supplied code
is asked **once**, because re-reading a file cannot change the answer, and a budget of three would
print "2 attempt(s) left" about a file. A prompted code keeps `RECOVERY_CODE_ATTEMPTS`. That is the
whole content of `recovery_code_attempts`.

**Step 4.** After `print_recovery_code`, emit §4's warning when stderr is not a terminal.

### 5.4 What deliberately does not change

- The ceremony order. `restore_ceremony_order.rs` pins it and stays green.
- The attended path. With no flag, every prompt, every retry and every message is what it is today.
- `apply_local_state_export`'s two return shapes and their meanings.
- The event core, the wire format, the schema. There is no migration in this slice.

---

## 6. Testing

TDD throughout: the failing test first, then the code. Grouped by what they pin.

**Pure, no database, no tty** (`crates/cairn-node/src/restore/recovery_code.rs` unit tests plus a new
integration test):

1. A file holding a code returns it.
2. A trailing newline is tolerated.
3. A whitespace-only file is refused, and the error names why rather than saying "empty".
4. A missing path is refused, and the error is distinguishable from (3).
5. `recovery_code_attempts(true) == 1`, `recovery_code_attempts(false) == RECOVERY_CODE_ATTEMPTS`.
6. The exposure warning's text names the stream and does not claim to have prevented anything.

**The `--help` surface** (`restore_needs_nothing_about_the_dead_node.rs`):

7. The **secret half of #512's budget, now pinnable.** That file's module doc currently says the secret
   half is "deliberately NOT pinned here, because today it would pin a defect", and enumerates the
   defect: the recovery code "has **no flag and no environment variable at all**". This slice makes
   that paragraph false, so the doc is rewritten and the pin is added: the retained secret has a
   non-interactive path, and it is **optional**, so `required_flags` is unchanged and a solo clinic
   still supplies nothing but `--conn` and `--from`.

**The CLI, driven as a subprocess** (a new `restore_cli_surface.rs`, DB-gated) — this is #570:

8. A restore with `--old-recovery-code-file` and no pseudo-terminal **restores the clinical plane and
   the bodies open**. This is the headline: it is the assertion `restore_reads_the_clinical_plane.rs`
   makes, driven through the shipped command rather than the library, and it is what proves #572 is
   actually closed. Counting rows is not enough, for the reason 2d's review round already established.
9. The **non-zero exit** on an incomplete clinical restore, driven with a deliberately-refusable
   record. `main.rs`'s comment says "a script must see that" and nothing asserts it.
10. **Warning reachability**, which the existing text-grepping guard cannot prove: the AEAD caveat
    print, the untrusted-clinical warning's call site, and the gap and duplicate notices. Gate any of
    those blocks behind `if false` and the current guard stays green.
11. A wrong code in the file degrades exactly as a wrong typed code does: warn, skip local-state,
    restore stands, non-zero exit.

**The registry encoder** (#570 item 3):

12. `actor_registry_rows_to_json` round-trips through the Rust path with a content assertion, covering
    `op = "supersede"` with `superseded_by` and a non-null `pinned` (ADR-0029's agent-actor
    determinant). Today the SQL mirror uses hand-written JSON that bypasses the encoder entirely, so
    `recorded_at` travels `TIMESTAMPTZ → ::text → String → JSON → ::TIMESTAMPTZ` with no assertion at
    any point, and a precision or timezone bug would re-authorise a revoked clinician through the door
    built to restore the registry.

**The rig, as proof rather than as a test:**

13. `scripts/measure_dr_restore.py` drops its pseudo-terminal and passes the code by file. Its "why the
    pseudo-terminal is here" section becomes the record of a closed gap. The published figure is **not**
    re-measured and not restated: this slice changes how a secret arrives, not what a restore costs, and
    re-running the curve is not owed. If the rig's own suite has a mutant that survives the change, that
    is a finding.

---

## 7. What is still broken when this merges

Named rather than assumed away, in the house style:

- **The freshly-minted recovery code still goes to stderr**, on this path and on the federation-only
  path that has been open all along. §4's warning reports it; nothing prevents it. Filed by this slice.
- **#527/#562's triage note is corrected, not vindicated.** *"No cron-run command reaches
  `print_recovery_code`"* was already untrue before this slice. The corrected note must say so, because
  a future reader who finds the sentence and this slice's date will otherwise conclude that this slice
  broke it.
- **#567** is untouched: `verify-backup`'s OK is still federation-only, so a green verify still says
  nothing about the plane a restore now applies.
- **#568** is untouched: `do_requeue`'s custody-carrying arm still has no test, and every test call
  site still passes `None`.
- **#569** is untouched: db/052's registry door still discards a content conflict silently.
- **The eight unwritten §7 design-test pins from 2d** are not written by this slice. #570 is a
  different list.

---

## Paper-parity benchmark (§1.2)

**Paper counterpart:** a practice restoring its records from an off-site copy, and a practice
*rehearsing* that restore to confirm the copy is still good.

**Step count.** The three numbers behind [#512](https://github.com/cairn-ehr/cairn-ehr/issues/512) are
**unchanged by this slice** and are not re-derived here: paper *N* = 2, architecture-forced *M* = 3,
UI bundling target *K* = 2. This slice adds **no human act**. In the attended ceremony the flag is
absent and every act is what it was. In the drill it *replaces* one human act with a file read, so the
count moves down rather than up. `M > N` still stands, #512 stays open, and the excess act is the one
that session identified: the old node's recovery code is a second, separately-prompted secret, asked
for after the node plane is already applied.

**Time budget.** Not re-measured, and deliberately so. The measured figure is 116.7 s against a 600 s
budget, linear at 1.17 ms/event with no bend
(`crates/cairn-node/results/2026-09-10-macos-m3max.md`). Reading a secret from a file instead of a
terminal does not touch the per-event cost that figure is about. Re-running the curve would restate a
number this slice did not change, which is how a results file starts to drift from what it measured.

**What this slice does move, and it is the point.** #512's budget says a restore completes "within 10
minutes **unattended** after the operator's last keystroke". Before this slice that word could not be
taken at face value: there was no way to reach the last keystroke without a human at a terminal. After
it, a drill genuinely runs unattended, which is the first time the budget's own wording is
operationally true.
