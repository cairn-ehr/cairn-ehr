# DR slice 2c — the backup captures both planes: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended)
> or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax
> for tracking.

**Goal:** `cairn-node backup` writes a CAIRNB3 medium carrying **both planes** — today's `node_event`
federation set *and* every `event_log` row with its wrapped DEK — so a solo clinic's backup finally holds
the clinical record.

**Architecture:** One in-DB source of truth (`db/051`) gives the shred predicate a single home; a
plane-generic capture loop reads pages from it, builds one signed CAIRNB3 segment per page and appends
with `sync_all()`; the `CAIRNL1` export gains the actor registry and read-after-write verification;
`BackupHealth` v2 reports per-plane scope so `verify-backup` can refuse a kit that cannot restore.

**Tech Stack:** Rust (`cairn-node`, `cairn-medium`, `cairn-sync`, `cairn-event`), PostgreSQL 18 +
`cairn_pgx`, `tokio-postgres`, `ciborium`.

**Spec:** [`docs/superpowers/specs/2026-09-04-dr-slice-2c-both-planes-captured-design.md`](../specs/2026-09-04-dr-slice-2c-both-planes-captured-design.md)
— read it first, **including errata E1–E3 in §4**, which correct the draft body above them.

## Global Constraints

- **Licence:** AGPL-3.0-only. Every dependency AGPL-3.0-compatible. **This slice adds no new
  third-party dependency** — every crate it touches already has what it needs.
- **TDD, without exception.** Failing test first, then the minimum code that passes it. This is the §9
  safety-critical surface: a defect here silently loses a clinical record.
- **House rule 6 (crypto sink names).** Never name a non-cryptographic value `salt`, `nonce` or `iv`,
  and never write key material as a literal. Wrapped DEKs in fixtures are derived at runtime
  (`std::array::from_fn`). `crates/cairn-node/tests/crypto_sink_names_are_genuine.rs` enforces this.
- **DB-gated tests self-skip without `CAIRN_TEST_PG`**, and a bare `cargo test` FAILS unless
  `CAIRN_ALLOW_DB_SKIP=1` is exported (#450). Take `db::test_serial_guard(&base)` **before**
  `connect_and_load_schema`. Bind UUIDs as text and cast in SQL (`$1::text::uuid`).
- **Never pipe cargo to `tail`** — it masks the exit status. Use `--no-fail-fast` and `--all-targets`.
- **The gate is `scripts/run-db-gated-tests.sh`.** This slice touches three crates and adds a
  migration, so `-p cairn-node` alone is not a gate: it never builds `cairn-sync`'s cross-crate
  `clinical_pull.rs`, and it does not run the `db/tests/*.sql` mirrors.
- **A new `pub fn` in `crates/cairn-node/tests/common/mod.rs` must ALSO be added to
  `identity_scaffolding_shared.rs`'s hand-written expected-helper array**, or its derivation test fails.
- **Commit convention `feat(#500):` / `test(#500):`** — parenthesised, so it never closes an issue.
  The PR body is where `Closes #522` and `Closes #524` go, deliberately. **Do not write the sentence
  "this does not fix #500" in the PR body**: the closing-keyword guard will refuse it, and before that
  guard existed the same sentence closed #500 for three days.

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** the practice's nightly backup ritual — mount the volume, run the backup, take
  the medium off-site. The clinical half of that ritual is what this slice restores: today's medium
  carries the federation plane only, so the paper counterpart has no architectural equivalent at all.
- **Steps:** paper *N* = **1** (mount the medium, run one backup) → architecture-forced *M* = **1**
  (`cairn-node backup`, unchanged flags, now both planes) → UI bundling target *K* = **1**.
  **`M > N` is an architecture defect — file it, never accept it** (house rule 7). The two ways this
  slice could break it are named so they can be tested rather than hoped for: adding a separate
  clinical-capture command, or making the *clinical* capture depend on a passphrase an unattended cron
  run cannot supply. **Task 12 Step 3 tests the second one directly**
  (`backup_still_exits_zero_when_the_export_is_skipped`), and Task 7 tests that a capture with no
  signing key at all still captures.
- **Time + cognitive load:** cognitive load is unchanged by construction — the operator types the same
  command and reads one more line of scope. Time budget, **measured in Task 13** (this slice is the
  first to expose a runnable capture, so the measurement is owed here): a nightly capture over an
  **unchanged** log appends nothing and completes in **< 2 s** against a warm database; a first capture
  of 10 000 fresh clinical events completes in **< 60 s**. If a measurement falls outside its budget,
  **that is the finding — file an issue, never adjust the budget.**

## File Structure

| file | responsibility |
|---|---|
| `db/051_clinical_capture_source.sql` | **new** — `event_custody_surviving` view + `cairn_clinical_page()`; the shred predicate's only home |
| `db/tests/051_clinical_capture_source_test.sql` | **new** — the in-DB mirror |
| `crates/cairn-event/src/schema_generation.rs` | 50 → 51 |
| `crates/cairn-node/src/db.rs`, `crates/cairn-sync/src/main.rs` | both loader lists gain db/051 |
| `crates/cairn-medium/src/chain.rs` | **new fn** `chain_tail` (#522) |
| `crates/cairn-medium/src/attest.rs` | `segment_commitment` binds `dek_wrapped` (#524) |
| `crates/cairn-medium/src/wire_pins.rs` | golden pins re-frozen for the new commitment |
| `crates/cairn-node/src/capture.rs` | **new** — the page read + pure row→record mapping + the plane-generic loop |
| `crates/cairn-node/src/backup.rs` | `backup_to` captures both planes onto CAIRNB3; `BackupHealth` v2 |
| `crates/cairn-node/src/localstate.rs` / `localstate_read.rs` | actor-registry slot; export query onto the view |
| `crates/cairn-node/src/main.rs` | `restore` + `verify-backup` onto `parse_any`; staleness exit code |

---

## Task 1: `db/051` — one home for the shred predicate

**Files:**
- Create: `db/051_clinical_capture_source.sql`
- Create: `db/tests/051_clinical_capture_source_test.sql`
- Modify: `crates/cairn-event/src/schema_generation.rs:45`
- Modify: `crates/cairn-node/src/db.rs` (SCHEMA list tail), `crates/cairn-sync/src/main.rs` (its list tail)

**Interfaces:**
- Produces: SQL view `event_custody_surviving(event_id UUID, dek_wrapped BYTEA)`; SQL function
  `cairn_clinical_page(after_seq BIGINT, page_limit BIGINT) RETURNS TABLE (seq BIGINT, signed_bytes
  BYTEA, attestation BYTEA, attester_key BYTEA, dek_wrapped BYTEA)`. Every later task consumes these.

- [ ] **Step 1: Write the failing in-DB mirror**

Create `db/tests/051_clinical_capture_source_test.sql`:

```sql
\set ON_ERROR_STOP on
\ir _scratch_database_guard.sql
-- Issue #500 slice 2c — the in-DB mirror for the clinical capture's source of truth.
--
-- WHY THIS MIRROR EXISTS: `cairn_clinical_page` and `event_custody_surviving` are the ONE
-- definition of "a shredded body's key must not travel". Three callers select from them
-- (the serve door, the local-state export, the backup capture), so a change here is a
-- change to all three at once — which is the point, and the reason it deserves a test at
-- the SQL layer rather than only through whichever Rust caller happens to cover it.

BEGIN;

-- A sealed event with surviving custody, and a second one whose body has been shredded.
INSERT INTO event_log (event_id, patient_id, event_type, schema_version, hlc_wall,
                       hlc_counter, node_origin, signed_bytes, content_address)
VALUES ('11111111-1111-7111-8111-111111111111', gen_random_uuid(), 'clinical.medication.asserted',
        '1', 1, 1, 'n1', '\xaa'::bytea, '\x01'::bytea),
       ('22222222-2222-7222-8222-222222222222', gen_random_uuid(), 'clinical.medication.asserted',
        '1', 2, 1, 'n1', '\xbb'::bytea, '\x02'::bytea);

INSERT INTO event_dek (event_id, dek_wrapped) VALUES
    ('11111111-1111-7111-8111-111111111111', '\xdeadbeef'::bytea),
    ('22222222-2222-7222-8222-222222222222', '\xfeedface'::bytea);

INSERT INTO erasure_shred_log (target_event_id) VALUES ('22222222-2222-7222-8222-222222222222');

-- 1. The view hides the shredded row and only that row.
DO $$
DECLARE n INT;
BEGIN
    SELECT count(*) INTO n FROM event_custody_surviving
     WHERE event_id = '22222222-2222-7222-8222-222222222222';
    ASSERT n = 0, 'a shredded body''s custody must not survive in the view';
    SELECT count(*) INTO n FROM event_custody_surviving
     WHERE event_id = '11111111-1111-7111-8111-111111111111';
    ASSERT n = 1, 'an unshredded body''s custody must survive — otherwise the test above is vacuous';
END $$;

-- 2. The page function carries the event but NULLs the shredded DEK. Both halves matter:
--    dropping the EVENT would fork the event set (the #342 trap); carrying the DEK would
--    defeat the shred.
DO $$
DECLARE r RECORD;
BEGIN
    SELECT * INTO r FROM cairn_clinical_page(0, NULL)
     WHERE seq = (SELECT seq FROM event_log WHERE event_id = '22222222-2222-7222-8222-222222222222');
    ASSERT r.signed_bytes = '\xbb'::bytea, 'the shredded event itself must still travel';
    ASSERT r.dek_wrapped IS NULL, 'the shredded event''s DEK must NOT travel';
END $$;

-- 3. page_limit NULL means "no limit" (the unpaginated serve path is the SAME statement).
DO $$
DECLARE n INT;
BEGIN
    SELECT count(*) INTO n FROM cairn_clinical_page(0, NULL);
    ASSERT n >= 2, 'a NULL page_limit must not limit';
    SELECT count(*) INTO n FROM cairn_clinical_page(0, 1);
    ASSERT n = 1, 'a page_limit of 1 must return exactly one row';
END $$;

-- 4. after_seq is STRICTLY greater — the puller's cursor semantics (#196).
DO $$
DECLARE n INT; first_seq BIGINT;
BEGIN
    SELECT min(seq) INTO first_seq FROM event_log;
    SELECT count(*) INTO n FROM cairn_clinical_page(first_seq, NULL) WHERE seq = first_seq;
    ASSERT n = 0, 'after_seq must be exclusive';
END $$;

ROLLBACK;
```

- [ ] **Step 2: Run it and watch it fail**

```bash
scripts/run-db-sql-tests.sh 051_clinical_capture_source_test.sql
```
Expected: FAIL — `relation "event_custody_surviving" does not exist`.

- [ ] **Step 3: Write the migration**

Create `db/051_clinical_capture_source.sql`. The header must carry the WHY (house rule 3) and the
privilege argument, because that is the part a reviewer cannot re-derive:

```sql
-- Cairn — the clinical capture's source of truth (#500 slice 2c).
--
-- WHY THIS FILE EXISTS. "A shredded body's key must not travel" had TWO hand-written
-- spellings in two crates — `NOT EXISTS` in the local-state export, `LEFT JOIN … CASE WHEN`
-- in cairn-sync's serve door — and slice 2c's capture would have been a third. That is the
-- mirror-list defect class this repo keeps paying for (#182, #404, #441), with a SAFETY
-- predicate as the mirrored thing. ADR-0001 (fat Postgres, thin daemon) says where it goes:
-- one definition, in the floor, inherited by every caller including one talking raw SQL.
--
-- ⚠️ PRIVILEGE. db/037 REVOKEs event_dek from PUBLIC *and* from cairn_agent, granting SELECT
-- only to cairn_node. A plain Postgres view reads its base tables as the VIEW'S OWNER, so an
-- unguarded view here would hand every role that can select it exactly the custody access
-- db/037 refused. `security_invoker = true` makes the view read as the CALLER, so db/037's
-- grants keep binding through it. Do not remove that option to "simplify"; the guard test
-- `custody_view_does_not_widen_access` exists because this is a decoy path around a floor
-- that looks correct at its own site (the #430/#431 shape).

CREATE OR REPLACE VIEW event_custody_surviving
    WITH (security_invoker = true) AS
    SELECT d.event_id, d.dek_wrapped
      FROM event_dek d
     WHERE NOT EXISTS (
         SELECT 1 FROM erasure_shred_log s WHERE s.target_event_id = d.event_id
     );

REVOKE ALL ON event_custody_surviving FROM PUBLIC;
GRANT SELECT ON event_custody_surviving TO cairn_node;

-- One page of the clinical plane, in the shape a peer response and a medium segment both
-- need. `page_limit` is BIGINT and NULL means "no limit" — Postgres reads LIMIT NULL as
-- unlimited, so the unpaginated serve path stays the SAME statement with a NULL parameter
-- rather than becoming a second query that could drift from this one.
--
-- The +1 PROBE stays at the CALL SITE, deliberately. `rows.len() == limit` cannot tell "the
-- log ends exactly here" from "there is one more we cut off", and `complete` is the puller's
-- only termination signal (slice 2b). The caller owns that claim, so the caller asks for
-- limit + 1; teaching this function to do it would put the answer in a place that cannot see
-- who is asking.
CREATE OR REPLACE FUNCTION cairn_clinical_page(after_seq BIGINT, page_limit BIGINT)
RETURNS TABLE (seq BIGINT, signed_bytes BYTEA, attestation BYTEA,
               attester_key BYTEA, dek_wrapped BYTEA)
LANGUAGE sql STABLE AS $$
    SELECT e.seq, e.signed_bytes, e.attestation, e.attester_key, c.dek_wrapped
      FROM event_log e
      LEFT JOIN event_custody_surviving c ON c.event_id = e.event_id
     WHERE e.seq > after_seq
     ORDER BY e.seq
     LIMIT page_limit;
$$;

REVOKE EXECUTE ON FUNCTION cairn_clinical_page(BIGINT, BIGINT) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION cairn_clinical_page(BIGINT, BIGINT) TO cairn_node;
```

- [ ] **Step 4: Register it in all three places, or the loader silently lags**

`crates/cairn-event/src/schema_generation.rs:45` → `pub const SCHEMA_GENERATION: i32 = 51;`
(its companion guard reads `db/` at test time and fails if this is not the newest prefix).

Append to `SCHEMA` in **`crates/cairn-node/src/db.rs`** *and* to the list in
**`crates/cairn-sync/src/main.rs`** — both, with a comment saying why:

```rust
    // db/051 (#500 slice 2c): event_custody_surviving + cairn_clinical_page — the ONE
    // definition of the shred predicate. In BOTH lists: cairn-sync's serve door SELECTs
    // from the function, so a node whose loader lags would serve from a function that
    // does not exist.
    (
        "051_clinical_capture_source",
        include_str!("../../../db/051_clinical_capture_source.sql"),
    ),
```

- [ ] **Step 5: Run the mirror and the generation guards**

```bash
scripts/run-db-sql-tests.sh 051_clinical_capture_source_test.sql
cargo test -p cairn-event --test schema_generation
cargo test -p cairn-node --lib db::tests::full_schema_list_carries_the_repo_generation
```
Expected: all PASS.

- [ ] **Step 6: Commit**

```bash
git add db/051_clinical_capture_source.sql db/tests/051_clinical_capture_source_test.sql \
        crates/cairn-event/src/schema_generation.rs crates/cairn-node/src/db.rs \
        crates/cairn-sync/src/main.rs
git commit -m "feat(#500): db/051 — one home for the shred predicate"
```

---

## Task 2: the privilege guard — the view must not widen custody access

**Files:**
- Create: `crates/cairn-node/tests/custody_view_privileges.rs`

**Interfaces:**
- Consumes: `event_custody_surviving`, `cairn_clinical_page` (Task 1).

- [ ] **Step 1: Write the failing test**

```rust
//! db/051's view must not become a way around db/037's custody REVOKE.
//!
//! db/037 revokes `event_dek` from `cairn_agent` — an advisory actor may hold clinical
//! content but never the keys. A Postgres view reads its base tables as the VIEW'S OWNER
//! unless `security_invoker` is set, so a view over `event_dek` is a textbook decoy path
//! around a floor that looks correct at its own site (#430/#431). This asserts the floor
//! still binds THROUGH the new objects.

mod common;
use cairn_node::db;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

#[tokio::test]
async fn custody_view_does_not_widen_access() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    // Anti-vacuity: prove the view is readable by the privileged role FIRST, so a failure
    // below is a refusal rather than a missing object.
    c.query("SELECT count(*) FROM event_custody_surviving", &[])
        .await
        .expect("cairn_node may read the view");

    c.execute("SET LOCAL ROLE cairn_agent", &[]).await.unwrap();
    let denied = c
        .query("SELECT count(*) FROM event_custody_surviving", &[])
        .await;
    assert!(
        denied.is_err(),
        "cairn_agent must NOT reach custody through the view — db/037 revoked event_dek \
         from it, and a view without security_invoker would hand it back"
    );

    let denied_fn = c
        .query("SELECT count(*) FROM cairn_clinical_page(0, NULL)", &[])
        .await;
    assert!(
        denied_fn.is_err(),
        "cairn_agent must NOT reach custody through the page function either"
    );
}
```

- [ ] **Step 2: Run it**

```bash
CAIRN_TEST_PG="$CAIRN_TEST_PG" cargo test -p cairn-node --test custody_view_privileges -- --nocapture
```
Expected: PASS if Task 1's `security_invoker` and grants are right. **If it fails, the migration is
wrong — fix db/051, never the test.**

- [ ] **Step 3: Commit**

```bash
git add crates/cairn-node/tests/custody_view_privileges.rs
git commit -m "test(#500): db/051 must not widen who can read custody"
```

---

## Task 3: the two existing callers move onto the one definition

**Files:**
- Modify: `crates/cairn-node/src/localstate_read.rs:69-80` (the export query)
- Modify: `crates/cairn-sync/src/main.rs:5591-5604` (the serve door query)
- Create: `crates/cairn-node/tests/shred_predicate_has_one_home.rs`

**Interfaces:**
- Produces: nothing new; both call sites now consume Task 1's objects.

- [ ] **Step 1: Write the failing source guard**

```rust
//! Every place that decides whether a shredded body's key may TRAVEL, named.
//!
//! Not style: this is the wire-level half of the crypto-shred guarantee, and before db/051
//! it had two spellings in two crates. A third would be silent — every caller keeps
//! working, and only the one that drifts stops filtering.
//!
//! NAME, NEVER COUNT (the house rule a count cannot satisfy: it cannot separate "one site
//! moved" from "one site added and one deleted"). The allow-list below is the inventory of
//! every legitimate mention, each with the reason it is not a second definition of the
//! travel predicate. When this fails: if you added a CALLER, select from
//! `event_custody_surviving` / `cairn_clinical_page` instead. If you added a genuinely new
//! decision site, add it here WITH its reason, in the same commit.

#[path = "common/sources.rs"]
mod sources;

use std::path::Path;

/// (repo-relative file, why this mention is not a rival definition).
const ALLOWED: &[(&str, &str)] = &[
    (
        "db/051_clinical_capture_source.sql",
        "THE definition: event_custody_surviving is the one filter every caller inherits",
    ),
    // The implementer fills the rest in from Step 2's output — expect db/020's apply-door
    // refusal and db/037's shred executor. Each entry states why it DECIDES something
    // else (whether to create a row / whether to delete one), not whether a key travels.
];

#[test]
fn the_travel_filter_has_one_definition_and_every_other_mention_is_named() {
    let root = sources::repo_root();
    let roots = vec![root.join("db"), root.join("crates")];
    let mut found: Vec<String> = Vec::new();
    for path in sources::source_files(&roots, &["target"], &["sql", "rs"]) {
        let text = sources::read_source(&path);
        let mentions = text.lines().any(|line| {
            let l = line.trim();
            !l.starts_with("--") && !l.starts_with("//") && l.contains("erasure_shred_log")
        });
        if mentions {
            let rel = path.strip_prefix(&root).unwrap_or(Path::new("")).display();
            found.push(rel.to_string());
        }
    }
    found.sort();
    found.dedup();
    let allowed: Vec<String> = ALLOWED.iter().map(|(f, _)| (*f).to_string()).collect();
    assert_eq!(
        found, allowed,
        "the inventory of files deciding anything about erasure_shred_log has changed.\n\
         Found: {found:#?}\nAllowed: {allowed:#?}"
    );
}
```

**Step 1a: run it once to LEARN the real inventory before pinning it.** The list above is deliberately
incomplete: `db/020`'s apply-door refusal and `db/037`'s shred executor legitimately mention the table,
and possibly others. Run the test, read the `Found:` list, and add each entry **with the reason it is
not a rival definition of the travel predicate**. A guard whose allow-list was copied from a guess
protects nothing — this repo has shipped one of those already (#511's conversion count, which named a
guard that counted none of the thing it claimed).

- [ ] **Step 2: Run it to see it fail with TWO extra sites**

```bash
cargo test -p cairn-node --test shred_predicate_has_one_home
```
Expected: FAIL listing `localstate_read.rs` and `cairn-sync/src/main.rs`.

- [ ] **Step 3: Move the export query onto the view**

In `crates/cairn-node/src/localstate_read.rs`, replace the `db.query(...)` statement:

```rust
    let rows = db
        .query(
            "SELECT c.event_id::text AS event_id, c.dek_wrapped \
             FROM event_custody_surviving c \
             ORDER BY c.event_id",
            &[],
        )
        .await
        .context("reading event_dek custody for the local-state export")?;
```

And rewrite the header comment above it — it currently argues for a `NOT EXISTS` clause that no longer
lives here. It must now say: the filter moved to db/051, this caller inherits it, and the reason the
last-line defence still matters is unchanged. **Do not leave the old comment; that is the #530
pattern, three times over in this repo already.**

- [ ] **Step 4: Move the serve door onto the function**

In `crates/cairn-sync/src/main.rs`, replace the five-column SELECT with:

```rust
            let mut rows = client.query(
                "SELECT seq,
                        encode(signed_bytes,'hex'),
                        encode(attestation,'hex'),
                        encode(attester_key,'hex'),
                        encode(dek_wrapped,'hex')
                   FROM cairn_clinical_page($1, $2)",
                &[&after_seq, &probe],
            )?;
```

Keep the whole comment block above it — the probe argument, the `complete` reasoning and the shred
explanation are all still true — but correct the sentence that describes the `LEFT JOIN … CASE WHEN`
as living *here*: it now names db/051 as the definition and this as a caller.

- [ ] **Step 5: Run the guard and the cross-crate suite**

```bash
cargo test -p cairn-node --test shred_predicate_has_one_home
CAIRN_TEST_PG=... cargo test -p cairn-sync --test clinical_pull -- --test-threads=2
```
Expected: both PASS. `clinical_pull` is the suite that proves the serve door still serves; `-p
cairn-node` never builds it.

- [ ] **Step 6: Commit**

```bash
git add crates/cairn-node/src/localstate_read.rs crates/cairn-sync/src/main.rs \
        crates/cairn-node/tests/shred_predicate_has_one_home.rs
git commit -m "refactor(#500): the export and the serve door select from db/051"
```

---

## Task 4: `chain_tail` in `cairn-medium` (closes #522)

**Files:**
- Modify: `crates/cairn-medium/src/chain.rs`
- Modify: `crates/cairn-medium/src/lib.rs` (re-export)

**Interfaces:**
- Produces: `pub fn chain_tail(m: &MediumV3, report: &ChainReport) -> ChainTail` where
  `pub struct ChainTail { pub next_index: u32, pub prev_commitment: String }`. Tasks 7 and 8 consume it.

- [ ] **Step 1: Write the failing test** (append to `chain.rs`'s test module)

```rust
    #[test]
    fn chain_tail_of_an_empty_medium_starts_the_chain() {
        let m = MediumV3 { segments: vec![], truncated_tail: false };
        let report = chain_report(&m);
        let tail = chain_tail(&m, &report);
        assert_eq!(tail.next_index, 0, "the first segment is index 0");
        assert_eq!(
            tail.prev_commitment, "",
            "index 0 has no predecessor — an empty string, never a placeholder"
        );
    }

    #[test]
    fn chain_tail_follows_the_last_VERIFIED_segment_not_the_file_tail() {
        // Two good segments, then one whose attestation does not verify. Appending after
        // the bad one would chain onto a commitment no reader trusts, so the tail is the
        // last VERIFIED position — the same rule `watermark` already follows.
        let m = medium_with_two_signed_then_one_broken();
        let report = chain_report(&m);
        let tail = chain_tail(&m, &report);
        assert_eq!(tail.next_index, 2, "the broken segment must not advance the tail");
        assert_eq!(tail.prev_commitment, commitment_of_segment_1(&m));
    }
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo test -p cairn-medium chain_tail
```
Expected: FAIL — `cannot find function chain_tail`.

- [ ] **Step 3: Implement it**

```rust
/// Where the next appended segment goes: its `index` and its `prev_commitment`.
///
/// WHY THIS EXISTS (#522). The CAIRNB3 chain is ONE global chain in file order across both
/// planes, so every writer must derive the same next index and predecessor. Before this,
/// `cairn-node` (node plane) and the capture path (clinical plane) each had to compute it,
/// and two writers deriving one invariant independently is how they come to disagree — a
/// disagreement that shows up as a broken chain on a medium nobody can re-cut.
///
/// It follows the last VERIFIED segment, never the file tail, for the same reason
/// [`watermark`] does: an unverifiable trailing segment (a torn append) must not become the
/// predecessor of a good one, or one interrupted backup poisons every capture after it.
pub fn chain_tail(m: &MediumV3, report: &ChainReport) -> ChainTail { … }
```

- [ ] **Step 4: Run the tests**

```bash
cargo test -p cairn-medium
```
Expected: PASS, 96+ tests.

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-medium/src/chain.rs crates/cairn-medium/src/lib.rs
git commit -m "feat(#522): chain_tail — one derivation of the next index and predecessor"
```

---

## Task 5: the segment attestation binds custody (closes #524)

**Files:**
- Modify: `crates/cairn-medium/src/attest.rs:82-93` (`segment_commitment`)
- Modify: `crates/cairn-medium/src/wire_pins.rs` (re-freeze)

**Interfaces:**
- Produces: an unchanged signature, changed semantics — `segment_commitment` now commits to each
  record's `dek_wrapped`, including its absence.

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn deleting_a_wrapped_dek_breaks_the_commitment() {
        // #524: the attestation bound (event_address, source_seq) and NOT the DEK, so
        // stripping custody out of a verified segment left the medium reporting fully
        // intact and the restored body silently unopenable.
        let with_custody = vec![record_with_dek(1, Some(dek_bytes(7)))];
        let stripped = vec![record_with_dek(1, None)];
        assert_ne!(
            segment_commitment(&with_custody),
            segment_commitment(&stripped),
            "removing a DEK must change the commitment, or the deletion is undetectable"
        );
    }

    #[test]
    fn a_different_wrapped_dek_breaks_the_commitment() {
        let a = vec![record_with_dek(1, Some(dek_bytes(7)))];
        let b = vec![record_with_dek(1, Some(dek_bytes(8)))];
        assert_ne!(segment_commitment(&a), segment_commitment(&b));
    }
```

`dek_bytes(seed)` derives at runtime (`std::array::from_fn(|i| seed ^ i as u8)`) — house rule 6(a); and
it is called `dek_bytes`, not `salt`/`nonce`/`iv` — house rule 6(b).

- [ ] **Step 2: Run to verify it fails**

```bash
cargo test -p cairn-medium deleting_a_wrapped_dek
```
Expected: FAIL — the two commitments are equal today.

- [ ] **Step 3: Implement**

```rust
pub fn segment_commitment(records: &[MediumRecord]) -> String {
    let per_record: Vec<Vec<u8>> = records
        .iter()
        .map(|r| {
            let mut item = event_address(&r.signed_bytes);
            item.extend_from_slice(&r.source_seq.to_be_bytes());
            // #524 — custody is committed to, ABSENCE INCLUDED. A one-byte tag separates
            // "no DEK travelled" from "a DEK travelled", so stripping custody to None is
            // as detectable as swapping it. Without the tag, an empty DEK and no DEK would
            // hash identically and the strip would be invisible again, one level down.
            match &r.dek_wrapped {
                None => item.push(0u8),
                Some(dek) => {
                    item.push(1u8);
                    item.extend_from_slice(&event_address(dek));
                }
            }
            item
        })
        .collect();
    let refs: Vec<&[u8]> = per_record.iter().map(Vec::as_slice).collect();
    crate::marker::commitment_over(&refs)
}
```

- [ ] **Step 4: Re-freeze the golden pins, with the reason recorded**

`wire_pins.rs`'s segment-commitment pins change. Update them, and add above them:

```rust
// ⚠️ RE-FROZEN 2026-09-05 (#524, slice 2c). These bytes changed ON PURPOSE: the segment
// commitment gained `dek_wrapped` (absence included). It cost no compatibility because at
// that moment NOTHING outside this crate's tests wrote CAIRNB3 — `backup_to` still wrote
// CAIRNB2 — so no medium in the field carried the old commitment. After slice 2c that is
// no longer true: from here on a change to these bytes is a FORMAT BREAK and needs a new
// container revision, not a re-freeze.
```

- [ ] **Step 5: Run the full crate suite**

```bash
cargo test -p cairn-medium --all-targets
```
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/cairn-medium/src/attest.rs crates/cairn-medium/src/wire_pins.rs
git commit -m "feat(#524): the segment attestation binds custody, absence included"
```

---

## Task 6: `capture.rs` — the page read and the pure mapping

**Files:**
- Create: `crates/cairn-node/src/capture.rs`
- Modify: `crates/cairn-node/src/lib.rs` (`pub mod capture;`)
- Create: `crates/cairn-node/tests/clinical_capture_read.rs`

**Interfaces:**
- Produces:
  - `pub struct ClinicalRow { pub seq: i64, pub signed_bytes: Vec<u8>, pub attestation: Option<Vec<u8>>, pub attester_key: Option<Vec<u8>>, pub dek_wrapped: Option<Vec<u8>> }`
  - `pub async fn read_clinical_page(db: &Client, after_seq: i64, page_limit: i64) -> anyhow::Result<Vec<ClinicalRow>>`
  - `pub async fn read_node_page(db: &Client, after_seq: i64, page_limit: i64) -> anyhow::Result<Vec<ClinicalRow>>`
  - `pub fn to_medium_record(row: &ClinicalRow) -> MediumRecord` (pure)

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn a_clinical_page_carries_the_event_its_token_and_its_custody() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = provisioned_clinic(&c).await;
    let (_id, signed) = author_sealed_clinical_event(&c, &sk, &kid).await;

    let page = capture::read_clinical_page(&c, 0, 500).await.unwrap();

    let row = page.iter().find(|r| r.signed_bytes == signed)
        .expect("the sealed clinical event must be on the page");
    assert!(row.dek_wrapped.is_some(), "a born-sealed body must carry its wrapped DEK");
    assert!(row.seq > 0);

    let record = capture::to_medium_record(row);
    assert_eq!(record.signed_bytes, signed, "signed bytes travel VERBATIM, never re-serialized");
    assert_eq!(record.source_seq, row.seq);
    assert_eq!(record.dek_wrapped, row.dek_wrapped, "custody is copied wrapped, never re-wrapped");
}

#[tokio::test]
async fn a_page_respects_its_limit_and_its_exclusive_cursor() { … }
```

- [ ] **Step 2: Run to verify it fails**

```bash
CAIRN_TEST_PG=... cargo test -p cairn-node --test clinical_capture_read
```
Expected: FAIL — `unresolved import cairn_node::capture`.

- [ ] **Step 3: Implement `capture.rs`**

The module doc must state the two things a junior reader cannot infer: that the DEK is copied **already
wrapped to this node's key** (so no plaintext key passes through), and that the shred filter lives in
db/051, not here.

```rust
/// One clinical row as db/051 hands it over. A thin, honest mirror of the function's
/// RETURNS TABLE — deliberately not `MediumRecord`, so the DB shape and the medium shape can
/// evolve independently and the mapping between them stays one reviewable function.
pub struct ClinicalRow { … }

/// Map a row to the record the medium carries. PURE — the whole DB-to-format decision in one
/// place a reviewer can hold in their head.
///
/// `dek_wrapped` is copied VERBATIM. It is already wrapped to this node's unwrap public key,
/// which is the key a restored node inherits (ADR-0066), so there is nothing to translate and
/// no plaintext key material passes through this path. `None` means exactly what
/// `MediumRecord::dek_wrapped` documents: unsealed, no custody here, or shredded — and db/051
/// is what makes the third case true.
pub fn to_medium_record(row: &ClinicalRow) -> MediumRecord { … }
```

`read_node_page` reads `node_event` with the same row shape (`SELECT seq, signed_bytes, NULL, NULL,
NULL FROM node_event WHERE seq > $1 ORDER BY seq LIMIT $2`) so Task 7's loop is genuinely
plane-generic. Its doc says why the three NULLs are honest: the federation plane has no attestation
token and no custody, and never will.

- [ ] **Step 4: Run the tests**

```bash
CAIRN_TEST_PG=... cargo test -p cairn-node --test clinical_capture_read
```
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-node/src/capture.rs crates/cairn-node/src/lib.rs \
        crates/cairn-node/tests/clinical_capture_read.rs
git commit -m "feat(#500): read one clinical page, and map it to a medium record"
```

---

## Task 7: the plane-generic capture loop

**Files:**
- Modify: `crates/cairn-node/src/capture.rs`
- Create: `crates/cairn-node/tests/capture_loop.rs`

**Interfaces:**
- Produces:
  - `pub struct PlaneCapture { pub plane: Plane, pub records_appended: usize, pub watermark: Option<i64> }`
  - `pub async fn capture_plane(db, medium: &mut Vec<u8>, plane: Plane, signer: Option<(&SigningKey, &str)>, self_id_hex: &str, page_events: i64) -> anyhow::Result<PlaneCapture>`

- [ ] **Step 1: Write the failing tests**

```rust
/// The property CAIRNB3 exists for: a nightly backup of a quiet clinic must not grow the
/// medium by a single byte. If this fails, the append-only design has bought nothing and a
/// year of nightly backups is a year of re-recorded history.
#[tokio::test]
async fn a_capture_over_an_unchanged_log_appends_nothing() {
    let (c, sk, kid) = clinic().await;
    let mut medium = serialize_v3(&[]).unwrap();
    author_sealed_clinical_event(&c, &sk, &kid).await;

    capture::capture_plane(&c, &mut medium, Plane::Clinical, Some((&sk, &kid)), &id, 500)
        .await
        .unwrap();
    let after_first = medium.clone();

    let second = capture::capture_plane(&c, &mut medium, Plane::Clinical, Some((&sk, &kid)), &id, 500)
        .await
        .unwrap();

    assert_eq!(second.records_appended, 0, "nothing new to capture");
    assert_eq!(medium, after_first, "an unchanged log must append no segment at all");
}

/// Resumption is by WATERMARK, so a second capture carries only what is new — and carries
/// it exactly once. A duplicate is not a correctness bug (set-union), which is precisely
/// why nothing else would catch it.
#[tokio::test]
async fn a_capture_resumes_from_the_watermark_and_never_re_appends() {
    let (c, sk, kid) = clinic().await;
    let mut medium = serialize_v3(&[]).unwrap();
    let (_id_a, bytes_a) = author_sealed_clinical_event(&c, &sk, &kid).await;
    capture::capture_plane(&c, &mut medium, Plane::Clinical, Some((&sk, &kid)), &id, 500)
        .await
        .unwrap();

    let (_id_b, bytes_b) = author_sealed_clinical_event(&c, &sk, &kid).await;
    let second = capture::capture_plane(&c, &mut medium, Plane::Clinical, Some((&sk, &kid)), &id, 500)
        .await
        .unwrap();

    assert_eq!(second.records_appended, 1, "only the new event");
    let all = clinical_records(&parse_any(&medium).unwrap());
    assert_eq!(all.iter().filter(|r| r.signed_bytes == bytes_a).count(), 1,
        "the first event must appear EXACTLY once — set-union would hide a duplicate");
    assert_eq!(all.iter().filter(|r| r.signed_bytes == bytes_b).count(), 1);
}

/// A torn append costs exactly ONE increment, because the watermark follows the last
/// VERIFIED segment rather than the file tail. This is the property that makes an
/// interrupted backup safe to simply re-run.
#[tokio::test]
async fn a_torn_append_costs_exactly_one_increment() {
    let (c, sk, kid) = clinic().await;
    let mut medium = serialize_v3(&[]).unwrap();
    let (_id, bytes) = author_sealed_clinical_event(&c, &sk, &kid).await;
    capture::capture_plane(&c, &mut medium, Plane::Clinical, Some((&sk, &kid)), &id, 500)
        .await
        .unwrap();

    // Simulate the interrupted write: cut the last segment in half.
    medium.truncate(medium.len() - 20);

    let again = capture::capture_plane(&c, &mut medium, Plane::Clinical, Some((&sk, &kid)), &id, 500)
        .await
        .unwrap();

    assert_eq!(again.records_appended, 1,
        "the torn segment did not advance the watermark, so its record is re-captured");
    let all = clinical_records(&parse_any(&medium).unwrap());
    assert!(all.iter().any(|r| r.signed_bytes == bytes),
        "and the record is readable again — a torn append loses at most one increment");
}

/// PAPER-PARITY IN TEST FORM (§1.2, M must stay 1). An unattended cron run has no
/// passphrase, so it has no signing key. The segment travels UNSIGNED and flagged; it must
/// never be refused, or the clinical capture would force a second human act.
#[tokio::test]
async fn a_capture_without_a_signing_key_still_captures() {
    let (c, sk, kid) = clinic().await;
    let mut medium = serialize_v3(&[]).unwrap();
    let (_id, bytes) = author_sealed_clinical_event(&c, &sk, &kid).await;

    let done = capture::capture_plane(&c, &mut medium, Plane::Clinical, None, &id, 500)
        .await
        .expect("an unavailable signing key must never BLOCK a capture");

    assert_eq!(done.records_appended, 1);
    let image = parse_any(&medium).unwrap();
    let seg = image_segments(&image).last().unwrap();
    assert!(seg.attestation.is_none(), "unsigned, and honestly so");
    assert!(clinical_records(&image).iter().any(|r| r.signed_bytes == bytes),
        "the clinical record travels either way — the signature is provenance, not custody");
}

/// Paging is not cosmetic here: a capture larger than one page must produce a chain whose
/// indices are contiguous and whose predecessors link, or the medium is unverifiable.
#[tokio::test]
async fn a_multi_page_capture_produces_one_unbroken_chain() {
    let (c, sk, kid) = clinic().await;
    let mut medium = serialize_v3(&[]).unwrap();
    for _ in 0..5 {
        author_sealed_clinical_event(&c, &sk, &kid).await;
    }

    capture::capture_plane(&c, &mut medium, Plane::Clinical, Some((&sk, &kid)), &id, 2)
        .await
        .unwrap();

    let image = parse_any(&medium).unwrap();
    let report = chain_report(as_v3(&image));
    assert!(report.chain_intact(), "every appended segment must chain to its predecessor");
    assert!(image_segments(&image).len() >= 3, "5 events at 2 per page is at least 3 segments");
}
```

- [ ] **Step 2: Run to verify they fail**

```bash
CAIRN_TEST_PG=... cargo test -p cairn-node --test capture_loop
```

- [ ] **Step 3: Implement the loop**

Order is load-bearing and must be commented as such:

1. `parse_any` the existing image → `MediumV3` (a fresh medium starts as `serialize_v3(&[])`).
2. `chain_report` → `watermark(m, &report, plane)` and `chain_tail(m, &report)`.
3. Loop: `read_*_page(db, after, page_events + 1)`; if empty, **stop without appending**.
4. Build the segment; `serialize_and_verify_v3`-equivalent check before it can touch the file.
5. `append_segment`, then `write` + **`sync_all()`** before the next page — durability is this
   caller's half of the contract (`append_segment`'s doc says so).
6. Return the plane's new watermark.

- [ ] **Step 4: Run the tests**

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(#500): the plane-generic capture loop — watermark, page, append, sync"
```

---

## Task 8: `restore` and `verify-backup` read CAIRNB3 (Erratum E2) — BEFORE the writer switches

**Files:**
- Modify: `crates/cairn-node/src/main.rs:2447` (`verify-backup`), `:2528` (`restore`)
- Create: `crates/cairn-node/tests/reads_both_medium_revisions.rs`

**Interfaces:**
- Produces: `pub fn node_plane_events(image: &MediumImage) -> Result<Vec<Vec<u8>>, BackupError>` in
  `backup.rs` — the one place that answers *"which events does the restore path apply?"* for either
  revision.

**This task lands BEFORE Task 9 on purpose.** Teaching the readers first means no commit in this
branch's history leaves the tree writing a medium it cannot read.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn a_legacy_medium_still_reads_exactly_as_before() {
    let bytes = cairn_medium::serialize_container(Some(&marker), &events).unwrap();
    assert_eq!(backup::node_plane_events(&parse_any(&bytes).unwrap()).unwrap(), events);
}

#[test]
fn a_v3_medium_yields_its_node_plane_and_ignores_the_clinical_one() {
    // The compatibility obligation: restore must behave EXACTLY as today on the node
    // plane. Restoring the clinical plane is 2d's, and this test is what stops someone
    // reading 2c as having done it.
    let bytes = v3_with_node_and_clinical_segments();
    let got = backup::node_plane_events(&parse_any(&bytes).unwrap()).unwrap();
    assert_eq!(got, node_events, "the node plane comes back in file order");
    assert!(!got.contains(&clinical_event), "2c does not restore clinical events — 2d does");
}
```

- [ ] **Step 2: Run to verify it fails**, then implement `node_plane_events` and switch both call
  sites from `parse_container`/`verify_medium_bytes` to `parse_any` + the new helper.

- [ ] **Step 3: Check the messages, not just the code.** `verify-backup`'s and `restore`'s operator
  text describe "the medium" in ways that assumed one revision. Any sentence that is now false gets
  rewritten in this commit (the #530 pattern is the most repeated finding in this programme).

- [ ] **Step 4: Run**

```bash
CAIRN_TEST_PG=... cargo test -p cairn-node --test reads_both_medium_revisions \
    --test backup_restore_roundtrip --no-fail-fast
```

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(#500): restore and verify-backup read either medium revision"
```

---

## Task 9: `backup_to` captures both planes onto CAIRNB3

**Files:**
- Modify: `crates/cairn-node/src/backup.rs:224-280`
- Modify: `crates/cairn-node/tests/dr_clinical_guarantee_gap.rs` (the inversion — Task 13 finishes it)

- [ ] **Step 1: Write the failing test** in `crates/cairn-node/tests/backup_carries_both_planes.rs`:

```rust
#[tokio::test]
async fn the_medium_carries_the_clinical_event_and_its_custody() {
    // The inversion of #500's pin, on the medium file `backup_to` actually writes — not on
    // a fixture the test builds, which would leave the production site unpinned.
    let (sk, kid) = provisioned_clinic(&c).await;
    let (_id, signed) = author_sealed_clinical_event(&c, &sk, &kid).await;

    backup::backup_to(&c, &medium_path, &health_path, 1_700_000_000, Some((&sk, &kid)))
        .await.unwrap();

    let image = parse_any(&std::fs::read(&medium_path).unwrap()).unwrap();
    let clinical = clinical_records(&image);
    let found = clinical.iter().find(|r| r.signed_bytes == signed)
        .expect("#500: the clinical event must be ON THE MEDIUM");
    assert!(found.dek_wrapped.is_some(), "and its custody must travel with it");
}
```

- [ ] **Step 2: Run to verify it fails** (today's medium is CAIRNB2 and federation-only).

- [ ] **Step 3: Implement** — `backup_to` builds/loads the CAIRNB3 image, calls `capture_plane` for
  `Plane::Node` then `Plane::Clinical`, and writes health last.

**Delete the `⚠️ #500` block on `read_event_set`** and replace it with what is true now. Leaving it is
the defect this programme has hit in every slice.

- [ ] **Step 4: Run the DR suites**

```bash
CAIRN_TEST_PG=... cargo test -p cairn-node --test backup_carries_both_planes \
    --test dr_clinical_guarantee_gap --test reads_both_medium_revisions --no-fail-fast
```

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(#500): the backup captures both planes"
```

---

## Task 10: `BackupHealth` v2 — per-plane scope

**Files:** `crates/cairn-node/src/backup.rs:36-46`, `crates/cairn-node/tests/backup_health_v2.rs`

**Interfaces:**
- Produces: `BackupHealth { version: 2, last_backup_unix, medium_path, medium_bytes, node_events: u64,
  clinical_events: u64, clinical_watermark: Option<i64>, export_covers_seq: Option<i64> }`

- [ ] **Step 1: Write the failing tests**

```rust
/// A v1 sidecar written by yesterday's binary must still READ. An operator upgrading must
/// not lose their health record — and "health may only ever under-claim" means the missing
/// fields become None/0, never a parse failure that reads as "no backup ever ran".
#[test]
fn a_v1_sidecar_still_reads_with_the_new_fields_absent() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("backup-status.json");
    std::fs::write(
        &path,
        r#"{"version":1,"last_backup_unix":1700000000,"medium_path":"/m/backup.cairn",
            "event_count":42,"medium_bytes":4096}"#,
    )
    .unwrap();

    let health = read_health(&path).expect("a v1 sidecar must still parse");
    assert_eq!(health.last_backup_unix, 1_700_000_000);
    assert_eq!(health.clinical_events, 0, "absent means zero known, never a parse failure");
    assert_eq!(
        health.export_covers_seq, None,
        "a v1 sidecar knows nothing about export coverage, and None is that honest answer — \
         0 would claim the export covers seq 0, which is a claim it never made"
    );
}

#[test]
fn a_v2_sidecar_round_trips_every_field() {
    let health = BackupHealth {
        version: 2,
        last_backup_unix: 1_700_000_000,
        medium_path: "/m/backup.cairn".into(),
        medium_bytes: 8192,
        node_events: 7,
        clinical_events: 41_204,
        clinical_watermark: Some(91_338),
        export_covers_seq: Some(91_338),
    };
    let path = tempdir().unwrap().path().join("backup-status.json");
    write_health(&path, &health).unwrap();
    assert_eq!(read_health(&path).unwrap(), health);
}

/// The rule that keeps a stale kit detectable: a skipped export must NOT advance the field.
/// If it did, `verify-backup` would compare the medium against a coverage figure no export
/// ever achieved — the precise-untruth composite this whole programme exists to end.
#[test]
fn a_skipped_export_leaves_the_previous_coverage_untouched() {
    let previous = Some(500_i64);
    assert_eq!(export_coverage_after(previous, ExportOutcome::Skipped), previous);
    assert_eq!(export_coverage_after(previous, ExportOutcome::Written(900)), Some(900));
}
```

- [ ] **Step 2: Run to verify they fail**

```bash
cargo test -p cairn-node --lib backup:: -- --nocapture
```
Expected: FAIL — `BackupHealth` has no `clinical_events`.

- [ ] **Step 3: Implement**

```rust
pub struct BackupHealth {
    pub version: u8,
    pub last_backup_unix: i64,
    pub medium_path: String,
    pub medium_bytes: u64,
    /// Per-plane scope (#500 slice 2c). v1 carried ONE `event_count`, which is exactly how
    /// #500 stayed invisible: a true count of what the medium held, with nothing to say
    /// that what it held was the federation plane alone. A count without a scope is the
    /// honest-surface half of a dishonest composite.
    #[serde(default)]
    pub node_events: u64,
    #[serde(default)]
    pub clinical_events: u64,
    #[serde(default)]
    pub clinical_watermark: Option<i64>,
    /// `max(event_log.seq)` at the moment the export was last WRITTEN — not at the moment
    /// this sidecar was written. `None` = no export has ever been written here. Never
    /// advanced by a skipped export (`export_coverage_after`), because a coverage figure
    /// no export achieved is worse than none: it would make `verify-backup` pass.
    #[serde(default)]
    pub export_covers_seq: Option<i64>,
}

/// PURE. Which coverage figure the next sidecar carries.
pub fn export_coverage_after(previous: Option<i64>, outcome: ExportOutcome) -> Option<i64> { … }
```

Note `event_count` is **removed**, not kept alongside: two count fields that must agree is the mirror
shape this slice is elsewhere deleting. Its readers (`describe_health`, `status`) move to
`node_events + clinical_events` and say which is which.

- [ ] **Step 4: Golden-pin the sidecar (spec §9 test 7)**

A round trip cannot catch a mirrored rename — writer and reader move together and every assertion
stays green (2a's 19-of-19 lesson). Add a golden JSON pin asserting the exact serialized field names,
so renaming `clinical_watermark` on both sides fails here.

- [ ] **Step 5: Run and commit**

```bash
cargo test -p cairn-node --lib backup:: --all-targets
git commit -am "feat(#500): BackupHealth v2 reports per-plane scope"
```

---

## Task 11: the export carries the actor registry

**Files:** `crates/cairn-node/src/localstate.rs`, `localstate_read.rs`,
`crates/cairn-node/tests/localstate_wire_pins.rs`

**Interfaces:**
- Produces: `LocalState::actor_registry() -> &[Vec<u8>]`, and `from_custody`'s successor
  `from_custody_and_registry(episode_deks, unwrap_secret, actor_registry)`.

- [ ] **Step 1: Freeze the CURRENT bytes first, before the field exists**

The order is the whole method (#511): a pin taken *after* the change proves nothing about the change.

```rust
/// The exact CAIRNL1 CBOR a PRE-registry build produces, frozen 2026-09-05. Still green
/// after Task 11 = an export written by yesterday's binary still restores, and today's
/// binary still writes what yesterday's can read.
#[test]
fn the_pre_registry_bundle_bytes_are_unchanged() {
    let ls = LocalState::from_custody(vec![dek_cbor(1), dek_cbor(2)], Some(secret32(9)));
    assert_eq!(hex::encode(to_cbor(&ls)), PRE_REGISTRY_BUNDLE_HEX);
}
```

- [ ] **Step 2: Run it, and capture the real hex** — it fails printing the actual value; paste that in
  as `PRE_REGISTRY_BUNDLE_HEX` and re-run to green. **Commit this before touching `LocalState`.**

```bash
cargo test -p cairn-node --test localstate_wire_pins
git commit -am "test(#500): freeze the CAIRNL1 bytes before the registry slot moves them"
```

- [ ] **Step 3: Write the failing additive tests**

```rust
#[test]
fn an_old_bundle_still_parses_with_the_registry_absent() {
    let old = hex::decode(PRE_REGISTRY_BUNDLE_HEX).unwrap();
    let ls = from_cbor(&old).expect("an export written before the registry existed must restore");
    assert!(ls.actor_registry().is_empty(), "absent registry = empty, never a parse failure");
}

/// #511's lesson, applied: a `skip_serializing` mutant that deserializes to the empty case
/// passes every round trip. Pin the EMPTY encoding explicitly, or "the registry silently
/// never travels" is indistinguishable from "this node has no actors".
#[test]
fn the_empty_registry_encoding_is_pinned() {
    let ls = LocalState::from_custody_and_registry(vec![], None, vec![]);
    assert_eq!(hex::encode(to_cbor(&ls)), EMPTY_REGISTRY_BUNDLE_HEX);
}

#[test]
fn a_registry_row_round_trips_through_the_seal() {
    let ls = LocalState::from_custody_and_registry(vec![], None, vec![actor_row_cbor(1)]);
    let back = from_cbor(&to_cbor(&ls)).unwrap();
    assert_eq!(back.actor_registry(), &[actor_row_cbor(1)]);
}
```

- [ ] **Step 4: Implement the slot and the read**

`LocalState` gains `actor_registry: Vec<Vec<u8>>` (private, `#[serde(default)]`), an accessor, and
`from_custody_and_registry` **replacing** `from_custody` as the second producer — the producer set
stays closed at two (`empty` + one), which is the invariant `dr_clinical_guarantee_gap`'s per-file half
pins. **No `set_actor_registry`**: a setter is how a third producer sneaks in.

In `localstate_read.rs`:

```rust
    // The actor registry rides the export because it can ride nothing else: actor_event has
    // no signed_bytes and replicates nowhere, while every clinical apply door gates on
    // actor_current. Without it a restored node refuses its own history (2a §3).
    //
    // ⚠️ These rows arrive authenticated by the CONTAINER's AEAD, not by per-row signatures
    // — the one part of a restore that is not verify-on-apply. 2e's ADR owes that caveat;
    // do not let this comment be the only place it is written down.
    let registry = db
        .query(
            "SELECT actor_event_id::text, actor_id, op, kind, pinned, signing_key_id, \
                    superseded_by, seq \
             FROM actor_event ORDER BY seq",
            &[],
        )
        .await
        .context("reading the actor registry for the local-state export")?;
```

- [ ] **Step 5: Run and commit**

```bash
CAIRN_TEST_PG=... cargo test -p cairn-node --test localstate_wire_pins --test dr_clinical_guarantee_gap
git commit -am "feat(#500): the local-state export carries the actor registry"
```

---

## Task 12: the export is verified after write, and `verify-backup` refuses a stale kit

**Files:** `crates/cairn-node/src/main.rs` (`seal_and_write_local_state_export`, the `verify-backup`
arm), `crates/cairn-node/tests/verify_backup_scope.rs`

- [ ] **Step 1: Write the failing tests**

The staleness decision is a **pure function**, so the exit-code policy is testable without a database,
a medium or a CLI:

```rust
/// What `verify-backup` concluded about the kit as a WHOLE (review finding I5's shape:
/// the events being intact is not the same as the kit being restorable).
#[derive(Debug, PartialEq, Eq)]
pub enum KitVerdict {
    /// Medium and export agree: everything on the medium has custody coverage.
    Restorable,
    /// The medium holds clinical events written after the export last covered anything.
    /// Those bodies restore as ciphertext unless their medium-borne DEKs open them.
    ExportStale { medium_seq: i64, export_seq: Option<i64> },
    /// No export beside the medium, or one that could not be used.
    ExportMissing(String),
}

#[test]
fn an_export_older_than_the_medium_is_stale() {
    assert_eq!(
        kit_verdict(Some(900), Some(500)),
        KitVerdict::ExportStale { medium_seq: 900, export_seq: Some(500) }
    );
}

#[test]
fn an_export_that_covers_the_medium_is_restorable() {
    assert_eq!(kit_verdict(Some(900), Some(900)), KitVerdict::Restorable);
    // Ahead is fine: the export is written after the capture in the same run.
    assert_eq!(kit_verdict(Some(900), Some(950)), KitVerdict::Restorable);
}

#[test]
fn a_medium_with_no_clinical_events_is_not_stale() {
    // A fresh node that has never written a clinical event has nothing uncovered. Calling
    // that stale would train an operator to ignore the one signal this adds.
    assert_eq!(kit_verdict(None, None), KitVerdict::Restorable);
}

#[test]
fn a_medium_with_clinical_events_and_NO_export_is_missing_not_stale() {
    // Different remedies: "run backup with a passphrase" vs "recover the export". The
    // #502 lesson — merging unreadable/absent into one class named a remedy that refuses
    // while the file exists.
    assert!(matches!(kit_verdict(Some(900), None), KitVerdict::ExportMissing(_)));
}
```

- [ ] **Step 2: Run to verify they fail**, then implement `kit_verdict` and wire the `verify-backup`
  arm to exit **1** on anything but `Restorable`, printing which of the two it is and the remedy.

- [ ] **Step 3: Write the asymmetry test — `backup` must NOT start failing**

```rust
#[tokio::test]
async fn backup_still_exits_zero_when_the_export_is_skipped() {
    // Deliberate asymmetry, and the §1.2 constraint in test form: `backup` wrote a good
    // medium, and failing it would page an operator over a success — and would make the
    // clinical capture depend on a passphrase an unattended cron run cannot supply, which
    // is M > N. `verify-backup` is the cron HEALTH CHECK, and it is the one that refuses.
    std::env::remove_var("CAIRN_KEY_PASSPHRASE");
    let report = backup::backup_to(&c, &medium, &health, now, None).await;
    assert!(report.is_ok(), "a passphrase-less capture must still write the medium");
    let image = parse_any(&std::fs::read(&medium).unwrap()).unwrap();
    assert!(!clinical_records(&image).is_empty(), "and it must still carry the clinical plane");
}
```

- [ ] **Step 4: Add the export's read-after-write**

In `seal_and_write_local_state_export`, after `atomic_write`: re-read the file, `parse_sidecar` it, and
confirm the sealed payload is present and the container parses — **before** `export_covers_seq`
advances. The medium has had this since slice B; the export, which is the only artifact carrying this
node's custody key off the machine, has not.

- [ ] **Step 5: Run and commit**

```bash
CAIRN_TEST_PG=... cargo test -p cairn-node --test verify_backup_scope --no-fail-fast
git commit -am "feat(#500): the export is verified after write, and a stale kit fails verify-backup"
```

---

## Task 13: the guarantee tests, the point-in-time pin, and the §1.2 measurement

**Files:** `crates/cairn-node/tests/dr_clinical_guarantee_gap.rs`,
`crates/cairn-node/tests/medium_point_in_time.rs`, `docs/HANDOVER.md`, `docs/ROADMAP.md`

- [ ] **Step 1: Invert the gap pin.** `medium_carries_the_federation_plane_and_no_clinical_event`
  becomes `medium_carries_both_planes`, and a **new** half pins what is still true:
  `nothing_yet_restores_a_clinical_event_from_a_medium` — so 2c cannot be read as closing #500.

- [ ] **Step 2: Write the point-in-time test** — the most important test in the slice:

```rust
#[tokio::test]
async fn a_medium_restores_the_state_at_capture_time() {
    // NOT a leak, and named so nobody "fixes" it. A body readable when the medium was
    // taken is still readable in that medium after a later shred: that is what a backup
    // IS (the maintainer's definition, spec §2.1). A body shredded BEFORE the capture
    // never gets its DEK written at all.
    let (id_a, _) = author_sealed_clinical_event(&c, &sk, &kid).await;
    backup::backup_to(...).await.unwrap();                    // capture with A readable
    shred(&c, id_a).await;                                     // erase A afterwards
    let image = parse_any(&std::fs::read(&medium_path).unwrap()).unwrap();
    assert!(dek_for(&image, id_a).is_some(),
        "the medium reproduces the state at capture time — a later shred does not reach \
         a segment already written, and rewriting one would forfeit the integrity \
         guarantee that is the core's job (spec §2.1)");

    let (id_b, _) = author_sealed_clinical_event(&c, &sk, &kid).await;
    shred(&c, id_b).await;                                     // erase B BEFORE capture
    backup::backup_to(...).await.unwrap();
    let image = parse_any(&std::fs::read(&medium_path).unwrap()).unwrap();
    assert!(dek_for(&image, id_b).is_none(),
        "a body shredded before its first capture never has its DEK written");
}
```

- [ ] **Step 3: Measure the §1.2 budget** and record the numbers in the plan's benchmark table:
  unchanged-log capture < 2 s; 10 000 fresh events < 60 s. **Outside budget ⇒ file an issue.**

- [ ] **Step 4: Run the full gate** — budget hours, start it in the background:

```bash
scripts/run-db-gated-tests.sh 2>&1 | tee /tmp/2c-gate.log
```

- [ ] **Step 5: Update HANDOVER and ROADMAP**, then open the PR. The PR body says
  `Closes #522` and `Closes #524`, and states that **#500 stays open** — phrased so the closing-keyword
  guard accepts it (keep the keyword away from the reference).
