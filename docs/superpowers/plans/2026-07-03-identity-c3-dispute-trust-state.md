# C3 implementation plan — `dispute` + chart trust-state projection

**Design:** `docs/superpowers/specs/2026-07-03-identity-c3-dispute-trust-state-design.md`
**Mode:** inline TDD, red-first. Each task writes its failing test(s) first, then the minimum code to
pass. Runs on the PG18 / `cairn_pgx` rig; DB-gated tests need
`CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test"`.

## Task 1 — Rust dispute builders (pure) + unit tests

**File:** `crates/cairn-event/src/identity.rs` (extend; do not touch the C1 `LinkAssertion` code).

- Red: add `#[cfg(test)]` cases — `dispute_assertion_body` carries `dispute_id`/`subject`/`reason`;
  `dispute_resolution_body` carries `dispute_id`/`subject`/`resolution`; `render_dispute_twin` starts
  `dispute opened: ` and contains subject + dispute_id; `render_dispute_resolved_twin` starts
  `dispute resolved: ` and contains the resolution.
- Green: add
  - `pub struct DisputeAssertion<'a> { dispute_id, subject, reason }`
  - `pub struct DisputeResolution<'a> { dispute_id, subject, resolution }`
  - `pub fn dispute_assertion_body(&DisputeAssertion) -> Value`
  - `pub fn dispute_resolution_body(&DisputeResolution) -> Value`
  - `pub fn render_dispute_twin(&DisputeAssertion) -> String`
  - `pub fn render_dispute_resolved_twin(&DisputeResolution) -> String`
  All pure `serde_json`, no I/O, mirroring the existing `assertion_body` idiom. Required fields only (no
  omit-when-absent needed — reason/resolution are mandatory).
- Verify: `cargo test -p cairn-event`.

## Task 2 — db/023 event-type registration + structural floor + twin hook

**File:** `db/023_identity_dispute.sql` (new); register in `crates/cairn-node/src/db.rs` `SCHEMA` list
as `("023_identity_dispute", include_str!("../../../db/023_identity_dispute.sql"))` right after
`022_node_event_quarantine`.

- `INSERT INTO event_type_class` the two types (`identity.dispute.asserted`,
  `identity.dispute.resolved`), both `('additive', FALSE)`, `ON CONFLICT DO NOTHING`.
- `cairn_check_dispute_assertion(p_type text, b jsonb)`: valid-uuid `dispute_id`, valid-uuid `subject`,
  and a required non-empty descriptive field (`reason` when `p_type` ends `.asserted`, `resolution` when
  `.resolved`). Distinct legible `RAISE EXCEPTION` per violation.
- `CREATE OR REPLACE FUNCTION cairn_event_twin` — copy db/018's current body verbatim and add a dispute
  branch that calls `cairn_check_dispute_assertion(p_type, b)` and joins the HARD-require-twin set
  (`v_identity := true` path, or a parallel `v_dispute` flag with the same hard-require behaviour). Do
  **not** drop the demographic or identity-link branches or the honest-degrade fallback.
- Red first: the Task 3 acceptance test `valid_dispute_is_accepted` + the floor-rejection tests fail
  (unknown event_type / no floor) before this file exists; green after.

## Task 3 — `chart_dispute` overlay table + AFTER-INSERT trigger

**File:** `db/023_identity_dispute.sql` (same migration, next section).

- `chart_dispute(dispute_id PK, subject, state CHECK(open|resolved), reason, hlc_wall, hlc_counter,
  origin, updated_at)`; partial index `(subject) WHERE state='open'`; `GRANT SELECT ... TO cairn_agent`.
- `chart_dispute_apply()` trigger fn: read `dispute_id`/`subject` from `NEW.body`; `state` from
  `event_type` (`.asserted`→open, `.resolved`→resolved); `reason` = `body->>'reason'` for asserted else
  `body->>'resolution'`. Upsert keyed on `dispute_id`, `ON CONFLICT DO UPDATE ... WHERE
  (EXCLUDED.hlc_wall, EXCLUDED.hlc_counter, EXCLUDED.origin) > (chart_dispute.hlc_wall, ...)` — the C1
  HLC-overlay tiebreak. AFTER INSERT trigger `WHEN (NEW.event_type IN (...))`.
- No component recompute, no advisory lock, no oversize guard (single-row standing fact).
- Tests (red-first, in `identity_dispute.rs`): `open_creates_open_dispute`,
  `newer_resolve_overlays_open`, `older_open_does_not_reopen_resolved`, `idempotent_reassert_one_row`.

## Task 4 — `chart_trust` VIEW + surface on `person_chart`

**File:** `db/023_identity_dispute.sql` (same migration, final section).

- `CREATE VIEW chart_trust AS SELECT subject AS patient_id, 'under-review' AS trust_state FROM
  chart_dispute WHERE state='open' GROUP BY subject;` `GRANT SELECT ... TO cairn_agent`.
- `CREATE OR REPLACE VIEW person_chart` — copy db/018's definition and append
  `COALESCE(ct.trust_state,'confirmed') AS trust_state` with a `LEFT JOIN chart_trust ct ON
  ct.patient_id = pc.patient_id`. Column is appended last (CREATE OR REPLACE VIEW constraint).
- Tests (red-first): `open_marks_chart_under_review` (chart_trust + person_chart.trust_state),
  `resolve_returns_to_confirmed`, `no_dispute_reads_confirmed`,
  `two_disputes_resolve_one_stays_under_review` / `resolve_all_confirmed`,
  `dispute_before_chart_still_under_review` (query chart_trust directly for a subject with no
  patient_chart row).

## Task 5 — floor-rejection tests

**File:** `crates/cairn-node/tests/identity_dispute.rs`.

- `bad_dispute_id_rejected`, `missing_subject_rejected`, `empty_reason_rejected`,
  `empty_resolution_rejected`, `missing_twin_rejected` — each asserts the specific `db_msg` substring.
  Use the `db_msg` helper + `$1::text::uuid` conventions from `identity_linkage.rs`.

## Task 6 — full suite, clippy, self-review, docs, PR

- `cargo test -p cairn-event` (pure) + `CAIRN_TEST_PG=… cargo test -p cairn-node --test identity_dispute`
  + `cargo test --workspace` + `cargo clippy --workspace --all-targets`.
- Self-review against the four governing principles + the house rules (esp. reviewer-legibility, pure
  functions, inline docs for a junior dev).
- Regenerate `docs/HANDOVER.md` (C3 done; next = `reattribute`/`identify`) and update
  `docs/ROADMAP.md` if it tracks identity slices.
- Commit per task (design, plan, then per-task TDD commits), push to
  `claude/cairn-continued-v71wtr`, open a **draft** PR.

## Ordering / dependency notes

- Task 1 (Rust) is independent and can land first (pure, no DB).
- Tasks 2→3→4 are one migration file built in sequence; the `identity_dispute.rs` test helpers
  (`setup`, `submit_dispute`, `submit_dispute_resolved`, `db_msg`, trust-state readers) are written once
  at the start of Task 3 and reused. Model the `setup()` truncation guard on `identity_linkage.rs` (guard
  `chart_dispute` behind `to_regclass` so `setup()` stays correct as the migration grows).
- Task 5 tests only need Tasks 2's floor, so they can be authored alongside Task 2 and confirmed red.
