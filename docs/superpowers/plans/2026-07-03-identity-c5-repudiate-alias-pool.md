# Plan — C5 `repudiate` + the known-alias pool

Design: `docs/superpowers/specs/2026-07-03-identity-c5-repudiate-alias-pool-design.md`.
Inline-TDD, red-first. Mirrors C4 (`db/024` + `identity.rs` + `identity_identify.rs`).

## Task 1 — pure `cairn-event` builder (red → green)
`crates/cairn-event/src/identity.rs`:
- `pub struct RepudiationAssertion<'a> { subject, value, reason }`.
- `pub fn repudiation_assertion_body(&a) -> Value` → `{subject, value, reason}` (all mandatory; no
  omit-when-absent discipline, like dispute).
- `pub fn render_repudiate_twin(&a) -> String` → `name repudiated: {subject} — "{value}" ({reason})`.
- Unit tests: body carries exactly subject/value/reason (no stray keys); twin non-empty, contains subject,
  value, reason, starts with `name repudiated: `.

Run `cargo test -p cairn-event` (no DB needed) → green.

## Task 2 — the in-DB floor + overlay + projection (`db/025_identity_repudiate.sql`)
Additive DDL only; leave db/010–024 untouched; no SCHEMA bump.
1. `INSERT INTO event_type_class ('identity.repudiate.asserted','suppressing',FALSE)` — suppressing forces
   the db/005 human-attestation gate; `targets_other_author=FALSE` (no `target_event_id`; value-grained).
2. `cairn_check_repudiation_assertion(b jsonb)`: payload present; `subject` string + valid uuid; `value`
   string + non-empty (trimmed); `reason` string + non-empty (§4.1). Each a distinct legible RAISE.
3. `CREATE OR REPLACE FUNCTION cairn_event_twin` — add the `identity.repudiate.asserted` branch (floor +
   `v_twin_required` set together, per the C4 ladder); preserve every existing branch. `submit_event` NOT
   re-declared.
4. `name_repudiation (subject uuid, value text, reason text, hlc_wall, hlc_counter, origin, updated_at,
   PK(subject,value))` + GRANT SELECT to cairn_agent. HLC-latest-wins upsert trigger
   `name_repudiation_apply` on `event_type='identity.repudiate.asserted'` (the C4 overlay-apply shape:
   `ON CONFLICT (subject,value) DO UPDATE … WHERE EXCLUDED.(hlc_wall,hlc_counter,origin) > stored`).
5. `CREATE OR REPLACE VIEW patient_name_current` — copy db/012's exact column list + ORDER BY, add
   `WHERE NOT EXISTS (SELECT 1 FROM name_repudiation r WHERE r.subject = patient_name.patient_id
   AND r.value = patient_name.value)`. Column contract UNCHANGED (reload-idempotent).
6. `CREATE OR REPLACE VIEW patient_alias_pool AS SELECT subject AS patient_id, value, reason, hlc_wall,
   hlc_counter, origin, updated_at FROM name_repudiation;` + GRANT SELECT to cairn_agent.

Wire into `crates/cairn-node/src/db.rs` SCHEMA_FILES after `024_identity_identify` with a header comment.

## Task 3 — DB-gated integration tests (`crates/cairn-node/tests/identity_repudiate.rs`)
Harness mirrors `attestation.rs` (needs a human attester + token) fused with `identity_identify.rs`
(name assertions via `demographic.field.asserted`). Helpers: enroll agent + human; submit a name
(`demographic.field.asserted`, field=name, authored twin); mint token via `sign_attestation`; submit a
repudiation via `submit_event($1,$2,$3)`.
Tests (per the design test plan): accept + winner-changes · alias-pool entry · retained-set preserved ·
only-name → no winner · idempotent/HLC-latest reason · un-attested refused · four floor rejections
(empty value, empty reason, bad subject, missing twin).

## Task 4 — verify + docs
- `cargo test --workspace` green (incl. C1/C3/C4 regression); `cargo clippy --workspace` clean.
- Regenerate HANDOVER.md + append a ROADMAP slice line.
- Commit per task; push; open draft PR.
