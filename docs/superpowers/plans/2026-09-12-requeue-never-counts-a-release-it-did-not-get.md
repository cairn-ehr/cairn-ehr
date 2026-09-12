# `requeue` never counts a release it did not get — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:test-driven-development. Each
> behaviour change below is watched failing first. For the two tests that already exist and assert
> the OLD behaviour, the discipline is inversion — the assertion flips and the file records why, the
> same way `nothing_yet_restores_a_clinical_event_from_a_medium` was inverted rather than deleted.

**Goal:** Close [#578](https://github.com/cairn-ehr/cairn-ehr/issues/578),
[#579](https://github.com/cairn-ehr/cairn-ehr/issues/579),
[#580](https://github.com/cairn-ehr/cairn-ehr/issues/580) and
[#581](https://github.com/cairn-ehr/cairn-ehr/issues/581) — four findings from the review of PR #577,
all in one code path. `do_requeue` deletes a pen row whenever the apply door returns `Ok`, without
asking whether the custody it was carrying actually landed. On a restored solo node the pen row is
the last copy of that DEK, so the command an operator runs *because the pen told them to* destroys
the key it was holding, prints nothing a monitor can read, and exits 0.

**Architecture:** One shared predicate in the database (`cairn_custody_landed`, added to the existing
`db/052_restore_doors.sql`), one new pure module in `cairn-sync`, and the outcome loop in
`do_requeue` rewritten around a single rule. No new migration file and **no `SCHEMA_GENERATION`
bump** — see *Why db/052 and not db/053* below.

**Tech Stack:** Rust, PL/pgSQL, `postgres`/`tokio-postgres`, PostgreSQL 18 + `cairn_pgx`. No new
dependency.

**Spec:** None. Every rule below is already decided — ADR-0067 (a restore reads the clinical plane),
ADR-0005 (a shred destroys the key, never the event), ADR-0060 decision 2 (an interrupted run still
owes its report), and `db/021`'s `acked` (a human explicitly licenses an exclusion). This slice makes
the code agree with them; it decides nothing new.

Paper-parity: not clinical-surface — `cairn-sync requeue` is an operator recovery command, not a
clinician workflow, and this slice changes no step on any path a clinician walks. It *removes* an
operator act (the second requeue that used to be needed after the first had already destroyed the
key) rather than adding one, and it adds no confirmation prompt, which §1.2 forbids as a safety
mechanism.

## Global Constraints

- **AGPL-3.0.** No dependency added.
- **TDD, without exception.** Every assertion here is watched failing for the *stated* reason first.
  A test that passes on the first run proves nothing until a deliberate mutation has been shown to
  break it, and #568's own header records two mutations that survived a first draft because the
  break did not match the shape of the claim.
- **Inline documentation for a junior developer.** Each pure function carries *why it exists and how
  it fits*. The uniform rule below is stated once, at the rule's site, and referenced elsewhere.
- **Files stay under 500 lines where feasible.** `crates/cairn-sync/src/main.rs` is 13 484 lines with
  two open decomposition issues (**#531**, **#329**), so every new function lands in a **new module**
  rather than growing it. `db/052_restore_doors.sql` is 355 lines and gains ~60.
  `crates/cairn-sync/tests/requeue_releases_custody.rs` is 821 lines, so the new DB-gated tests go in
  a second file.
- **Never hard-code cryptographic material, and never give a non-cryptographic value a cryptographic
  name** (house rule 6). Keys come from `generate_key` at runtime; `salt`/`nonce`/`iv` stay reserved
  for real constructions.
- **DB-gated tests** read `CAIRN_TEST_PG` and take `cairn_node::db::test_serial_guard` (advisory
  locks are per-database, not cluster-wide — #476). A DB-free `cargo test` needs
  `CAIRN_ALLOW_DB_SKIP=1` since #450.
- **The gate is the FULL workspace**, never piped through `tail` (that masks cargo's exit code).
- **Commit messages** use the `fix(#578):` form — the parenthesis breaks GitHub's closing-keyword
  adjacency (#546) — and end with
  `Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>`.

---

## The one rule this slice adds

> **A pen row that carries a wrapped DEK is released only when custody actually landed.**

Everything below follows from it. It is deliberately uniform across all three ways custody can fail
to land, because the three are **not distinguishable at the moment of the decision**:

| what happened | today | under the rule |
|---|---|---|
| `node_unwrap_key` absent, so `db/020` step 9 silently skips custody (#578) | row deleted, `released`, exit 0 | row **retained**, annotated, counted |
| this node's custody key could not be resolved at all (#580) | row deleted, `released`, exit 0 | row **retained**, annotated, counted |
| the wrapped DEK did not open with the key we have | row deleted, `released`, exit 0 | row **retained**, annotated, counted |

**Why the third row is not carved out, which is the reviewable choice here.** It is tempting to say
a DEK that will not open is "foreign" and its row is worth nothing. That claim cannot be made from
where the code stands: *"did not open with the key we have right now"* is not *"not ours"*. The
operator may be holding the right `<key>.unwrap` on a USB stick they have not plugged in yet, which
is precisely the #495 shape this whole DR path exists to survive. Deleting the row bets an
unrecoverable key on an inference the code cannot support — principle 4 inverted, a precise untruth
where an imprecise near-truth was free.

**Why retention is not a leak, and how a row leaves the pen.** A genuinely foreign or permanently
unopenable row would otherwise sit in the pen forever. `db/021` already has the escape and names it:
`acked = TRUE` is *"a human explicitly licenses the exclusion"*. Task 4 makes `do_requeue` honour it,
so the operator's recorded decision — not a guess by the code — is what finally drops the row.

**Exit code: 0.** A run that retained rows reports loudly on stderr and in `--metrics`, and exits 0,
matching `do_pull`'s standing ruling that a sanctioned degradation *"must not fail the cycle but must
be alertable"* — the same rule #579 cites. Nothing is lost when a row is retained, and the remedy is
a later run, not a failed one. ⚠️ **This is the one place a reviewer should push back if they
disagree**: the counter-argument is that a cron wrapper drops stderr and ignores JSON, leaving the
exit code as the only channel. It is recorded here rather than settled silently.

---

## Why db/052 and not db/053

`custody_landed` exists today in `crates/cairn-node/src/restore/clinical.rs`. `cairn-sync` needs the
same question answered and **cannot call it**: `cairn-node` is the higher layer (it would invert the
dependency and drag in `cairn-medium`, `cairn-medication-view` and `cairn-patient-search`), and the
two crates use different Postgres clients (`tokio-postgres` async vs `postgres` sync), so no Rust
function is shareable even if the edge were acceptable.

The predicate also carries a rule that must not fork: **a logged shred counts as landed.** `db/020`
step 9 refuses custody outright for an already-shredded target, which is ADR-0005's anti-resurrection
rule. A second implementation that forgot the shred clause would retain a row whose key was destroyed
on purpose, forever — and the bug would look exactly like correct behaviour.

So it goes in the database, which is this project's stated integration boundary (ADR-0001, and
`db/052`'s own header, which moved the pen door there for the same reason). It goes into **db/052
rather than a new db/053** because:

- `connect_and_load_schema` replays **every** `db/*.sql` on every connect, so a `CREATE OR REPLACE
  FUNCTION` added to an existing file reaches every node on its next start. A new file buys nothing
  here.
- `SCHEMA_GENERATION` is pinned equal to the newest migration **prefix**
  (`crates/cairn-event/tests/schema_generation.rs`). A new file forces 52 → 53, and that constant
  lives in `cairn-event`, which everything depends on — a whole-tree rebuild and relink for a
  read-only `EXISTS`.
- db/052 *is* the restore-doors file. `cairn_custody_landed` answers a restore-door question and both
  crates already load it (`cairn-node/src/db.rs:333`, `cairn-sync/src/main.rs:216`).

Constraint carried from #207: **no view-widening across files**, and a widened `CREATE TABLE` needs a
paired `ALTER`. Neither applies — this adds a function, not a relation.

---

## Tasks

### Task 1 — `cairn_custody_landed` in db/052, and one caller stops inlining it

**Test first** (`crates/cairn-node/tests/` — extend the restore suite, or a small new DB-gated file):
the door returns TRUE for an event with an `event_dek` row, TRUE for one with an
`erasure_shred_log` row and no DEK, FALSE for a sealed event with neither, and FALSE for a content
address that is not in `event_log` at all. Watched failing before the function exists.

**Then:**

1. `db/052_restore_doors.sql` gains `cairn_custody_landed(p_content_address BYTEA) RETURNS BOOLEAN`,
   `SECURITY DEFINER`, `REVOKE ... FROM PUBLIC`, `GRANT EXECUTE ... TO cairn_node` — the same shape
   and the same privilege posture as `cairn_quarantine_event` beside it. The body is the predicate
   lifted verbatim from `restore/clinical.rs::custody_landed`, and its comment carries that
   function's shred-log paragraph, which is the part a reader will otherwise delete as redundant.
2. `crates/cairn-node/src/restore/clinical.rs::custody_landed` calls the door instead of inlining the
   SQL. Its own doc-comment shrinks to a pointer at db/052 and keeps the local-fault error wording —
   the behaviour and every existing test stay green, and that is the assertion that this step is a
   refactor.

### Task 2 — the pure module: `crates/cairn-sync/src/requeue.rs`

No database. Every function here is pure and unit-tested in the same file, which is what keeps the
fast suite meaningful for a slice whose real behaviour is DB-gated.

1. **`enum WrappedDekFault { Damaged, DidNotOpen }` + `fn classify_wrapped_dek(wrapped: &[u8]) ->
   Option<WrappedDekFault>`** — #581 part 2. Today the unwrap-failure branch catches `Err(_)` and
   then asserts one specific cause the operator will act on. The distinction is **structural**, not a
   string match: `cairn_event::seal::WRAPPED_DEK_LEN` is public, and a `dek_wrapped` of any other
   length is a truncated or corrupt pen row (a `db/052` write defect or disk damage) that never
   reaches decryption. A correctly-sized blob that will not open is the other finding. Two different
   remedies, so two different messages.
   *Anti-vacuity:* a unit test feeds `unwrap_dek` a truncated blob and asserts the classifier and the
   real error agree, so the coupling to `cairn-event` is exercised rather than assumed.
2. **`struct RequeueCounts`** with `released`, `released_with_custody`, `custody_retained`,
   `skipped_acked`, `still_quarantined`, `vanished`, and **`fn metrics(&self, examined, references_unlearnable) ->
   serde_json::Value`** — #579. Two new fields, and the `examined` identity comment is restated for
   five outcomes rather than three. `released_with_custody` is a plain integer, not null-never-zero:
   under the rule of this slice the code always looks, so 0 means *"none of the released rows carried
   custody"*, which is true rather than misleading. (Contrast `references_unlearnable`, which stays
   null when the #465 report never ran.)
3. **The operator messages**, one builder per outcome, so the text is testable and the loop stays
   readable: retained-for-custody (naming which of the three causes applied and the remedy),
   skipped-because-acked (naming how to put the row back in play), and the two DEK-fault lines.

### Task 3 — `do_requeue`: ask, then delete (#578, #580)

**Test first** — DB-gated, new file `crates/cairn-sync/tests/requeue_retains_unlanded_custody.rs`,
built on `requeue_releases_custody.rs`'s fixtures (the dead-node harness, `pen`, `run_requeue`,
`twin_after_release`). The headline test is the #578 chain end to end:

1. Author one born-sealed record on a healthy node; keep its `signed_bytes`, `dek_wrapped` and twin.
2. Wipe the clinical tier **and `node_unwrap_key`** — the restored-node state where the custody key
   was never re-established.
3. Pen the record with its `dek_wrapped`, exactly as `restore::clinical` pens a
   `CustodyDidNotLand`.
4. Run the real `cairn-sync requeue --key`. Assert: the **pen row survives**, its `dek_wrapped` is
   byte-identical, the event is in `event_log`, `custody_retained` is 1 and `released` is 0.
5. Then register the unwrap key and run requeue **again**. Assert the row is gone and
   **`event_clear.twin` reads back the original text** — the body opens. This second half is what
   makes the first half a recovery rather than a stall, and it is the assertion #568 established as
   the only one worth making.

Mutations to run and kill, named up front: delete the custody check (step 4 must fail); make the
check ignore the shred clause (a shredded target must still release); key the check on `dek.is_some()`
instead of `penned_dek.is_some()` (the #580 case — unresolvable key — must still retain).

**Then** rewrite the loop's `Ok(_)` arm: if the row carried `dek_wrapped` and
`cairn_custody_landed` is false, `UPDATE` the row's `last_requeue_at` / `last_requeue_error` with the
annotation, count `custody_retained`, print, and **do not delete**. Otherwise release as today,
counting `released_with_custody` when the row carried a DEK that landed.

**Then** `cmd_requeue`'s warning stops promising what the next line used to undo (#580): it becomes a
description of what this run will do, because the pen now does hold both halves until the operator
fixes the key.

### Task 4 — acked rows (#581 part 1)

**Decide, then fix the code rather than the comment.** `restore/clinical.rs` justifies its counting
by saying *"`do_requeue` skips those"*, which is false. `do_pull` **does** skip them
(`skipped_acked`, `crates/cairn-sync/src/main.rs:4064`), and `db/021` is explicit that `acked` is
*"a recorded human decision, never an automatic one"*. A requeue that re-applies an acked row
silently overrides a decision a human recorded on purpose. So `do_requeue` skips acked rows, matching
the pull path and making the restore's comment true.

It must not skip *silently*, which would be the same defect wearing the other coat: each skipped row
gets a stderr line naming the way back in (clear `acked` on that row), and the run reports
`skipped_acked` in `--metrics`, exactly as `do_pull` does.

**Test first:** an acked pen row is untouched by requeue and counted; the same row un-acked releases
normally.

### Task 5 — invert the two tests that pin the old behaviour

`requeue_releases_custody.rs`'s `without_resolvable_custody_the_record_still_releases_but_stays_sealed`
and `a_penned_dek_from_another_node_releases_the_record_and_says_custody_was_lost` both assert that
the row is deleted. Under the rule they retain. **Invert them in place, with a header note saying
what they used to assert and which issue changed it** — never delete them: they are the record of
what this path used to do, and a reader who finds only the new assertion cannot tell that the old one
was considered.

### Task 6 — docs, gate, PR

HANDOVER and ROADMAP record the slice (house rules 7/8, and the fixes ride the work PR per the
project's convention, not a separate docs PR). Full-workspace `cargo test` via
`scripts/run-db-gated-tests.sh`. Draft PR opened before the session ends, per house rule 8.

---

## What this slice does NOT do

- **It does not change the exit code.** Reasoning above; flagged for review rather than settled
  quietly.
- **It does not touch `do_pull`.** The pull path deletes its pen row on release too, but a puller
  sees the DEK again next cycle and the peer still holds it — the asymmetry ADR-0067 already records.
  Whether the pull path owes the same guard is **#536**, which is open and stays open.
- **It does not add a `--dry-run` or a `--require-custody` flag.** #580 mentions both as
  possibilities; neither is needed once the pen stops emptying itself, and a flag that must be
  remembered is a worse safety mechanism than a default that cannot lose the key.
