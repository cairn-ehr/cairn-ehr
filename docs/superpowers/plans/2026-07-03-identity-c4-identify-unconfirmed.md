# C4 implementation plan — `identify` + the *unconfirmed* trust state

**Design:** `docs/superpowers/specs/2026-07-03-identity-c4-identify-unconfirmed-design.md`
**Mode:** inline TDD, red-first. Each task writes its failing test(s) first, then the minimum code to
pass. Runs on a PG16 / `cairn_pgx` 0.2.0 rig; DB-gated tests need `CAIRN_TEST_PG` set to the test cluster.

## Task 1 — Rust identify/pending builders (pure) + unit tests

**File:** `crates/cairn-event/src/identity.rs` (extend; do not touch the C1 `LinkAssertion` or C3
`DisputeAssertion` code).

- Red: add `#[cfg(test)]` cases — `pending_assertion_body` carries `subject`/`basis` and NOT `method`;
  `identify_assertion_body` carries `subject`/`method` and NOT `basis`; `render_pending_twin` starts
  `identity pending: ` and contains subject + basis; `render_identify_twin` starts `identity confirmed: `
  and contains subject + method.
- Green: add
  - `pub struct PendingAssertion<'a> { subject, basis }`
  - `pub struct IdentifyAssertion<'a> { subject, method }`
  - `pub fn pending_assertion_body(&PendingAssertion) -> Value`
  - `pub fn identify_assertion_body(&IdentifyAssertion) -> Value`
  - `pub fn render_pending_twin(&PendingAssertion) -> String`
  - `pub fn render_identify_twin(&IdentifyAssertion) -> String`
  All pure `serde_json`, no I/O, mirroring the existing `dispute_assertion_body` idiom. Required fields
  only (no omit-when-absent — basis/method are mandatory).
- Verify: `cargo test -p cairn-event`.

## Task 2 — db/024 event-type registration + structural floor + twin hook

**File:** `db/024_identity_identify.sql` (new); register in `crates/cairn-node/src/db.rs` `SCHEMA` list
as `("024_identity_identify", include_str!("../../../db/024_identity_identify.sql"))` right after
`023_identity_dispute`.

- `INSERT INTO event_type_class` the two types (`identity.pending.asserted`,
  `identity.identify.asserted`), both `('additive', FALSE)`, `ON CONFLICT DO NOTHING`.
- `cairn_check_identity_state_assertion(p_type text, b jsonb)`: valid-uuid `subject`, and a required
  non-empty descriptive field (`basis` when `p_type` = `identity.pending.asserted`, else `method`).
  Distinct legible `RAISE EXCEPTION` per violation. No cross-existence check on subject.
- `CREATE OR REPLACE FUNCTION cairn_event_twin` — copy db/023's current body verbatim and add an
  identity-state branch that calls `cairn_check_identity_state_assertion(p_type, b)` and joins the
  HARD-require-twin set (a `v_identity_state` flag with the same hard-require behaviour). Do **not** drop
  the demographic, identity-link, dispute branches, or the honest-degrade fallback.
- Red first: the Task 3 acceptance test `valid_pending_is_accepted` + the floor-rejection tests fail
  (unknown event_type / no floor) before this file exists; green after.

## Task 3 — `chart_identity_state` overlay table + AFTER-INSERT trigger

**File:** `db/024_identity_identify.sql` (same migration, next section).

- `chart_identity_state(subject PK, state CHECK(pending|identified), detail, hlc_wall, hlc_counter,
  origin, updated_at)`; partial index `(subject) WHERE state='pending'`; `GRANT SELECT ... TO cairn_agent`.
- `chart_identity_state_apply()` trigger fn: read `subject` from `NEW.body`; `state` from `event_type`
  (`identity.pending.asserted`→pending, `identity.identify.asserted`→identified); `detail` =
  `body->>'basis'` for pending else `body->>'method'`. Upsert keyed on `subject`, `ON CONFLICT DO UPDATE
  ... WHERE (EXCLUDED.hlc_wall, EXCLUDED.hlc_counter, EXCLUDED.origin) > (chart_identity_state.hlc_wall,
  ...)` — the C1/C3 HLC-overlay tiebreak. AFTER INSERT trigger `WHEN (NEW.event_type IN (...))`.
- No component recompute, no advisory lock, no oversize guard, **no subject-consistency guard** (the key
  is the subject).
- Tests (red-first, in `identity_identify.rs`): `pending_creates_pending_row`,
  `newer_identify_overlays_pending`, `older_pending_does_not_reopen_identified`,
  `newer_pending_reopens_after_identify`, `idempotent_reassert_one_row`.

## Task 4 — rework `chart_trust` to severity-max compose

**File:** `db/024_identity_identify.sql` (same migration, final section). `db/023` left untouched.

- `CREATE OR REPLACE VIEW chart_trust` as the `WITH trust_source(patient_id, severity)` UNION ALL of
  `chart_dispute WHERE state='open'` → 2 and `chart_identity_state WHERE state='pending'` → 1, selecting
  `CASE max(severity) WHEN 2 THEN 'under-review' WHEN 1 THEN 'unconfirmed' END::text` GROUP BY patient_id.
  Column contract stays `(patient_id uuid, trust_state text)` so CREATE OR REPLACE is reload-idempotent
  and `person_chart_trust` (C3) is untouched. `GRANT SELECT ... TO cairn_agent` (idempotent re-grant).
- Tests (red-first): `pending_marks_chart_unconfirmed` (chart_trust + person_chart_trust.trust_state),
  `identify_returns_to_confirmed`, `no_identity_reads_confirmed`,
  `dispute_and_pending_reads_under_review` → `resolve_dispute_leaves_unconfirmed` →
  `identify_then_confirmed` (the compose/precedence proof), `pending_before_chart_still_unconfirmed`
  (query chart_trust directly for a subject with no patient_chart row).

## Task 5 — floor-rejection tests

**File:** `crates/cairn-node/tests/identity_identify.rs`.

- `bad_subject_rejected`, `missing_subject_rejected`, `empty_basis_rejected`, `empty_method_rejected`,
  `missing_twin_rejected` — each asserts the specific `db_msg` substring. Use the `db_msg` helper +
  `$1::text::uuid` conventions from `identity_dispute.rs`.

## Task 6 — full suite, clippy, self-review, docs, PR

- `cargo test -p cairn-event` (pure) + `CAIRN_TEST_PG=… cargo test -p cairn-node --test identity_identify`
  + `cargo test --workspace` + `cargo clippy --workspace --all-targets`. Also re-run
  `--test identity_dispute` to prove the chart_trust rework did not regress C3.
- Self-review against the four governing principles + the house rules (esp. reviewer-legibility, pure
  functions, inline docs for a junior dev).
- Regenerate `docs/HANDOVER.md` (C4 done; next = `reattribute`/`repudiate`) and update `docs/ROADMAP.md`
  if it tracks identity slices.
- Commit per task (design, plan, then per-task TDD commits), push to the designated branch, open a
  **draft** PR.

## Ordering / dependency notes

- Task 1 (Rust) is independent and can land first (pure, no DB).
- Tasks 2→3→4 are one migration file built in sequence; the `identity_identify.rs` test helpers
  (`setup`, `submit_identity_state`, `db_msg`, trust-state readers) are written once at the start of
  Task 3 and reused. Model the `setup()` truncation guard on `identity_dispute.rs` (guard
  `chart_identity_state` behind `to_regclass` so `setup()` stays correct as the migration grows). The
  compose test also reuses the dispute submit helper — factor a small `submit_dispute` local or reuse the
  pattern inline.
- Task 5 tests only need Task 2's floor, so they can be authored alongside Task 2 and confirmed red.
