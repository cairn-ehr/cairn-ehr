# ADR-0056 decision 5: the residual refusal contract — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the *residual* half of ADR-0056 true as written on the clinical plane — where the floor
genuinely refuses, the bytes are **penned verbatim by digest**, the refusal is **answered legibly**, and
the cycle **fails loudly** instead of exiting SUCCESS with a silently-frozen watermark.

**Architecture:** Slice 58 removed the *unknown-type* refusal at the door (admit-and-defer), so the
refusals that reach the puller's `Err` arm today are the point-3 residual ones: unenrolled/revoked signer,
malformed envelope, oversize, `t_effective` past the HLC ceiling, never-lawful contributor shapes. Those
are all **deliberate** `RAISE EXCEPTION`s (SQLSTATE `P0001`); a transient DB fault is anything else. The
clinical puller currently cannot tell them apart, because `apply_signed` flattens `postgres::Error` into a
`String` and drops the SQLSTATE. Restoring that one fact lets the puller route a deliberate refusal onto the
**same pen + re-offer floor the unverifiable path already uses** (one contract, not two — the ADR's own
"a second contract is a second thing to reason about at every door"), and leaves the freeze arm for genuine
infrastructure trouble — now loud.

**Tech Stack:** Rust (`cairn-sync` clinical puller, `cairn-node` node-plane puller), `postgres` /
`tokio-postgres`, PostgreSQL 18. No schema change, no `SCHEMA_GENERATION` bump, no new dependency.

**Issues:** [#267](https://github.com/cairn-ehr/cairn-ehr/issues/267),
[#269](https://github.com/cairn-ehr/cairn-ehr/issues/269),
[#270](https://github.com/cairn-ehr/cairn-ehr/issues/270).
[#268](https://github.com/cairn-ehr/cairn-ehr/issues/268) is **deliberately not implemented** — see
"Task 7" and the finding below.
**ADR:** [ADR-0056](../../spec/decisions/0056-unknown-event-types-admitted-uninterpreted.md) decision 5
(implements; no new ADR).

## Global Constraints

- **Licence:** AGPL-3.0. No new dependencies in this slice.
- **TDD:** the failing test comes first, always.
- **Junior-readable comments:** every non-trivial block explains *why* it exists and how it fits.
- **Pure functions:** both new decisions (is this refusal deliberate? is this cycle loud?) are pure
  functions over small value inputs, unit-tested with no database, then wired in at one call site each.
- **Never hard-code cryptographic material in tests** — derive keys at runtime (house rule 6, #146).
- **Full-workspace verification:** `cargo test --workspace`, never a per-crate run, and never piped
  through `tail` (that masks cargo's exit code).
- **Test env:** `CAIRN_TEST_PG` / `CAIRN_TEST_PG2` / `CAIRN_TEST_PG3` (PG18 + `cairn_pgx`).

## Paper-parity (§1.2)

Paper-parity: not clinical-surface — this slice changes only what a node does with bytes its own floor
refused (persist them and say so, rather than discard them and exit quiet). It adds no human act at any
layer and exposes no runnable clinical surface. Its paper counterpart — an illegible referral you cannot
file still stays in the tray with a note saying why, and someone is told — motivates the change rather
than being a workflow it introduces.

## The design decision inside this slice (stated so a reviewer can push back)

ADR-0056 decision 5 says "the watermark does not advance past an unresolved refusal", and §6.3 names the
mechanism in the same breath: *"(a quarantine floor pins it)"*. Two readings are available for a
deliberately-refused **verifiable** event:

1. **Freeze** the cursor at it (today's behaviour for this class), or
2. **Pen it, pin the re-offer floor at its seq, and let the cursor advance** — the *unverifiable* path's
   existing mechanism, where the floor re-ships the refused slot every cycle until it heals.

This slice takes **(2)**, for three reasons: the floor is what §6.3 names; availability-over-consistency
(principle 5) says one bad event from one bad author must not withhold thousands of applicable events from
other authors; and one mechanism for both refusal classes is cheaper to reason about than two. The freeze
arm survives for the genuinely undecidable case — a transient DB fault, where retrying the same event next
cycle is the *only* correct move — and becomes loud (#270).

The cost, accepted: a bootstrap that pulls a large history before its actor-enrollment ceremony now pens
those events (bounded by the existing per-peer quota) instead of halting at the first one. Auto-release on
successful apply — the node plane's existing `#111` behaviour, mirrored here — makes that transient: the
repair pull drains the pen. Unverifiable rows can never auto-release (their bytes never apply), so the pen's
forensic value is untouched by construction.

## Finding: why #268 is NOT implemented here

#268 asks that the node plane's `P0001` skip-and-advance be aligned with this contract. Implemented
literally, it would be a defect, and the codebase already says so: `node_quarantine.rs`'s existing test
comments "penning it would flood the pen with ordinary, self-healing refusals", and `PullStats::rejected`
is documented as "the normal node-plane case".

The reason is that the two planes' `P0001`s are not the same kind of event. `stream_node_events` serves
**every** `node_event` row, so a puller routinely receives events authored by nodes it does not peer with
(peer C's genesis, C's peering events) and refuses them with `author % is not an active peer (deny-all)`.
That is the node plane's **scoping**, not a refusal of history the node should hold. Penning it would (a)
grow a permanent pen row per untrusted-peer event, (b) hold `stats.pending > 0` forever, making the loud
integrity signal permanent for a completely normal condition (alarm fatigue — exactly what ADR-0009's
notification economy forbids), and (c) eventually hit the quota and freeze the link.

Aligning the node plane therefore needs a **refusal-class partition first** — the door distinguishing
"not-for-me trust-graph deny-all" from "genuinely refused history" (oversize, malformed payload), e.g. by
giving the latter a distinct SQLSTATE in `db/007`. That is a floor change with its own design question, so
it stays filed. This slice records the finding on #268 and leaves it `loop:blocked`.

## File Structure

| File | Responsibility | Change |
|---|---|---|
| `crates/cairn-sync/src/main.rs` | the clinical puller | **Modify** — `ApplyError` (message + SQLSTATE), the two pure predicates, the pen-on-deliberate-refusal arm, auto-release, the loud condition + message |
| `crates/cairn-sync/tests/clinical_pull.rs` | two-node integration | **Modify** — the freeze test becomes the pen test: loud exit, bytes penned, and the repair pull drains the pen |
| `crates/cairn-node/tests/node_quarantine.rs` | node-plane integration | **Modify** — add the #269 heal test |
| `docs/spec/sync.md` | §6.3 failure-mode table | **Modify** — the honest-status sentence loses #267/#270, keeps #268 |
| `docs/HANDOVER.md`, `docs/ROADMAP.md` | working scaffolding | **Modify** — Slice 60 + the 07-31/08-01 loop backlog |

---

### Task 1: The two pure decisions, unit-tested first

**Files:** `crates/cairn-sync/src/main.rs` (+ its in-crate `#[cfg(test)]` module)

- [ ] RED: unit tests for `refusal_is_deliberate(Option<&str>)` — `Some("P0001")` true; `None`
      (dropped connection), `Some("40001")` (serialization failure), `Some("57014")` (statement timeout),
      `Some("53100")` (disk full) all false.
- [ ] RED: unit tests for `cycle_is_loud(unverifiable, refused, frozen, pen_failed)` — every input that is
      not "all zero / no freeze" is loud; the all-clean case is quiet. Pins #270 as a value-level fact.
- [ ] GREEN: both functions, each with a comment naming the failure they exist to prevent.

### Task 2: Preserve the SQLSTATE through `apply_signed`

**Files:** `crates/cairn-sync/src/main.rs`

- [ ] RED: a unit test asserting `ApplyError` keeps both the door's legible message (message + DETAIL, the
      #109 behaviour that must not regress) and the SQLSTATE.
- [ ] GREEN: `apply_signed` returns `Result<bool, ApplyError>`; the two call sites (`do_pull`,
      `do_requeue`) adapt. `ApplyError: std::error::Error` so it still boxes into `R<_>`.

### Task 3 (#267): pen a deliberate door refusal on verifiable bytes

**Files:** `crates/cairn-sync/src/main.rs`, `crates/cairn-sync/tests/clinical_pull.rs`

- [ ] RED (canned-server unit test): a batch `[good, refused-by-unenrolled-author, good]` → the pull fails
      loudly, `sync_quarantine` holds the refused bytes verbatim with a legible reason, the floor pins at
      its seq, and the cursor advances past it so the two good events applied.
- [ ] RED (two-node integration): rewrite `refused_apply_freezes_the_watermark_and_recovers_without_loss`
      → `refused_apply_pens_the_bytes_and_recovers_without_loss`: first pull exits **non-zero**, pens the
      bytes, applies nothing; after the enrollment repair the next pull converges **and the pen is empty**.
- [ ] GREEN: route a deliberate refusal into the existing `refused` pen arm; add auto-release
      (`DELETE FROM sync_quarantine WHERE content_digest = $1`) on a successful apply, gated on an active
      floor so the common path does no per-event DELETE.

### Task 4 (#270): a frozen watermark fails loud

**Files:** `crates/cairn-sync/src/main.rs`

- [ ] GREEN: `frozen` joins the loud condition via `cycle_is_loud`; the error message names the freeze, its
      cause, and what the operator should do. The `run` daemon already classifies a `PullIntegrityError` as
      an integrity condition (not a partition), so this changes exit status and log class, not the cadence.

### Task 5 (#269): a node-plane skipped event heals via the full sweep

**Files:** `crates/cairn-node/tests/node_quarantine.rs`

- [ ] RED→GREEN (characterization): a verifiable `node.enrolled` from a node A does **not** yet peer with
      is refused (deny-all) and skipped; the cursor advances past it; an **incremental** pull does not
      re-offer it (this is the cost #268 exists to remove); after `peer.added` a **full sweep** admits it.

### Task 6: spec §6.3 honest status

**Files:** `docs/spec/sync.md`

- [ ] Update the residual-refusal row: the pen now holds verifiable door refusals and the clinical plane
      fails loudly; the node-plane divergence (#268) remains, stated with the reason from the finding above.

### Task 7 (#268): record the finding

- [ ] Comment on #268 with the routine-deny-all analysis and the refusal-class-partition prerequisite;
      leave it `loop:blocked`.

### Task 8: gate + docs

- [ ] `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
      `cargo test --workspace` (all three DSNs set), `scripts/run-db-sql-tests.sh`.
- [ ] HANDOVER + ROADMAP: Slice 60, plus the 2026-07-31/08-01 tech-debt-loop backlog neither file records.
