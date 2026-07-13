# twin-check registry dispatch (#173) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the verbatim-copied `cairn_event_twin` IF/ELSIF dispatch chain (re-declared in 11 migrations) with one stable dispatcher over a locked registry table, unifying all per-type check functions on `(p_type text, b jsonb)` — with **zero behaviour change**.

**Architecture:** A new locked table `cairn_event_twin_check(event_type, check_fn, twin_required_msg)` in db/005, plus a self-enforcing `BEFORE INSERT/UPDATE` validation trigger, drives a single dynamic dispatcher declared once in db/005. Each slice migration drops its copied chain and registers one additive row instead. The 4 legacy `(b jsonb)` check fns are migrated to the uniform `(p_type text, b jsonb)` signature (the other 5 already have it).

**Tech Stack:** PostgreSQL 18 + PL/pgSQL (the in-DB floor), `cairn_pgx` (COSE/Ed25519 verify), Rust `tokio-postgres` DB-gated tests, `cargo test`/`fmt`/`clippy`, mkdocs.

## Global Constraints

- **Zero behaviour change.** Every currently-accepted event stays accepted with the identical stored twin; every currently-rejected event stays rejected with the identical message text (`submit_event: <msg>`, byte-for-byte).
- **AGPL-3.0**; no new dependencies.
- **TDD** — failing test first; this is the safety-critical §9 in-DB floor.
- **Reviewer-legible inline docs** for a junior contributor on every non-trivial SQL object.
- **Migration replay:** `db.rs` re-runs ALL `db/*.sql` in filename order on every connect. Every migration edit must be replay-idempotent (`CREATE OR REPLACE`, `DROP … IF EXISTS`, `INSERT … ON CONFLICT DO NOTHING`). A later migration must not append columns to an earlier `CREATE OR REPLACE VIEW`.
- **Atomicity:** the "flip" (Task 2) is irreducible — because the last-loaded chain-copy wins and calls the old check-fn signatures, the signature unification + dispatcher swap + removal of ALL chain-copies + all seed INSERTs must land in one commit. Do not split it.
- **DB-gated tests** need `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test"` (PG18 + cairn_pgx on the Mac :5532 cluster). They self-serialize cluster-wide via `db::test_serial_guard`.
- **No new `db/` file** — the registry lives in db/005 (already in the `db.rs` `SCHEMA` list); `db.rs` is not edited.

---

## File Structure

- `db/005_submit.sql` — **modify**: add the registry table + validation trigger + REVOKEs (Task 1); replace the dispatcher with the registry-driven one (Task 2).
- `db/010, 011, 018, 025` — **modify**: unify the 4 legacy check-fn signatures (Task 2).
- `db/014` — **modify**: `demographic_field` signature (CREATE OR REPLACE only) (Task 2).
- `db/010, 011, 015, 018, 023, 024, 025, 031, 032, 033` — **modify**: remove the copied `cairn_event_twin` block; add the registry INSERT(s) (Task 2).
- `crates/cairn-node/tests/twin_registry.rs` — **create**: DB-gated registry-mechanism tests (Task 1 trigger tests; Task 3 dispatch tests).
- `crates/cairn-node/tests/twin_dispatch_single_source.rs` — **create**: no-DB source guard (Task 2).
- `db/tests/034_twin_registry_test.sql` — **create**: SQL mirror of the registry mechanism (Task 3).
- `docs/spec/decisions/0048-twin-check-registry-dispatch.md` — **create**: ADR-0048 (Task 4).
- `mkdocs.yml`, `docs/spec/decisions/README.md`, `docs/spec/index.md`, `docs/HANDOVER.md`, `docs/ROADMAP.md` — **modify**: nav/index/version/handover (Task 4).

---

## Task 1: Registry table + validation trigger (db/005)

Add the new machinery to db/005 **without** touching the dispatcher yet — the copied chains stay live, so runtime behaviour is unchanged. TDD the trigger (a new object).

**Files:**
- Modify: `db/005_submit.sql` (insert after the `event_type_class` block, ~line 25)
- Create: `crates/cairn-node/tests/twin_registry.rs`

**Interfaces:**
- Produces: table `cairn_event_twin_check(event_type text PK, check_fn text, twin_required_msg text)`; trigger `cairn_event_twin_check_validate` calling `cairn_check_twin_registry_fn()`; both readable/insertable by the DB owner (migrations/tests), locked from PUBLIC.

- [ ] **Step 1: Write the failing test**

Create `crates/cairn-node/tests/twin_registry.rs`:

```rust
//! #173 — the cairn_event_twin registry. DB-gated on $CAIRN_TEST_PG, serialized
//! cluster-wide via db::test_serial_guard (same idiom as medication.rs). Part 1 (this
//! file, Task 1): the registry table's validation trigger fails closed on a check_fn that
//! does not exist with the unified (text, jsonb) signature. Part 2 (Task 3) adds per-type
//! dispatch tests.
use cairn_node::db;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// The real RAISE EXCEPTION text (tokio_postgres wraps DB errors as a generic "db error").
fn db_msg(e: &tokio_postgres::Error) -> String {
    e.as_db_error()
        .map(|d| d.message().to_string())
        .unwrap_or_else(|| e.to_string())
}

#[tokio::test]
async fn registry_trigger_rejects_missing_check_fn() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    // A registry row naming a function that does not exist is refused at insert time.
    let err = c
        .execute(
            "INSERT INTO cairn_event_twin_check (event_type, check_fn, twin_required_msg) \
             VALUES ('test.bogus.asserted', 'cairn_check_does_not_exist', 'x')",
            &[],
        )
        .await
        .expect_err("bogus check_fn must be rejected");
    assert!(
        db_msg(&err).contains("does not exist"),
        "unexpected: {}",
        db_msg(&err)
    );

    // A row naming an existing (text, jsonb) check fn is accepted, then cleaned up.
    c.execute(
        "INSERT INTO cairn_event_twin_check (event_type, check_fn, twin_required_msg) \
         VALUES ('test.ok.asserted', 'cairn_check_medication_assertion', 'x')",
        &[],
    )
    .await
    .expect("valid (text,jsonb) check fn must be accepted");
    c.execute(
        "DELETE FROM cairn_event_twin_check WHERE event_type = 'test.ok.asserted'",
        &[],
    )
    .await
    .unwrap();

    // A row with NULL check_fn (twin-required-only, no structural check) is accepted.
    c.execute(
        "INSERT INTO cairn_event_twin_check (event_type, check_fn, twin_required_msg) \
         VALUES ('test.nullfn.asserted', NULL, 'x')",
        &[],
    )
    .await
    .expect("NULL check_fn must be accepted");
    c.execute(
        "DELETE FROM cairn_event_twin_check WHERE event_type = 'test.nullfn.asserted'",
        &[],
    )
    .await
    .unwrap();
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test twin_registry registry_trigger_rejects_missing_check_fn -- --nocapture`
Expected: FAIL — `relation "cairn_event_twin_check" does not exist` (table not created yet).

- [ ] **Step 3: Add the registry table, trigger, and REVOKEs to db/005**

In `db/005_submit.sql`, immediately AFTER the `event_type_class` INSERT block (after the `ON CONFLICT (event_type) DO NOTHING;` around line 25) and BEFORE the `cairn_twin_skeleton` function, insert:

```sql
-- Per-type twin/floor-check registry (#173, ADR-0048). Sibling of event_type_class:
-- a new event type registers its structural check + twin requirement by INSERTing ONE
-- row here (additive), instead of copying the whole cairn_event_twin dispatch chain into
-- a new migration. The single stable dispatcher (below) reads this table. Columns are
-- independent: check_fn NULL ⇒ no structural floor for this type; twin_required_msg NULL
-- ⇒ an absent authored twin degrades honestly to a skeleton (ADR-0039) rather than raising.
CREATE TABLE IF NOT EXISTS cairn_event_twin_check (
    event_type         TEXT PRIMARY KEY,
    check_fn           TEXT,
    twin_required_msg  TEXT
);

-- Fail-closed at REGISTRATION time (not first-call): a registered check_fn must exist with
-- the unified (text, jsonb) signature. A slice that registers a typo'd or not-yet-created
-- check fn fails loudly on schema load, for this migration and every future one, with
-- nothing to remember. (to_regprocedure returns NULL for an absent function; valid type
-- names never raise.) Residual: this validates registration, not later function mutation —
-- a migration that broke a check fn's signature afterwards would surface at runtime
-- (the dispatcher's EXECUTE raises, still fail-closed).
CREATE OR REPLACE FUNCTION cairn_check_twin_registry_fn()
RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.check_fn IS NOT NULL
       AND to_regprocedure(NEW.check_fn || '(text, jsonb)') IS NULL THEN
        RAISE EXCEPTION 'cairn_event_twin_check: check_fn %(text, jsonb) does not exist (fail closed)', NEW.check_fn;
    END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS cairn_event_twin_check_validate ON cairn_event_twin_check;
CREATE TRIGGER cairn_event_twin_check_validate
    BEFORE INSERT OR UPDATE ON cairn_event_twin_check
    FOR EACH ROW EXECUTE FUNCTION cairn_check_twin_registry_fn();

-- Safety surface (like event_type_class): a row pointing a type's check at a no-op would
-- drop its floor. Lock it down; submit_event reads it as its SECURITY DEFINER owner, so
-- cairn_agent needs no grant.
REVOKE INSERT, UPDATE, DELETE ON cairn_event_twin_check FROM PUBLIC;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test twin_registry -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Verify the whole suite stays green (behaviour unchanged — chains still live)**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node`
Expected: PASS (all existing DB-gated tests green; the registry table is additive and unused by the still-live chains).

- [ ] **Step 6: fmt + commit**

```bash
cargo fmt -p cairn-node
git add db/005_submit.sql crates/cairn-node/tests/twin_registry.rs
git commit -m "feat(db/005): twin-check registry table + fail-closed validation trigger (#173)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: The atomic flip — registry dispatcher, signature unification, chain removal, seed rows

Anchor the flip with the no-DB single-source guard (RED with 11 declarations → GREEN with 1), then make all coupled edits in one commit. **Do not split** (see Global Constraints › Atomicity).

**Files:**
- Create: `crates/cairn-node/tests/twin_dispatch_single_source.rs`
- Modify: `db/005` (dispatcher), `db/010`, `db/011`, `db/014`, `db/015`, `db/018`, `db/023`, `db/024`, `db/025`, `db/031`, `db/032`, `db/033`

**Interfaces:**
- Consumes: `cairn_event_twin_check` (Task 1); the 9 `cairn_check_*` functions; `cairn_twin_skeleton` (db/015).
- Produces: the stable `cairn_event_twin(p_type text, b jsonb)` dispatcher (db/005, the only declaration); all 9 check fns at `(p_type text, b jsonb) RETURNS void`; 15 seed rows in `cairn_event_twin_check`.

- [ ] **Step 1: Write the failing single-source guard test**

Create `crates/cairn-node/tests/twin_dispatch_single_source.rs`:

```rust
//! #173 — the cairn_event_twin dispatcher must be declared in EXACTLY ONE migration
//! (db/005). The prior copy-hazard was that each slice re-declared the whole IF/ELSIF
//! chain, so a stale copy could silently drop a floor check. This is a SOURCE-LEVEL guard
//! (no DB needed): it scans db/*.sql and fails if more than one file declares the function,
//! catching any re-introduction of the copy pattern in every `cargo test` / CI run.
use std::fs;
use std::path::PathBuf;

/// Repo-root db/ directory. CARGO_MANIFEST_DIR is crates/cairn-node; db/ is two levels up.
fn db_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../db")
        .canonicalize()
        .expect("db/ dir")
}

#[test]
fn cairn_event_twin_is_declared_in_exactly_one_migration() {
    let needle = "CREATE OR REPLACE FUNCTION cairn_event_twin(";
    let mut declaring: Vec<String> = Vec::new();
    for entry in fs::read_dir(db_dir()).expect("read db/") {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("sql") {
            continue;
        }
        let sql = fs::read_to_string(&path).expect("read sql");
        if sql.contains(needle) {
            declaring.push(path.file_name().unwrap().to_string_lossy().into_owned());
        }
    }
    declaring.sort();
    assert_eq!(
        declaring,
        vec!["005_submit.sql".to_string()],
        "cairn_event_twin must be declared ONLY in db/005 (#173); found in: {declaring:?}"
    );
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p cairn-node --test twin_dispatch_single_source`
Expected: FAIL — `declaring` lists 11 files (005, 010, 011, 015, 018, 023, 024, 025, 031, 032, 033).

- [ ] **Step 3: Unify the 4 legacy check-fn signatures**

In `db/010_demographics.sql`, replace the `cairn_check_identifier_assertion` declaration header. Change:
```sql
CREATE OR REPLACE FUNCTION cairn_check_identifier_assertion(b jsonb)
```
to (add the DROP line immediately before it, and add `p_type text` — unused, the fn validates the body):
```sql
-- Signature unified to (p_type text, b jsonb) for the #173 registry dispatch; p_type is
-- unused here (this check validates the body). DROP clears any stale (jsonb) overload on
-- an upgraded-in-place dev DB.
DROP FUNCTION IF EXISTS cairn_check_identifier_assertion(jsonb);
CREATE OR REPLACE FUNCTION cairn_check_identifier_assertion(p_type text, b jsonb)
```

In `db/011_demographics_fields.sql`, same treatment for `cairn_check_demographic_field` (this is the EARLIEST declaration, so the DROP lives here):
```sql
DROP FUNCTION IF EXISTS cairn_check_demographic_field(jsonb);
CREATE OR REPLACE FUNCTION cairn_check_demographic_field(p_type text, b jsonb)
```

In `db/014_demographics_address.sql`, `cairn_check_demographic_field` is re-declared (the live version). Change ONLY the header (no DROP — db/011 already dropped the old overload earlier in load order):
```sql
CREATE OR REPLACE FUNCTION cairn_check_demographic_field(p_type text, b jsonb)
```

In `db/018_identity_linkage.sql`, same as db/010 for `cairn_check_link_assertion`:
```sql
DROP FUNCTION IF EXISTS cairn_check_link_assertion(jsonb);
CREATE OR REPLACE FUNCTION cairn_check_link_assertion(p_type text, b jsonb)
```

In `db/025_identity_repudiate.sql`, same for `cairn_check_repudiation_assertion`:
```sql
DROP FUNCTION IF EXISTS cairn_check_repudiation_assertion(jsonb);
CREATE OR REPLACE FUNCTION cairn_check_repudiation_assertion(p_type text, b jsonb)
```

Leave the function BODIES unchanged. Leave the 5 already-`(p_type text, b jsonb)` fns (`dispute_assertion`, `identity_state_assertion`, `medication_assertion`, `medication_dose`, `medication_reconciliation`) untouched.

- [ ] **Step 4: Replace db/005's dispatcher with the registry-driven one**

In `db/005_submit.sql`, replace the existing trivial `cairn_event_twin` (the `CREATE OR REPLACE FUNCTION cairn_event_twin(p_type text, b jsonb) … BEGIN RETURN cairn_twin_skeleton(p_type, b); END; $$;` block, ~lines 35–45) with:

```sql
-- The single, stable per-event-type twin hook (§3.13/§4.5, #173/ADR-0048). Declared ONCE
-- here and never re-declared — a new event type registers a cairn_event_twin_check row in
-- its own migration (additive), so no slice ever copies this dispatch body (the prior
-- copy-a-stale-chain floor-regression hazard is designed out). Returns the plaintext twin
-- and, for a registered type, runs its structural floor (raising on violation).
--
-- Dispatch is dynamic: the check_fn name comes from the LOCKED, migration-only registry
-- table (never user input) and %I quotes it; a missing/mis-signed fn RAISES (fail-closed),
-- though the registry trigger already refused an unregistered fn at load time. The
-- EXECUTE 'SELECT fn($1,$2)' form is the dynamic equivalent of PERFORM fn(...) (every
-- check fn RETURNS void and works by RAISE-on-violation).
--
-- Twin policy (ADR-0039): an authored twin is carried verbatim for EVERY type; if absent,
-- a type with twin_required_msg RAISES (demographics + identity + medication hard-require
-- it), and every other type degrades honestly to a mechanical skeleton.
CREATE OR REPLACE FUNCTION cairn_event_twin(p_type text, b jsonb)
RETURNS text LANGUAGE plpgsql AS $$
DECLARE
    v_twin     text    := b ->> 'plaintext_twin';
    v_authored boolean := v_twin IS NOT NULL AND length(regexp_replace(v_twin, '\s+', '', 'g')) > 0;
    v_fn       text;
    v_msg      text;
BEGIN
    SELECT check_fn, twin_required_msg INTO v_fn, v_msg
        FROM cairn_event_twin_check WHERE event_type = p_type;

    IF v_fn IS NOT NULL THEN
        EXECUTE format('SELECT %I($1, $2)', v_fn) USING p_type, b;
    END IF;

    IF v_authored THEN
        RETURN v_twin;
    END IF;
    IF v_msg IS NOT NULL THEN
        RAISE EXCEPTION 'submit_event: %', v_msg;
    END IF;
    RETURN cairn_twin_skeleton(p_type, b);
END;
$$;
```

- [ ] **Step 5: Remove the copied chain from db/015 and register nothing (it added no type)**

In `db/015_globalise_twin.sql`, DELETE the entire `CREATE OR REPLACE FUNCTION cairn_event_twin(p_type text, b jsonb) … $$;` block (the generalised dispatcher, ~lines 29–59). KEEP everything else in the file (`cairn_twin_skeleton`, `cairn_twin_is_authored`, `cairn_twin_provenance_of`, `event_twin_provenance`, the GRANT). Add a one-line comment where the block was:
```sql
-- (The per-type twin dispatch moved to the db/005 registry dispatcher — #173/ADR-0048.
--  This migration keeps the improved skeleton + the twin-provenance read surfaces below.)
```

- [ ] **Step 6: Remove the copied chain + add the registry INSERT(s) in the 8 type-registering migrations**

In EACH of the migrations below, DELETE its `CREATE OR REPLACE FUNCTION cairn_event_twin(...) … $$;` block, and add the INSERT immediately AFTER that migration's existing `INSERT INTO event_type_class (...) … ;` block (so the two registrations sit together), BEFORE `COMMIT;`. Use the exact rows:

`db/010_demographics.sql`:
```sql
INSERT INTO cairn_event_twin_check (event_type, check_fn, twin_required_msg) VALUES
    ('demographic.identifier.asserted', 'cairn_check_identifier_assertion',
     'demographic assertion requires a non-empty authored twin (§4.5)')
ON CONFLICT (event_type) DO NOTHING;
```

`db/011_demographics_fields.sql`:
```sql
INSERT INTO cairn_event_twin_check (event_type, check_fn, twin_required_msg) VALUES
    ('demographic.field.asserted', 'cairn_check_demographic_field',
     'demographic assertion requires a non-empty authored twin (§4.5)')
ON CONFLICT (event_type) DO NOTHING;
```

`db/018_identity_linkage.sql`:
```sql
INSERT INTO cairn_event_twin_check (event_type, check_fn, twin_required_msg) VALUES
    ('identity.link.asserted',   'cairn_check_link_assertion', 'identity linkage assertion requires a non-empty authored twin (§5.7)'),
    ('identity.unlink.asserted', 'cairn_check_link_assertion', 'identity linkage assertion requires a non-empty authored twin (§5.7)')
ON CONFLICT (event_type) DO NOTHING;
```

`db/023_identity_dispute.sql`:
```sql
INSERT INTO cairn_event_twin_check (event_type, check_fn, twin_required_msg) VALUES
    ('identity.dispute.asserted', 'cairn_check_dispute_assertion', 'identity dispute assertion requires a non-empty authored twin (§5.7)'),
    ('identity.dispute.resolved', 'cairn_check_dispute_assertion', 'identity dispute assertion requires a non-empty authored twin (§5.7)')
ON CONFLICT (event_type) DO NOTHING;
```

`db/024_identity_identify.sql`:
```sql
INSERT INTO cairn_event_twin_check (event_type, check_fn, twin_required_msg) VALUES
    ('identity.pending.asserted',  'cairn_check_identity_state_assertion', 'identity-state assertion requires a non-empty authored twin (§5.7)'),
    ('identity.identify.asserted', 'cairn_check_identity_state_assertion', 'identity-state assertion requires a non-empty authored twin (§5.7)')
ON CONFLICT (event_type) DO NOTHING;
```

`db/025_identity_repudiate.sql`:
```sql
INSERT INTO cairn_event_twin_check (event_type, check_fn, twin_required_msg) VALUES
    ('identity.repudiate.asserted', 'cairn_check_repudiation_assertion', 'identity repudiation assertion requires a non-empty authored twin (§5.7)')
ON CONFLICT (event_type) DO NOTHING;
```

`db/031_medication.sql` (also DELETE the header comment warning about the copy hazard — it is now designed out):
```sql
INSERT INTO cairn_event_twin_check (event_type, check_fn, twin_required_msg) VALUES
    ('clinical.medication.asserted',           'cairn_check_medication_assertion', 'medication assertion requires a non-empty authored twin (§3.13/§3.15)'),
    ('clinical.medication-cessation.asserted', 'cairn_check_medication_assertion', 'medication assertion requires a non-empty authored twin (§3.13/§3.15)')
ON CONFLICT (event_type) DO NOTHING;
```

`db/032_medication_dose.sql`:
```sql
INSERT INTO cairn_event_twin_check (event_type, check_fn, twin_required_msg) VALUES
    ('clinical.medication-dose-change.asserted',     'cairn_check_medication_dose', 'medication dose assertion requires a non-empty authored twin (§3.13/§3.15)'),
    ('clinical.medication-dose-correction.asserted', 'cairn_check_medication_dose', 'medication dose assertion requires a non-empty authored twin (§3.13/§3.15)')
ON CONFLICT (event_type) DO NOTHING;
```

`db/033_medication_reconciliation.sql`:
```sql
INSERT INTO cairn_event_twin_check (event_type, check_fn, twin_required_msg) VALUES
    ('clinical.medication-reconciliation.asserted', 'cairn_check_medication_reconciliation', 'medication reconciliation requires a non-empty authored twin (§3.13/§3.15)'),
    ('clinical.medication-separation.asserted',     'cairn_check_medication_reconciliation', 'medication reconciliation requires a non-empty authored twin (§3.13/§3.15)')
ON CONFLICT (event_type) DO NOTHING;
```

- [ ] **Step 7: Run the single-source guard — now GREEN**

Run: `cargo test -p cairn-node --test twin_dispatch_single_source`
Expected: PASS (only `005_submit.sql` declares `cairn_event_twin`).

- [ ] **Step 8: Run the FULL workspace suite — behaviour preserved**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test --workspace`
Expected: PASS — every existing DB-gated suite (`demographics*`, `demographics_fields`, `demographics_names`, `demographics_sex_gender`, `demographics_address`, `identity_linkage`, `identity_dispute`, `identity_identify`, `identity_repudiate`, `medication`, `medication_dose`, `medication_reconciliation`, `twin_globalise`, `apply_remote_event`, `suppression_owner_gate`, `observed_evidence`, `photo_evidence`, `identity_evidence_text`) stays green, proving the dispatch is inert. If any negative-path test's asserted message changed, STOP — a message drifted; re-check the seed rows against db/033's old chain.

- [ ] **Step 9: clippy + fmt**

Run: `cargo clippy --workspace -- -D warnings` then `cargo fmt --all`
Expected: clean.

- [ ] **Step 10: Commit**

```bash
git add -A
git commit -m "refactor(db): registry-driven cairn_event_twin dispatch; unify check-fn signatures (#173)

Single stable dispatcher in db/005 over cairn_event_twin_check; remove the
copied IF/ELSIF chain from db/010-033; unify the 4 legacy check fns to
(p_type text, b jsonb). No behaviour change. Guarded by twin_dispatch_single_source.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: Registry dispatch contract tests (Rust + SQL mirror)

The registry is now live. Lock its contract with explicit tests beyond the regression signal.

**Files:**
- Modify: `crates/cairn-node/tests/twin_registry.rs` (add dispatch tests)
- Create: `db/tests/034_twin_registry_test.sql`

**Interfaces:**
- Consumes: the live registry + dispatcher (Task 2); `submit_event`; `cairn_event`/`cairn_node` helpers already used by `medication.rs`.

- [ ] **Step 1: Add dispatch tests to twin_registry.rs**

Append to `crates/cairn-node/tests/twin_registry.rs` (reuse the `connect`/`db_msg`/`cs` helpers from Task 1). These prove: (a) the registry actually seeds the expected mapping; (b) a registered type's structural check fires through dispatch; (c) an unregistered-but-classified type gets the skeleton (no raise); (d) fail-closed re-verified at the live door.

JSON bodies are passed as a `&str` cast to `::jsonb` in SQL (no reliance on a
tokio-postgres serde_json ToSql feature). Reuse the `cs`/`db_msg` helpers from Task 1.

```rust
#[tokio::test]
async fn registry_is_seeded_with_the_expected_mapping() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    // Assert the full 15-row mapping is present so a dropped registration is caught.
    let n: i64 = c
        .query_one("SELECT count(*) FROM cairn_event_twin_check", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 15, "expected 15 seeded twin-check rows");

    // Spot-check a representative mapping.
    let row = c
        .query_one(
            "SELECT check_fn, twin_required_msg FROM cairn_event_twin_check \
             WHERE event_type = 'identity.link.asserted'",
            &[],
        )
        .await
        .unwrap();
    let fn_name: String = row.get(0);
    let msg: String = row.get(1);
    assert_eq!(fn_name, "cairn_check_link_assertion");
    assert!(msg.contains("§5.7"));
}

#[tokio::test]
async fn dispatch_runs_the_registered_structural_check() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    // Call the dispatcher directly with a structurally-invalid link body (empty payload →
    // no subjects). cairn_check_link_assertion must fire and RAISE — proof the registry
    // dispatched to a check, not the skeleton. (An authored twin is present, so a raise can
    // only come from the structural check running BEFORE the authored-twin return.)
    let body = r#"{"schema_version":"identity.link/1",
                   "patient_id":"00000000-0000-0000-0000-000000000001",
                   "plaintext_twin":"linked","payload":{}}"#;
    let err = c
        .query_one("SELECT cairn_event_twin('identity.link.asserted', $1::jsonb)", &[&body])
        .await
        .expect_err("an invalid link body must be refused by the dispatched check");
    assert!(!db_msg(&err).is_empty());
}

#[tokio::test]
async fn unregistered_type_gets_skeleton_no_raise() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    // A type with no registry row and no authored twin returns the mechanical skeleton and
    // does NOT raise (honest degradation, ADR-0039) — matches note.added behaviour today.
    let body = r#"{"schema_version":"note/1",
                   "patient_id":"00000000-0000-0000-0000-000000000001",
                   "payload":{"text":"hi"}}"#;
    let twin: String = c
        .query_one("SELECT cairn_event_twin('note.added', $1::jsonb)", &[&body])
        .await
        .expect("unregistered type must not raise")
        .get(0);
    assert!(twin.contains("[note.added]"));
}
```

`serde_json` is already a dependency of `cairn-node`, but these tests do not need it (JSON is
passed as a string literal). No `Cargo.toml` change required.

- [ ] **Step 2: Run the Rust dispatch tests**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test twin_registry`
Expected: PASS (all trigger + dispatch tests).

- [ ] **Step 3: Write the SQL mirror**

Create `db/tests/034_twin_registry_test.sql` (co-located with the floor tests; run in the CI floor job). Mirror the fail-closed + dispatch facts in-DB:

```sql
-- #173 — twin-check registry mechanism, SQL mirror of crates/cairn-node/tests/twin_registry.rs.
-- Run after the schema is loaded. Uses a transaction that ROLLBACKs so it leaves no residue.
BEGIN;

-- 1. Fail-closed: a bogus check_fn is refused at registration.
DO $$
BEGIN
    INSERT INTO cairn_event_twin_check (event_type, check_fn, twin_required_msg)
        VALUES ('test.bogus', 'cairn_check_nope', 'x');
    RAISE EXCEPTION 'FAIL: bogus check_fn was accepted';
EXCEPTION WHEN others THEN
    IF position('does not exist' in SQLERRM) = 0 THEN
        RAISE EXCEPTION 'FAIL: wrong error: %', SQLERRM;
    END IF;
END $$;

-- 2. The registry carries the full 15-row mapping.
DO $$
DECLARE n int;
BEGIN
    SELECT count(*) INTO n FROM cairn_event_twin_check;
    IF n <> 15 THEN RAISE EXCEPTION 'FAIL: expected 15 twin-check rows, got %', n; END IF;
END $$;

-- 3. Dispatch runs the registered check: a self-link raises via the dispatcher.
DO $$
BEGIN
    PERFORM cairn_event_twin('identity.link.asserted', jsonb_build_object(
        'schema_version','identity.link/1',
        'patient_id','00000000-0000-0000-0000-000000000001',
        'plaintext_twin','linked',
        'payload', jsonb_build_object(
            'subject_a','00000000-0000-0000-0000-0000000000aa',
            'subject_b','00000000-0000-0000-0000-0000000000aa',
            'provenance','test')));
    RAISE EXCEPTION 'FAIL: self-link was not refused';
EXCEPTION WHEN others THEN
    IF position('FAIL:' in SQLERRM) = 1 THEN RAISE; END IF;  -- re-raise our own failure
END $$;

ROLLBACK;
```

- [ ] **Step 4: Run the SQL mirror**

Run: `PGPASSWORD= psql "host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" -v ON_ERROR_STOP=1 -f db/tests/034_twin_registry_test.sql`
Expected: no error output; final `ROLLBACK`.

- [ ] **Step 5: fmt + commit**

```bash
cargo fmt -p cairn-node
git add crates/cairn-node/tests/twin_registry.rs db/tests/034_twin_registry_test.sql
git commit -m "test(twin-registry): dispatch + seed + fail-closed contract (Rust + SQL mirror) (#173)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: ADR-0048 + docs + HANDOVER/ROADMAP

**Files:**
- Create: `docs/spec/decisions/0048-twin-check-registry-dispatch.md`
- Modify: `mkdocs.yml`, `docs/spec/decisions/README.md`, `docs/spec/index.md`, `docs/HANDOVER.md`, `docs/ROADMAP.md`

- [ ] **Step 1: Write ADR-0048**

Create `docs/spec/decisions/0048-twin-check-registry-dispatch.md` following the ADR-0045 structure (Status / Date / Context / Decision / Consequences). Content to capture:
- **Status:** Accepted (refines [ADR-0039](0039-globalise-authored-legibility-twin.md), [ADR-0022](0022-validated-submit-surface-the-write-path.md); closes [#173](https://github.com/cairn-ehr/cairn-ehr/issues/173)). Date 2026-07-13.
- **Context:** the verbatim-copy hazard — 11 migrations re-declared `cairn_event_twin`, each copying the growing chain; a stale copy silently drops a floor check (safety regression, no error). db/031's header warned about it.
- **Decision:** the per-type structural check + twin requirement is a **registry row** (`cairn_event_twin_check`), not a copied dispatch branch. The dispatcher is declared **once** (db/005) and dispatches dynamically over the locked table. All per-type check fns share one signature `(p_type text, b jsonb) RETURNS void`. A registered check fn is validated to exist at load time (fail-closed trigger). A new event type registers one additive row and never touches the dispatcher.
- **Consequences:** invariant binding future slices (register-via-row; unified signature; single-source dispatcher, enforced by `twin_dispatch_single_source.rs`); introduces the first dynamic SQL in the floor (bounded: locked table, `%I`, fail-closed, load-time validation); `event_type_class` deliberately NOT merged (future convergence); no wire/behaviour/spec-prose change.

- [ ] **Step 2: Register the ADR in nav + index**

In `mkdocs.yml`, after the ADR-0047 nav line (~line 153), add:
```yaml
      - ADR-0048 · Twin-check registry dispatch: spec/decisions/0048-twin-check-registry-dispatch.md
```
In `docs/spec/decisions/README.md`, add the ADR-0048 row following the existing table/list format (match how 0047 is listed).

- [ ] **Step 3: Bump the spec version**

In `docs/spec/index.md` line 9, change `**Spec version:** 0.48` to `**Spec version:** 0.49`. (Per project convention each ADR bumps the version string; there is no spec *prose* change — this ADR is below the spec line.)

- [ ] **Step 4: Build the docs to verify nav/links**

Run: `uv run --with-requirements docs/requirements.txt -- mkdocs build`
Expected: builds clean, no broken-link/nav warnings for the new ADR.

- [ ] **Step 5: Update HANDOVER.md + ROADMAP.md**

- HANDOVER.md: add a top "This session (2026-07-13)" block summarizing the #173 refactor (registry dispatch, signature unification, ADR-0048, spec v0.48→0.49, no behaviour change); move the medication-slice-3 block down as prior-session. Prune to stay concise.
- ROADMAP.md Phase 2: add a bullet under the in-DB floor noting the twin-check registry (#173/ADR-0048) closed the copy-hazard; single-source dispatcher enforced by guard test.

- [ ] **Step 6: Commit**

```bash
git add docs/spec/decisions/0048-twin-check-registry-dispatch.md mkdocs.yml docs/spec/decisions/README.md docs/spec/index.md docs/HANDOVER.md docs/ROADMAP.md
git commit -m "docs(adr): ADR-0048 twin-check registry dispatch; spec v0.49; handover/roadmap (#173)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Final verification (before PR)

- [ ] `cargo test --workspace` (with `CAIRN_TEST_PG`) — all green.
- [ ] `cargo clippy --workspace -- -D warnings` — clean.
- [ ] `cargo fmt --all --check` — clean.
- [ ] `cargo test -p cairn-node --test twin_dispatch_single_source` — GREEN (1 declaration).
- [ ] `uv run --with-requirements docs/requirements.txt -- mkdocs build` — clean.
- [ ] Grep sanity: `grep -rl "CREATE OR REPLACE FUNCTION cairn_event_twin(" db/` returns ONLY `db/005_submit.sql`.
- [ ] Grep sanity: `grep -rn "cairn_check_.*_assertion(b jsonb)\|cairn_check_.*_field(b jsonb)\|cairn_check_link_assertion(b jsonb)\|cairn_check_repudiation_assertion(b jsonb)" db/` returns nothing (all unified).
- [ ] Whole-branch review, then open PR linking #173.

## Self-review notes (author)

- **Spec coverage:** registry table + trigger (Task 1); dispatcher + signature unification + chain removal + 15 seed rows (Task 2); single-source guard (Task 2); dispatch/fail-closed contract tests (Task 3); ADR-0048 + docs + scope-guard note (Task 4). All spec sections covered.
- **Behaviour preservation:** enforced by the full existing suite staying green (Task 2 Step 8) — the strongest signal; seed rows transcribed verbatim from db/033's winning chain.
- **Atomicity honored:** the flip is one commit (Task 2) per the coupling analysis; Task 1 (additive machinery) and Task 3/4 (additive tests/docs) are safely separable.
