# ADR-0071 — A restore that left records behind exits INCOMPLETE

- **Status:** Accepted
- **Date:** 2026-09-16
- **Spec version at acceptance:** 0.73
- **Issue:** [#594](https://github.com/cairn-ehr/cairn-ehr/issues/594)
- **Relates to:** [ADR-0026](0026-node-durability-and-disaster-recovery.md) ·
  [ADR-0066](0066-identity-dies-with-the-disk-custody-must-not.md) ·
  [ADR-0067](0067-a-restore-reads-the-clinical-plane.md) ·
  [ADR-0068](0068-provenance-warns-never-gates-on-the-restore-path.md) ·
  [ADR-0069](0069-the-restore-takes-its-recovery-code-from-a-file.md)
- **Reverses:** one ruling of #500 slice 2c round 2 — that a torn medium restores at exit **0**.
  Nothing else of 2c's is disturbed, and in particular the ruling that a torn medium must not be
  **refused** stands in full.

## Context

`cairn-node restore` is reached twice: once by a clinic that has already lost its disk, and —
since [ADR-0069](0069-the-restore-takes-its-recovery-code-from-a-file.md) gave it a non-interactive
path — routinely, by a **drill** with no human at the terminal. A drill is the whole point: a
recovery procedure nobody rehearses is a recovery procedure nobody has.

A script driving such a run has one channel it can rely on. Warnings on stderr are dropped by a great
many cron wrappers; the stdout summary needs a parser. The **exit status** is what is left, and until
this ADR it could not distinguish these two runs:

- every record the medium carried is now in the node's log; and
- three nights of charts sat past a broken chain link, were never offered to the apply door, and
  **no retry of any command will ever reach them**.

Both exited **0**. That is [#500](https://github.com/cairn-ehr/cairn-ehr/issues/500)'s own signature —
a restore that reads *"restored"* to a solo clinic which then believes it has its charts back —
reappearing inside the mechanism built to prevent it.

Two further outcomes exited **1**, and said in words that they should not have: *"this exit code says
the restore is INCOMPLETE, not that it failed"*. `cairn-sync requeue` — the command that finishes such
a restore — has had **exit 3 (INCOMPLETE)** since [#578](https://github.com/cairn-ehr/cairn-ehr/issues/578),
and its own doc comment named the gap: restore "has only exit 1 to say it with".

## Decision

**1. `cairn-node restore` exits 3 (INCOMPLETE) whenever any record the medium carried is not in this
node's log when the command finishes**, printed after the whole summary. There are five such causes
and they share one status:

| Outcome | Records left unrestored | Recoverable by | Before | After |
|---|---|---|---|---|
| Clean restore | none | — | 0 | **0** |
| A **torn tail** | yes, lost from this copy | nothing | 0 | **3** |
| Records **past a mid-file chain break** | yes, on the medium, never offered | nothing | 0 | **3** |
| A **plane this build cannot route** | yes, on the medium | a newer build | 0 | **3** |
| Records **penned** | yes, in the pen with their custody | `cairn-sync requeue` | 1 | **3** |
| **No actor registry** | the whole clinical plane | a second restore, fresh database | 1 | **3** |
| The local-state bundle **could not be applied** | key material not installed | recover the export | 1 | **1** |

**2. Exit 1 narrows to FAILED: the ceremony was BLOCKED.** A local-state bundle that opened and then
could not be installed, a prompt that could not be asked (no `--old-recovery-code-file` and no tty), a
database fault, a clinical apply that died mid-run. This is checked **first, and outranks INCOMPLETE**,
because such a run also pens every sealed record it was offered: reporting it as a completed-but-partial
recovery would send an operator to `requeue` instead of to their recovery code.

**A WRONG recovery code is not this path.** `apply_local_state_export` returns `Ok(None)` for a code
that does not open the bundle — *an honest degradation the helper already reported* — and the run
lands on **3**, the same status as the no-registry case whose end state it shares exactly: the node is
restored as a federation peer, the charts are still on the medium, and the remedy is a second restore
into a fresh database. Reporting one end state with two statuses because the causes differed is the
incoherence this ADR removes, not one it should introduce.

**3. The rule is one pure predicate over five scalars**, `restore::completeness::Unrestored`, with the
exit status as its only consumer. The status therefore cannot disagree with the notices printed above
it: each field is read from the same variable that drove this run's own warning.

**4. One exit vocabulary across both binaries.** `restore` fills the quarantine pen and `requeue`
empties it; a script driving one recovery reads the number from both, and cannot be asked to learn two
vocabularies for it. `cairn-sync`'s constant is held equal to `cairn-node`'s by a test rather than by
a compile-time alias, because `cairn-sync` is a binary-only crate whose `cairn-node` dependency is a
**dev**-dependency — aliasing in production code would pull the whole node crate into that binary's
build graph to import an integer.

## Why this does not conflict with ADR-0068

[ADR-0068](0068-provenance-warns-never-gates-on-the-restore-path.md) decision 1 — *refusing converts
a partial loss into a total one* — is the ruling that a torn tail, an `Unknown(tag)` plane and a
legacy medium must each be **restored anyway**, because an operator reaches `restore` when everything
else has already failed and re-running the backup is usually impossible.

**That ruling is about gating, and nothing here gates.** By the time the verdict is computed, every
record this build is entitled to apply has been applied, the identity has been minted, and the whole
summary has printed. A non-zero status taken after that refuses nothing and costs nothing.

*Not refusing* and *reporting success* are different claims, and the torn-medium test had been making
the second on the strength of the first. That is the narrow thing this ADR reverses.

## Alternatives rejected

**Option 1 — keep exit 0 for all three, and record why.** The argument was that the records are not
recoverable by any retry of `restore`, so a non-zero status signals nothing actionable. It fails on
its own terms for two of the three: an unroutable plane is recovered by upgrading and restoring again,
and a chain break is the case where the records *could have been spliced in*, which is the one an
operator most needs to hear about. And "not actionable" is the wrong test for a **drill**: a drill's
whole product is the answer to *did this work?*, and exit 0 was answering yes.

**Option 3 — non-zero for a chain break only** (possible tampering), 0 for torn and unknown. This
asks the exit status to carry a **diagnosis** rather than a verdict, which is what the summary and the
warnings are for. It also leaves a drill unable to tell a complete recovery from a torn one, which is
the failure mode the whole DR path exists to make visible.

**Building only #594's three named causes.** This was the literal reading of the issue, and it would
have shipped an **inverted** signal: a monitoring script would read the most recoverable outcome (a
pen, which `requeue` empties in one command) as FAILED=1, and the least recoverable one (records past
a chain break, gone for good) as INCOMPLETE=3. The maintainer widened the decision rather than file
the inversion.

**A distinct exit code per cause.** Rejected as a vocabulary no operator would learn and no wrapper
would branch on correctly. The causes are not alternatives — a medium can be torn *and* carry an
unroutable plane *and* pen what it did offer — so a per-cause code would need a precedence order that
loses information the notice already carries in full.

## Consequences

- A cron drill can now ask *"did my clinic's records come back?"* and get an answer without a human.
  `verify-backup && backup` remains the wrong order for a different reason (see HANDOVER).
- **A restore that leaves an unroutable plane behind now fails a naive `restore && …` chain**, where
  before it passed. That is the intended reversal — the records are not in the log — and the notice
  names the remedy (upgrade, restore again).
- The two apologetic messages are deleted: the status says what they were saying.
- `scripts/measure_dr_restore.py` is unaffected. It provisions a clean medium, and its exit-status
  check — *"strictly broader than the summary"* — becomes strictly more useful, since a run that left
  records behind can no longer be timed as a complete one.

## Residuals — named, not assumed away

- **[#596](https://github.com/cairn-ehr/cairn-ehr/issues/596)** and
  **[#597](https://github.com/cairn-ehr/cairn-ehr/issues/597)**: a crashed restore's "restore again"
  and the straddled-duplicate notice's "All of them were applied" are still wrong and still
  misleading. **A truthful exit code does not make a false sentence true.**
- A restore that exits **1** from the FAILED path prints its top-level cause through `main`'s
  `Termination`, which for an unaskable prompt reads `Error: Device not configured (os error 6)` —
  naming neither the missing `--old-recovery-code-file` nor the fact that a non-interactive run needs
  it. This ADR makes the *status* of that run right and leaves its *text* wrong:
  **[#611](https://github.com/cairn-ehr/cairn-ehr/issues/611)**.
- **Exit 0 is a claim about RECORDS, not about provisioning, and the two can come apart.** A medium
  whose clinical plane is empty, beside an export that degraded honestly (a wrong code, a corrupt
  container), exits **0** having installed no custody key — and that node then refuses its first
  sealed write. Every step is correct by this ADR's own rule, and the run is still not what a drill
  wrapper reading 0 believes it is. Whether INCOMPLETE should widen from *records left behind* to
  *recovery left short* is a decision, not a patch: **[#613](https://github.com/cairn-ehr/cairn-ehr/issues/613)**.
  `restore --help` states the limit rather than over-promising.
- The verdict is computed from the same variables that drove the notices, so it cannot disagree with
  them — but it **inherits whatever those variables get wrong**. `past_chain_break` reads the same
  `gated_out` that `untrusted_clinical_notice` reads, so a medium whose gated-out count were wrong
  would be wrong in the status and the warning together, and neither would catch the other. The
  status is a faithful report of what this command believes; it is not a second opinion.
