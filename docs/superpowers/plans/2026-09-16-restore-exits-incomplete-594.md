# A restore that left records behind exits INCOMPLETE (#594) — Implementation Plan

Issue: [#594](https://github.com/cairn-ehr/cairn-ehr/issues/594) · decided by the maintainer
2026-09-15 (**option 2**), widened 2026-09-16 (see *The widening* below).

## What this builds, in one sentence

`cairn-node restore` grows a third exit status — **3 (INCOMPLETE)**, the code `cairn-sync requeue`
already uses — printed after its whole summary, whenever any record the medium carried is not in
this node's log when the command finishes; **1 stays FAILED** and **0 stays "everything applicable
came back"**.

## The decision, and what it reverses

**Maintainer decision (2026-09-15), on #594:** option 2. Exit **3** whenever any record on the medium
was not restored — records past a mid-file chain break, records in a plane this build cannot route,
**and a torn tail** — printed after the full summary.

It does **not** conflict with [ADR-0068](../../spec/decisions/0068-provenance-warns-never-gates-on-the-restore-path.md)
decision 1 ("refusing converts a partial loss into a total one"): that ruling is about **gating**, and
a non-zero exit taken *after* applying everything applicable refuses nothing. What it does reverse is a
narrower, code-level ruling: the **exit-0-on-a-torn-medium** pin in `restore_torn_medium_cli.rs`,
which was 2c round 2's. That test's exit assertion inverts here; every other assertion in it stands
(the prefix still restores, the WARNING still prints, the summary still repeats the tear).

## The widening (maintainer, 2026-09-16)

#594's text names three causes. `restore` already had **two more** arms that exit **1** for the same
kind of state — records **PENNED** (recoverable by `requeue`) and **NO ACTOR REGISTRY** (recoverable
by a second restore) — and both of their messages already say *"this exit code says the restore is
INCOMPLETE, not that it failed"*. `requeue.rs`'s own doc comment names the gap: restore "has only exit
1 to say it with".

Building only the three named causes would have left the signal **inverted**: a monitoring script
would read the MORE recoverable outcome (a pen, which `requeue` empties) as FAILED=1, and the LESS
recoverable one (records past a chain break, which no retry of anything recovers) as INCOMPLETE=3.
Asked, the maintainer chose **one INCOMPLETE code for all five**.

### The exit vocabulary this lands

| Outcome | Records left unrestored | Recoverable by | Before | After |
|---|---|---|---|---|
| Clean restore | none | — | 0 | **0** |
| Torn tail | yes, lost from this copy | nothing | 0 | **3** |
| Past a mid-file chain break | yes, on the medium | nothing (never offered) | 0 | **3** |
| A plane this build cannot route | yes, on the medium | a newer build | 0 | **3** |
| Records penned | yes, in the pen with their custody | `cairn-sync requeue` | 1 | **3** |
| No actor registry | the whole clinical plane | a second restore, fresh DB | 1 | **3** |
| Local-state bundle refused | key material not installed | recover the export | 1 | **1** |

**Precedence: FAILED (1) outranks INCOMPLETE (3).** A refused local-state bundle is checked first and
still `return Err(e)`. The line that justifies it is already in `main.rs` — *"a refused local-state
bundle means recovered key material was not installed — that is a failure, and scripts must see it as
one"* — and it is a different claim from "the restore did everything it safely could". A wrong or
absent recovery code blocks the ceremony; it is not a restore that finished with work left over.

## Global Constraints

- **TDD.** Every behaviour change is driven by a failing test first. Five of the tests here are
  **inversions or sharpenings of pins over shipped behaviour**, so they pass on their first run against
  the OLD code only if written wrong — each must be seen RED against `main` before the code moves.
- **No migration, no schema change.** `SCHEMA_GENERATION` stays **52**. Nothing in `db/` is touched.
- **No new dependency.**
- **One source of truth for the number 3.** `cairn-sync` already depends on `cairn-node`; the constant
  moves DOWN to `cairn-node` and `cairn_sync::requeue::EXIT_INCOMPLETE` becomes a compile-time alias of
  it, so the two binaries can never drift. `requeue`'s own doc comment stays where it is.
- **The verdict is a pure function over five scalars**, unit-tested with no database and no spawned
  binary; `main.rs` only prints what it returns and exits. (House rule 1 + the §9 reviewer-legibility
  rule: the decision of what "incomplete" means must be readable in one screen.)
- **`std::process::exit` skips destructors**, so stdout is flushed by hand — copy `cairn-sync`'s idiom
  at `main.rs:4808` verbatim, including its reason.
- **Every remedy currently printed must still be printed.** The two `anyhow::bail!` messages carry
  remedies; moving them off the `Err` path must not drop a sentence an operator mid-disaster needs.

## File map

| File | Change |
|---|---|
| `crates/cairn-node/src/restore/completeness.rs` | **NEW.** `EXIT_INCOMPLETE`, `Unrestored`, `Unrestored::is_complete`, `Unrestored::notice`, unit tests. Pure; no DB. |
| `crates/cairn-node/src/restore.rs` | `pub mod completeness;` |
| `crates/cairn-node/src/main.rs` | Build `Unrestored` from the restore arm's own locals; replace the two `bail!`s with the notice + `process::exit(3)`; keep `local_state_failure` first. |
| `crates/cairn-sync/src/requeue.rs` | `EXIT_INCOMPLETE` becomes an alias of `cairn_node::restore::completeness::EXIT_INCOMPLETE`; doc comment loses the "only exit 1 to say it with" clause, which stops being true. |
| `crates/cairn-node/tests/restore_exit_vocabulary.rs` | **NEW.** The one test that pins the two constants equal, and the no-DB unit-level table of the verdict. |
| `crates/cairn-node/tests/restore_cli_applies_nothing_untrusted.rs` | Tests 14 and 17b: the `// Exit status deliberately NOT asserted: #594` lines become assertions of 3; the file header's "deliberately NOT asserted" paragraph is rewritten. |
| `crates/cairn-node/tests/restore_torn_medium_cli.rs` | The exit-0 pin **inverts** to 3; its header records that it was 2c's ruling and that #594 reversed it. |
| `crates/cairn-node/tests/restore_cli_surface.rs` | The no-registry and pen assertions sharpen from `!success()` to `code() == Some(3)`; the two recovery-code failures sharpen to `Some(1)`, pinning the precedence. |
| `docs/spec/decisions/0071-a-restore-that-left-records-behind-exits-incomplete.md` | **NEW** ADR. |
| `docs/spec/decisions/README.md`, `docs/spec/index.md` | ADR index row; spec version **0.72 → 0.73**. |
| `docs/spec/backup-and-recovery.md` (or wherever the restore's operator contract lives) | The exit vocabulary, stated once. |
| `docs/HANDOVER.md`, `docs/ROADMAP.md` | Current state. |

---

### Task 0: Tracking documents current, and the test environment

1. `docs/HANDOVER.md` and `docs/ROADMAP.md` are verified against `main` (done at session start: PR
   #601 merged, `main` at `46a84ff`, nothing unmerged on any branch).
2. Resolve the cluster with `scripts/pg-target.sh` (**not** a hard-coded port) and export
   `CAIRN_TEST_PG*`. A narrow `cargo test` uses a scratch `CARGO_TARGET_DIR` so rust-analyzer's lock
   on the shared `target/` cannot stall it.
3. Record the baseline: the five CLI tests that will change all pass against `main` **today**.

### Task 1: The verdict — a pure module, unit-tested, no database

**RED first.** Write `crates/cairn-node/tests/restore_exit_vocabulary.rs` with the table below; it does
not compile until the module exists, which is the red phase for a new type.

`crates/cairn-node/src/restore/completeness.rs`:

```rust
/// The exit status of a restore that finished its ceremony but did not finish the recovery.
pub const EXIT_INCOMPLETE: i32 = 3;

/// What a finished `restore` left behind — one field per way a record can fail to reach the log.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Unrestored {
    pub past_chain_break: usize,
    pub unknown_plane: usize,
    pub torn_tail: bool,
    pub penned: usize,
    pub no_registry: bool,
}

impl Unrestored {
    pub fn is_complete(&self) -> bool { … }
    /// `None` when complete; else the operator notice, naming every cause and its remedy.
    pub fn notice(&self) -> Option<String> { … }
}
```

Unit tests (all pure, no `#[tokio::test]`, no `cs()` gate):

| Case | `is_complete` | `notice` must name |
|---|---|---|
| `Unrestored::default()` | `true` | — (`None`) |
| `past_chain_break: 2` | `false` | the count, "chain", "never offered" |
| `unknown_plane: 5` | `false` | the count, "upgrade" |
| `torn_tail: true` | `false` | "TORN" |
| `penned: 3` | `false` | the count, "`cairn-sync requeue`" |
| `no_registry: true` | `false` | "registry", "restore again" |
| all five at once | `false` | all five causes in one notice |
| any cause | `false` | the literal string `INCOMPLETE` and the literal `3` |

**The vacuity trap this closes:** a `notice()` that returned `Some("INCOMPLETE")` for everything would
pass a test that only checks `is_complete`. Each row asserts the **cause-specific** substring, so a
notice that forgot one cause fails at that row.

`penned == 0 && penned_but_acked > 0` is **not** a cause: an acked row is a recorded human decision that
those bytes never enter the record, and `penned()` already counts it (its `NOTE` line stays on stdout).
Named in the module doc so nobody "fixes" it.

### Task 2: `main.rs` — the restore arm reports its verdict

**RED first:** sharpen `restore_cli_surface.rs`'s no-registry (`line ~367`) and pen (`line ~444`)
assertions from `!out.status.success()` to `out.status.code() == Some(3)`. Both go red (they exit 1
today). Sharpen the two recovery-code failures (`~198`, `~314`) to `Some(1)`; those pass immediately —
they are **precedence pins**, and their mutation is in Task 5.

Then, at the tail of the `Cmd::Restore` arm, after the last `println!` and after
`if let Some(e) = local_state_failure { return Err(e); }`:

```rust
// FAILED (1) outranks INCOMPLETE (3) and is checked above: a refused local-state bundle is a
// ceremony that was blocked, not one that finished with work left over.
let unrestored = Unrestored {
    past_chain_break: if untrusted_notice.is_some() { clinical_plane.gated_out } else { 0 },
    unknown_plane: counts.unknown,
    torn_tail: torn_notice.is_some(),
    penned: clinical.penned(),
    no_registry: clinical.skipped_no_registry,
};
if let Some(notice) = unrestored.notice() {
    eprintln!("{notice}");
    std::io::stdout().flush()?;   // process::exit skips destructors
    std::process::exit(EXIT_INCOMPLETE);
}
```

The two `anyhow::bail!`s are **deleted**, and every remedy sentence they carried is checked off against
the notice and the stdout summary before the commit — line by line, not by eye.

### Task 3: The CLI pins invert

1. `restore_torn_medium_cli.rs` — `status.success()` → `code() == Some(3)`; header rewritten to say
   what the 2c ruling was, what #594 reversed, and what it did **not** reverse (the prefix still
   restores — that is what this file's other four assertions are for).
2. `restore_cli_applies_nothing_untrusted.rs` — the two `// Exit status deliberately NOT asserted:
   #594` comments become `assert_eq!(out.status.code(), Some(3), …)`; the header's whole
   "deliberately NOT asserted" paragraph is replaced by the ruling.
3. `restore_exit_vocabulary.rs` gets the cross-crate pin:
   `assert_eq!(cairn_node::restore::completeness::EXIT_INCOMPLETE, 3)` plus a comment naming
   `cairn_sync::requeue::EXIT_INCOMPLETE` as the alias. (`cairn-node` cannot `use` `cairn-sync` — the
   dependency runs the other way — so the alias itself is what makes drift impossible, and the
   constant's value is pinned here.)

### Task 4: ADR-0071 and the spec

ADR-0071, *A restore that left records behind exits INCOMPLETE*. It must state:

- the decision and the full seven-row exit table above;
- **why it does not conflict with ADR-0068 decision 1** (gating vs. reporting);
- **what it reverses** — 2c round 2's exit-0-on-torn pin — named as a reversal, not a tidy-up;
- the widening, and the inversion argument that drove it;
- **precedence**: FAILED outranks INCOMPLETE, and why a blocked ceremony is not an incomplete one;
- **rejected**: options 1 and 3 from #594, each with the reason;
- **residuals**: #596/#597 (the remedies a crashed or straddled restore prints are still wrong or
  misleading — a truthful exit code does not make a false sentence true).

Then `decisions/README.md` (index row), `docs/spec/index.md` (**0.73**), and the restore's operator
contract in the spec gets the exit vocabulary stated once.

**Before merge, check the ADR sentence by sentence against the code** — the #584 lesson: an ADR is
immutable, and #584's review found two false sentences, one inherited from its own design document.

### Task 5: Mutation proofs

Each mutation is applied, the named test is confirmed RED, and the mutation is reverted.

| # | Mutation | Must kill |
|---|---|---|
| M1 | `EXIT_INCOMPLETE = 1` | `restore_exit_vocabulary` + all five CLI pins |
| M2 | `notice()` returns `None` when only `torn_tail` is set | `restore_torn_medium_cli` |
| M3 | `notice()` returns `None` when only `past_chain_break` is set | untrusted test 14 |
| M4 | `notice()` returns `None` when only `unknown_plane` is set | untrusted test 17b |
| M5 | `notice()` returns `None` when only `penned` is set | `restore_cli_surface` pen |
| M6 | `notice()` returns `None` when only `no_registry` is set | `restore_cli_surface` no-registry |
| M7 | The `unrestored` block is moved ABOVE `local_state_failure` | the two recovery-code pins (they see 3, not 1) |
| M8 | `past_chain_break` reads `counts.clinical` instead of `clinical_plane.gated_out` | a clean-medium test must not exit 3 — proves the field is the gated count, not the total |
| M9 | `std::io::stdout().flush()` deleted | *expected survivor* — Rust's `Stdout` is a `LineWriter`, so `println!` has already flushed at each newline. Recorded as a survivor **with its reason**, not as a kill, and the call stays: it is the idiom's safety margin against a future `write!` without a newline. |

M9 is written down in advance precisely because #584's lesson was that a survivor must be reasoned
about, not assumed unobservable — and here the reasoning says it genuinely is unobservable.

### Task 6: Gate, review, tracking documents, PR

1. `cargo fmt --check`, `cargo clippy -- -D warnings`, the `-D warnings` doc build.
2. Full workspace gate with the DB env (background; ~2 h — do the docs pass while it runs).
3. Code review of the whole diff; fix findings, or file them (house rule 5).
4. HANDOVER + ROADMAP updated in **this** PR, not a separate one.
5. PR to `main`, linked to #594.

---

## Paper-parity benchmark (§1.2)

**Paper counterpart.** Re-establishing a practice's records from the off-site copy after the premises
burn: the box of charts comes back from the bank vault, and the practice manager checks whether
anything is missing — *"is that everything?"* — before the practice reopens on it.

**Steps.** Paper *N* = 1 human act (look at what came back and decide whether it is all of it).
Architecture-forced *M* = **1** — unchanged by this slice. UI bundling target *K* = 1.

This slice adds **no human act at all**: an exit status is read by a script, never by a person, and
every sentence a human reads was already being printed. What it changes is that the *paper* act —
"is that everything?" — becomes answerable **without a human**, which is the direction §1.2 wants.
Before it, a cron-driven drill that dropped stderr had no way to ask the question: exit 0 covered both
"everything came back" and "three nights of charts are still on the medium and always will be".

**Time + cognitive load.** Unchanged, and **not re-measured**. The #512 measurement stands as taken
(100 003 events in 116.7 s against a 600 s budget, `crates/cairn-node/results/2026-09-10-macos-m3max.md`)
— this slice adds one integer comparison and at most one `eprintln!` to a run of that length, which is
not measurable against a 600 s budget. Cognitive load falls slightly for the human case (a verdict line
replaces an `Error:`-prefixed message that told an operator their restore had "failed" when it had
not), and falls to zero for the scripted case. **`M > N` still stands for the restore as a whole and
#512 stays open** — the third act is the recovery code, and nothing here touches it.

## Deviations from the issue as written (decided while planning)

1. **The pen and no-registry arms are included** (the widening above — maintainer, 2026-09-16). #594's
   text named three causes; building only those would ship an inverted signal.
2. **`EXIT_INCOMPLETE` moves to `cairn-node` and `cairn-sync` aliases it.** #594 says "the code
   `requeue` already uses"; two copies of the literal `3` in two binaries is the drift this project
   keeps finding. The dependency direction permits the move.

## Review ledger

### Task 5 — mutation proofs (run 2026-09-16)

Every mutation applied to a clean tree, built, run, reverted, tree verified clean before the next.
Nine mutations, **eight killed, one survivor with its reason recorded in advance**.

| # | Mutation | Killed by | Note |
|---|---|---|---|
| M1 | `EXIT_INCOMPLETE = 1` | `restore_exit_vocabulary` (1 test) + `restore_torn_medium_cli` | — |
| M2 | `is_complete` ignores `torn_tail` | `restore_torn_medium_cli` | leaves `…untrusted` green — specific |
| M3 | `is_complete` ignores `past_chain_break` | `…untrusted::a_restore_applies_the_verified_prefix_and_not_one_record_past_a_chain_break` | leaves torn green |
| M4 | `is_complete` ignores `unknown_plane` | `…untrusted::a_plane_this_build_cannot_route_is_noted_with_its_count_and_never_applied` | leaves torn green |
| M5 | `is_complete` ignores `penned` | `restore_cli_surface` (pen) | leaves torn green |
| M6 | `is_complete` ignores `no_registry` | `restore_cli_surface` (2 tests) | leaves torn green |
| M7 | the verdict block moved ABOVE `local_state_failure` | `restore_cli_surface::without_the_flag_a_piped_restore_still_inherits_no_custody` | the precedence pin, and the ONLY test that sees it |
| M8 | `past_chain_break: counts.clinical` instead of `gated_out` | `restore_cli_surface::a_scripted_restore_brings_the_clinical_record_back` | a CLEAN medium would exit 3 |
| M9 | `stdout().flush()` (and its import) deleted | **SURVIVOR — expected, reason recorded before the run** | Rust's `Stdout` is a `LineWriter`, so every `println!` has already flushed at its newline. The call stays as the idiom's margin against a future `write!` without one. |

**M3 and M4 both live in the same file and each killed exactly the test that names its claim** —
checked by name, not by the failure count (the #593 lesson: read the panic line).

### Harness defects found while running the mutations

Two, both of the shape *"the mutation ran; the revert did not, and nothing said so"* — which
silently contaminates every mutation after it. Both were caught, and the second run redone.

1. **A deletion mutation cannot be reverted by swapping `""` back**: the empty string is not a
   unique anchor, so `revert` refused and left the file mutated. The first M2–M6 run was
   contaminated (M3 tested M2+M3, and so on) and was discarded; the harness was rewritten to swap
   whole blocks in both directions, and every run now refuses to start on a dirty tree and fails
   loudly if its own revert did not land.
2. **M7's block anchor started below its leading comment**, so the revert moved the code back and
   left the comment orphaned at the bottom of the arm. Re-anchored on the comment.

The lesson worth carrying: **a mutation harness needs its own positive control.** `git diff
--quiet` before apply and after revert is the whole of it, and without it the run reports
confident kills for mutations that were never cleanly applied.

### Findings while writing the tests (both corrections to this plan)

1. **A wrong recovery code is NOT the FAILED path** — `apply_local_state_export` returns
   `Ok(None)` for it. The plan assumed `local_state_failure`, and two "precedence pins" were
   aimed at the wrong tests. The genuine FAILED path is a prompt that cannot be *asked* (no flag,
   no tty): `rpassword` errors. That run also has INCOMPLETE causes, so it is the one place the
   precedence is observable end to end — M7's only killer.
2. **The `past_chain_break` guard the plan specified was dead logic.**
   `untrusted_clinical_notice` returns `Some` exactly when `gated_out > 0`, so
   `if untrusted_notice.is_some() { gated_out } else { 0 }` is just `gated_out` — and worse than
   redundant, since it implies the notice carries a condition the status must honour. Simplified,
   and ADR-0071's residual now states the honest version: the status inherits whatever `gated_out`
   gets wrong and is not a second opinion on it.
3. **`EXIT_INCOMPLETE` could not become a compile-time alias.** `cairn-sync` is a binary-only
   crate whose `cairn-node` dependency is a **dev**-dependency. Held equal by a unit test inside
   `requeue.rs` instead — the earliest point both numbers are visible. The two halves are not
   redundant: that test pins that they AGREE, `restore_exit_vocabulary` pins the VALUE.
