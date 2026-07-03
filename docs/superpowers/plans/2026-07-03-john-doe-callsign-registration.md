# Plan — §5.4 John Doe registration (slice A): callsign + matcher exclusion

Design: `docs/superpowers/specs/2026-07-03-john-doe-callsign-registration-design.md`.
TDD throughout (failing test first). No `db/` migration, no SCHEMA/ADR/spec bump — a compose of
already-built primitives + an advisory matcher exclusion.

## Task 1 — pure callsign generator (`cairn-event`)

**Red:** unit tests in `crates/cairn-event/src/john_doe.rs`:
- `callsign` starts with `Unknown-`.
- Parts appear in order `Unknown-<class>-<site>-<date>-<suffix>`.
- Each part is sanitized: internal whitespace and the `-` delimiter collapse to a single safe char, so a
  `site` of `"ED North"` cannot inject an extra field or break parsing.
- Deterministic: same inputs → same output.
- Distinct suffixes → distinct callsigns.
- Empty part → a stable placeholder token (`unknown`), never an empty segment / doubled delimiter.

**Green:** implement `pub fn callsign(class, site, date, suffix) -> String` + a private `sanitize_part`
(NFC-agnostic: lowercase, replace any run of non-alphanumeric with `-`, trim, empty→`unknown`). Add
`pub mod john_doe;` to `lib.rs`.

## Task 2 — `register-john-doe` compose (`cairn-node`)

**Red:** DB-gated integration tests `crates/cairn-node/tests/john_doe.rs` (mirror `auto_apply.rs` rig):
- After `register_john_doe`, `chart_trust` reports `unconfirmed` for the new UUID.
- `patient_name` has one row for the UUID with `use_key='callsign'` and `value` == the generated callsign;
  `patient_name_current` surfaces it (no legal name → callsign is the winner).
- Two successive `register_john_doe` calls create two distinct pending charts with two distinct callsigns.
- Pure builder unit tests (no DB): the two `EventBody`s are well-formed — name event is
  `demographic.field.asserted` with `facets.use="callsign"` and a non-empty twin; pending event is
  `identity.pending.asserted` with a non-empty twin; both carry the same `patient_id`.

**Green:**
- `john_doe.rs`: pure `build_callsign_name_body(patient_id, callsign, provenance, hlc, kid) -> EventBody`
  and `build_pending_body(patient_id, basis, provenance, hlc, kid) -> EventBody` (mirror
  `auto_apply::build_suggested_link_body`; sole contributor = the registering actor, role `suggested` for
  the un-attested desk path — additive events demand no attestation); an async `register_john_doe(client,
  sk, kid, node_origin, class, site, date, basis) -> anyhow::Result<(Uuid, String)>` that mints the UUID,
  derives the callsign (suffix = short hex of the UUID), ticks HLC (reuse the `next_hlc` pattern),
  signs + `submit_event`s both events in one txn, returns `(uuid, callsign)`.
- `lib.rs`: `pub mod john_doe;`.
- `main.rs`: `RegisterJohnDoe { class, site, basis }` subcommand; `site` defaults to the node name, `date`
  = today (the CLI edge supplies the clock; the library stays pure-ish/testable by taking `date`).

## Task 3 — matcher placeholder exclusion (`matcher/pipeline/db.py`, advisory)

**Red:** DB-gated tests `matcher/tests/test_john_doe_exclusion.py`:
- Seed two charts each with only a callsign-use name sharing a token → `generate_candidate_pairs` yields
  **no** pair between them (blocking excludes the callsign token).
- Seed one chart with a callsign-use name → `load_candidate(...).names` is empty/None (scoring excludes it).
- Seed one chart with BOTH a callsign name and a real legal name → the real name still blocks and scores
  (exclusion is placeholder-only). Two charts sharing the real token still pair.

**Green:**
- Add `PLACEHOLDER_NAME_USES = frozenset({"callsign"})` module constant + a small SQL fragment
  `use_key <> ALL(%s)` (or `NOT IN`) parameterized by the set.
- `load_candidate`: change the name query to `SELECT value, provenance_rank FROM patient_name
  WHERE patient_id=%s AND use_key <> ALL(%s)` passing the placeholder set.
- `_GROUPS_SQL` `name_tokens` CTE: add `WHERE use_key <> ALL(%s)` (or fold into the existing FROM). The
  `_GROUPS_SQL` is executed in `generate_candidate_pairs`; thread the placeholder-set param through. Keep
  the compound `name+year` pass consistent (it reads from `name_tokens`, so it inherits the exclusion for
  free — verify no separate name read).
- Note in a comment: the exclusion is advisory (matcher owns its feature space, §5.2/§5.13); the callsign
  stays a real, displayed name in `patient_name`.

## Task 4 — verify / review / ship

- `cd matcher && uv run pytest` (pure) + `CAIRN_TEST_PG=… uv run --extra pipeline pytest` (DB).
- `cargo test --workspace` + `cargo clippy --workspace` (needs the local PG + cairn_pgx rig; stand it up
  in-container as prior sessions did — PG16 + `--features pg16`, pgrx 0.18.1).
- Adversarial self-review (correctness of the SQL exclusion; the partition-safety of the suffix; that no
  real name is ever excluded).
- Regenerate `docs/HANDOVER.md`; commit; push `-u origin claude/handover-next-task-fpooj1`; open a draft PR.
