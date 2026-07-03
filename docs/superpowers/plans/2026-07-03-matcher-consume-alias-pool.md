# Plan — matcher consumes `patient_alias_pool` (known-alias evidence)

Design: `docs/superpowers/specs/2026-07-03-matcher-consume-alias-pool-design.md`. Advisory Python only; no
`db/` floor, no SCHEMA bump, no ADR/spec change. Inline TDD (red → green per task).

## Task 1 — pure known-alias detection (`pipeline/alias.py`)

Red: `tests/test_alias.py` (pure, no DB).
- returning persona: b's name == a's alias → one entry `alias_of == id_a`.
- both charts repudiated the same value → two entries (one per side).
- no corroboration (a has an alias no other chart bears) → empty.
- normalization: alias `"John Fakename"` vs name recorded `"john  fakename"` (case/space/NFC) → matches.
- empty aliases / empty names → empty, no crash.
- a's alias also still in a's own retained names (struck names persist) but NOT in b → no entry (compare
  against the *other* chart only).

Green: `known_alias_evidence(id_a, names_a, aliases_a, id_b, names_b, aliases_b)`. `names_*` are
`frozenset[Name] | None`; `aliases_*` are `frozenset[str]`. Reuse `adapter._name_bag`. Dedup by
`(value, alias_of)`; stable-sorted output for deterministic JSON.

## Task 2 — banding forces REVIEW on a known-alias match (`pipeline/banding.py`)

Red: extend `tests/test_banding.py`.
- `has_known_alias=True` + sub-`review` score, no veto → `REVIEW` (was `None`).
- `has_known_alias=True` + score ≥ `auto`, no veto → `REVIEW` (was `AUTO_CANDIDATE`; never auto-link on a
  known-false name).
- `has_known_alias=False` → all existing behavior unchanged (regression).

Green: add keyword-only `has_known_alias=False`; early `return Band.REVIEW` when set. Add `alias_evidence=()`
param to `build_payload`, appended to the `evidence` tuple.

## Task 3 — DB read + runner wiring (`pipeline/db.py`, `pipeline/runner.py`)

- `db.load_aliases(conn, patient_id) -> frozenset[str]`: `SELECT value FROM patient_alias_pool WHERE
  patient_id=%s`.
- `runner.propose`: after loading `rec_a`/`rec_b`, load `aliases_a`/`aliases_b`, compute
  `known_alias_evidence(...)`, pass `has_known_alias=bool(evidence)` to `band`, `alias_evidence=evidence`
  to `build_payload`. Preserve the sub-threshold `conn.rollback()` early return (unaffected — a known-alias
  match now yields REVIEW, so it takes the persist path).

## Task 4 — conftest + DB-gated e2e (`tests/conftest.py`, `tests/test_alias_pipeline.py`)

- conftest: extend `_SCHEMA_FILES` to include `018_identity_linkage` … `025_identity_repudiate` (mirror
  `db.rs`); add `name_repudiation` to `_PROJECTION_TABLES` truncate list.
- Red/green DB-gated `test_alias_pipeline.py` (`@pytest.mark.usefixtures`/`pg_conn`, skips without
  `CAIRN_TEST_PG`): seed chart A (real name + a `name_repudiation` row for the false name) and chart B (the
  same false name as its registered name); directly `INSERT INTO name_repudiation` (the C5 event floor is
  tested in `identity_repudiate.rs` — this test covers *consumption* only). Assert `load_aliases(A)` returns
  the value; run `propose(conn, A, B)` → `REVIEW`; read back `match_proposal.evidence` and assert it carries
  a `known_alias` entry naming the value and `alias_of == A`.

## Task 5 — suites, review, docs, PR

- `cd matcher && uv run pytest` (pure green) and, if `CAIRN_TEST_PG` reachable, `uv run --extra pipeline
  pytest` (DB green). `uv run ruff` if configured.
- Self-review (advisory-only, no floor/SCHEMA/ADR touched; confidentiality: value only, no `reason`).
- Regenerate `docs/HANDOVER.md`; append a ROADMAP note.
- Commit per-step; push `claude/cairn-handover-d8ch8c`; open a **draft** PR.
