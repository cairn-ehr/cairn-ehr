# ADR-0069 — The restore takes its recovery code from a file, so a drill can be rehearsed

- **Status:** Accepted
- **Date:** 2026-09-11
- **Closes:** [#572](https://github.com/cairn-ehr/cairn-ehr/issues/572) — *restore's recovery-code prompt
  has no non-interactive path, so a DR drill cannot be scripted or run unattended.* Filed from the #512
  measurement run and confirmed empirically there.
- **Unblocks:** [#570](https://github.com/cairn-ehr/cairn-ehr/issues/570) — the restore CLI surface had no
  tests, and this is one reason why: a test could not drive an `rpassword` prompt without a
  pseudo-terminal either.
- **Derives from:** [ADR-0026](0026-node-durability-and-disaster-recovery.md) (the restore ceremony),
  [ADR-0066](0066-identity-dies-with-the-disk-custody-must-not.md) (the local-state export carries the
  dead node's unwrap secret, which is what this code opens),
  [ADR-0068](0068-provenance-warns-never-gates-on-the-restore-path.md) (provenance warns, never gates —
  the reason there is no other interactive read anywhere in the restore arm, which the correction below
  depends on).

---

## Context

`restore` needs two secrets and treated them unalike:

| | Passphrase for the NEW key | The OLD node's recovery code |
|---|---|---|
| Flag | `--passphrase` | none |
| Environment variable | `CAIRN_KEY_PASSPHRASE` | none |
| Invented at restore time | yes | **no** |
| Retained off-node | no | **yes — the only such artifact** |
| What it opens, with the medium beside it | the new node's own key | **the clinic's whole clinical record** |

The recovery code was read through `rpassword::prompt_password`, which opens `/dev/tty` and fails on any
non-tty. A piped code did not merely get ignored: **the read errored, the export never opened, and the
restore finished having recovered zero patients while exiting non-zero** — [#500](https://github.com/cairn-ehr/cairn-ehr/issues/500)'s
own signature, arriving inside the mechanism built to prevent it.

Three consequences, and the first is the one that matters clinically:

1. **A clinic could not rehearse.** A practice that wants quarterly assurance that its medium still
   restores had to do it by hand every time, and the practices that will not do it by hand are the ones
   that most need the answer. The paper counterpart — checking the off-site box is still readable without
   waiting for a fire — has no such obstacle.
2. **The restore CLI could not be tested**, which is why its exit status and its operator warnings had no
   coverage at all (#570).
3. **The #512 measurement rig had to allocate a pseudo-terminal**, scaffolding that existed only because
   of this gap.

---

## Decision — the code may come from a **file**, named by `--old-recovery-code-file <PATH>`

Optional. With the flag absent the ceremony is exactly what it was: the same prompt, the same three
attempts, the same messages.

### Why a path, and not a flag value or an environment variable

Because the two secrets in the table above are not alike, and consistency with `CAIRN_KEY_PASSPHRASE` is
not a strong enough reason to give both the same exposure. The passphrase is invented at the keyboard and
protects a key that has not existed for ten seconds. The recovery code is the single retained off-node
artifact, and together with the medium sitting beside it, it yields the clinic's whole clinical record in
the clear.

A path keeps the secret **off the process table** (`ps auxww` shows the path, never the code), **out of
shell history**, and **out of the environment** — so not in `/proc/<pid>/environ`, not inherited by child
processes, and not in a crash dump. It also composes for free: a tmpfs path, a named pipe and
`/dev/stdin` all work with no additional mechanism, which covers the drill, the measurement rig and
#570's tests without a `-` sentinel.

### Why the name says "old"

`restore` also **mints and prints a new** recovery code. A flag called `--recovery-code-file` on such a
command invites the reading *"this is where the new code goes"*, on the one ceremony that has no second
attempt.

That ambiguity is not hypothetical. The #512 rig matched the string `"recovery code"` to find the prompt,
and the **new** node's shown-once banner satisfied it two steps early — so the rig typed its answer into
the terminal minutes before the real prompt and worked only as type-ahead. It also leaves
`--new-recovery-code-file` free as the consistent name for the sink the follow-up issue proposes.

### Two supporting rules

- **The file is read in the step-0 pre-flight**, before a byte is minted, for the same reason the two
  checks already there live there: by the time the unseal runs, `finalize_identity` has fenced the restore
  door and there is no free second attempt. A drill script pointed at a path that does not exist now costs
  nothing; the equivalent mistake used to cost an identity and a database.
- **A blank or whitespace-only file is refused.** `normalize_recovery_code` strips all spacing before the
  unwrap, so such a file would attempt an unseal under an *effectively empty* secret and come back `None`
  — bit-for-bit the answer a wrong code gives. The operator would then be told their code was wrong, or
  their export possibly damaged, and would go hunting for a code they had saved correctly.
  `establish-local-state-key` already guards this exact input for this exact reason. A **trailing newline
  is tolerated**: `printf '%s\n'` is how anyone writes one of these.

---

## Rejected — refusing the flag alongside a sealed new key

The first draft of this decision refused `--old-recovery-code-file` together with a sealed new key,
directing a drill to `--insecure-plaintext` and a real restore to an attended terminal. The stated aim was
to **mechanically preserve** [#527](https://github.com/cairn-ehr/cairn-ehr/issues/527)/[#562](https://github.com/cairn-ehr/cairn-ehr/issues/562)'s
triage note — *"no cron-run command reaches `print_recovery_code`"* — rather than let it expire silently.

It was rejected because reading the restore arm showed the note **is already false**, and the refusal
would therefore have bought nothing:

- `restore` calls `print_recovery_code` whenever it mints a sealed key.
- A medium with **no local-state export sibling** never reaches the recovery-code prompt at all, because
  that ceremony is entered only when the export bytes were found.
- Every other read in the restore arm between its start and that print is **print-only** — not by
  accident, but because ADR-0068 ruled that provenance warns and never gates.

So a sealed restore of a federation-only medium, with `CAIRN_KEY_PASSPHRASE` set, already runs start to
finish with no terminal and prints a fresh recovery code to stderr. Cron could already reach it.

Three further problems with the refusal, recorded so it is not re-proposed:

1. **It guards one path and leaves the open one open.** The exposure arrives by a route the flag does not
   appear on.
2. **It keys on the wrong signal.** The flag is not a proxy for *unattended*: an operator standing at the
   terminal reading a code off a USB stick would be refused, while the genuinely unattended path is
   untouched.
3. **It costs the measurement rig the ceremony it exists to measure.** The rig restores into a sealed key
   deliberately, and its stated subject is a restore *"as an operator would run it"*.

---

## Correction to #527/#562's triage note

**The note was already false before this slice existed.** It is corrected here rather than broken here,
and this ADR's date must not be read as the day it stopped being true. A future reader who finds that
sentence beside this date would otherwise conclude the wrong thing.

What replaces the refusal is an honest report: when a sealed key is minted and **stderr is not a
terminal**, `restore` prints a line saying that a secret has just been written to this stream and that
whatever captured it must be treated as secret. Keyed on the stream rather than on the flags, because the
honest question is *"will a human see this code?"* — and because that question also covers the
federation-only path that has been open all along.

---

## Consequences

- **A disaster-recovery drill can be rehearsed**, by cron or by CI, for the first time. This is the point.
- **The restore CLI is testable**, which #570 then acts on: the non-zero exit on an incomplete clinical
  restore, and the reachability of three operator warnings that a source-text grep can never establish.
- **The measurement rig drops its pseudo-terminal**, and the section of it explaining why one was needed
  becomes the record of a closed gap. A pty reappearing there means this decision has regressed.
- **#512's budget wording becomes operationally true.** It says a restore completes *"within 10 minutes
  **unattended** after the operator's last keystroke"*. Before this, there was no way to reach the last
  keystroke without a human at a terminal.
- **The secret half of #512's budget is now pinnable**, and is pinned. `restore_needs_nothing_about_the_dead_node.rs`
  had declined to assert it in as many words, because doing so while the retained secret had no flag would
  have pinned the defect.
- **No migration, no `SCHEMA_GENERATION` bump, no wire-format change.** The event core is untouched.

## What is still not true

- **The freshly-minted recovery code still goes to stderr**, on this path and on the older
  federation-only one. The warning above **reports** that; it does not prevent it, and its wording is
  pinned against claiming otherwise. The real fix is a place to put the new code that is not the
  process's stderr — a `--new-recovery-code-file <PATH>` sink, or a refusal to mint a sealed key when
  nothing can show its code to a human. Filed as [#575](https://github.com/cairn-ehr/cairn-ehr/issues/575),
  because it is a second secret-handling decision rather than part of this one.
- **`M > N` still stands and #512 stays open.** This slice adds no human act; in a drill it *replaces*
  one with a file read. The three numbers are unchanged: paper *N* = 2, architecture-forced *M* = 3, UI
  bundling target *K* = 2.
- **The time budget is not re-measured**, deliberately. Reading a secret from a file rather than a
  terminal does not touch the per-event cost that 116.7 s against 600 s is about, and restating a number
  this slice did not change is how a results file drifts from what it measured.
- **[#567](https://github.com/cairn-ehr/cairn-ehr/issues/567) is untouched**: `verify-backup`'s OK is
  still federation-only, so a green verify still says nothing about the plane a restore now applies.
- **[#568](https://github.com/cairn-ehr/cairn-ehr/issues/568) is untouched**: `do_requeue`'s
  custody-carrying arm still has no test.
- **[#569](https://github.com/cairn-ehr/cairn-ehr/issues/569) is untouched**: db/052's registry door still
  discards a content conflict silently.
