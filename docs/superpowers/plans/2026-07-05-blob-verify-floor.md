# Plan — the blob self-verification in-DB floor

Design: [specs/2026-07-05-blob-verify-floor-design.md](../specs/2026-07-05-blob-verify-floor-design.md).
TDD throughout: each task writes its failing test first.

## Task 1 — `cairn_pgx`: `cairn_blob_verify` + `cairn_blob_verify_error` (0.3.0)

- pg_tests first (red): matching pair → true / NULL error; one flipped content byte → false +
  legible error naming both hashes; truncated address → false; wrong-prefix (sha2-256
  multihash) address → false; empty address → false. Never a panic.
- Implement: thin wrappers over `cairn_event::blob_address` (one implementation rule).
  `#[pg_extern(immutable, parallel_safe)]`, same shape as `cairn_verify`/`cairn_verify_error`.
- Bump `extensions/cairn_pgx/Cargo.toml` version and `cairn_pgx.control` default_version
  to 0.3.0.

## Task 2 — `db/026_blob_verify_floor.sql` + hostile-client integration tests

- `crates/cairn-node/tests/blob_floor.rs` first (red against the current schema): the
  matrix from the design §5.
- Implement db/026: `cairn_blob_present_guard()` PL/pgSQL trigger function (RAISE with
  `cairn_blob_verify_error` as DETAIL; content-NULL raises its own legible message) + the
  INSERT and UPDATE triggers with the WHEN conditions from the design §2.2 (idempotent:
  `DROP TRIGGER IF EXISTS` + `CREATE`, `CREATE OR REPLACE` for the function — the schema
  array re-runs on every connect).
- Register in `crates/cairn-node/src/db.rs` SCHEMA (after 025) and
  `crates/cairn-sync/src/main.rs` SCHEMA (after 021).
- Rewrite db/003's lines-27–32 honest-gap comment to point at the db/026 floor.

## Task 3 — version floor + suite sweep

- `REQUIRED_PGX_FLOOR` `0.2.0 → 0.3.0` in cairn-sync (+ comment: db/026 references
  `cairn_blob_verify`; a stale .so must fail legibly at the gate, not with
  `undefined function` mid-schema-load). Check for tests pinning the constant.
- Full `cargo test --workspace` (DB-gated on the PG16 + cairn_pgx rig) + clippy + fmt.
- ROADMAP slice entry + HANDOVER note (kept small — a parallel session owns the
  demographics sections).
