# Generic Reprojection (#208) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** One code path for every projection — per-type triggers become registered apply
functions behind a single dispatcher; `cairn_reproject()` heals or rebuilds by replaying the
identical dispatch; the loader heals on generation change; the cost is measured at Bet-B volume.

**Architecture:** Spec: `docs/superpowers/specs/2026-07-20-generic-reprojection-design.md`
(approved 2026-07-20; three amendments discovered during planning are listed below and folded
into the spec doc in a companion commit). New registry `cairn_projection_apply` + dispatcher +
`cairn_replay_eligible` live in `db/005` (both loaders include it); `cairn_reproject`,
`reproject_log`, the `event_type` index, and legacy cleanup live in new `db/039`; every
projection-owning migration file is edited in place to register its rows.

**Tech Stack:** PL/pgSQL migrations (`db/*.sql`), Rust (tokio-postgres) in `crates/cairn-node`
+ `crates/cairn-sync`, SQL test mirrors under `db/tests/`, bash bench script.

## Spec amendments locked in during planning (fold into spec doc, Task 0)

1. **No `'*'` registry rows.** The legibility twin is materialized by the doors (a column on
   `event_log`), not by a trigger — there is no type-independent projection. `db/002`
   (`patient_chart_apply`) and `db/008` (`surrogate_project_apply`) branch internally on exactly
   `patient.created` / `patient.amended` / `note.added`; they register under those exact types.
2. **`heal_safe` per registry row.** `patient_chart.note_count` is a counter: replaying
   `note.added` through its apply increments *again*. Heal-mode replay is valid only for
   idempotent (insert-or-better) applies. Registry rows carry `heal_safe boolean`; heal mode
   skips unsafe rows with a logged notice (honest degradation); rebuild mode handles them
   (counters are correct replayed from truncate). Only `('note.added', patient_chart_apply)` is
   `heal_safe = false` today.
3. **Replay runs in the lenient-apply posture.** The #192 patient-consistency helper and the
   linkage/reconciliation component recomputes RAISE on pathology unless the transaction-local
   GUC `cairn.remote_apply = 'on'` (set by `apply_remote_event`). Replayed events include
   remotely-admitted ones, so `cairn_reproject` does `SET LOCAL cairn.remote_apply = 'on'`:
   events already admitted by a door stay admitted — replay heals, doors refuse (the ADR-0056
   gate-effect-not-presence logic applied to replay).

## Global Constraints

- **AGPL-3.0 only**; no new external dependencies are introduced by this plan.
- **TDD**: write the failing test first, run it to see it fail, then implement.
- **All migrations replay-idempotent**: `connect_and_load_schema` re-runs every `db/*.sql` on
  every connect (memory: no view-widening across files; seed-row edits need `ON CONFLICT DO
  UPDATE`).
- **House rule 6**: never hard-code cryptographic material in tests — derive at runtime.
- **Full-workspace verification**: `cargo test --workspace` (never only `-p cairn-node`; never
  pipe through `tail` — it masks the exit code), plus `scripts/run-db-sql-tests.sh`.
- **Test env**: DB-gated tests need `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb
  dbname=cairn_test"` (PG18 + cairn_pgx ≥ 0.3.0). SQL mirrors:
  `PGHOST=127.0.0.1 PGPORT=5532 scripts/run-db-sql-tests.sh`.
- **Branch**: work continues on `design/208-generic-reprojection` (already created; spec
  committed there).
- **Commit style**: `type(#208): summary`, `Co-Authored-By: Claude Fable 5
  <noreply@anthropic.com>` trailer.

## The registry row inventory (referenced by several tasks)

Every row the finished branch registers, with `heal_safe` and `run_order`. `run_order` mirrors
today's alphabetical trigger-name firing order wherever one event type has several fns
(verified 2026-07-20: no apply fn reads a *sibling* projection's table for the same event, so
the order is hygiene, not correctness).

| registered in | event_type | apply_fn | projection_tables | run_order | heal_safe |
|---|---|---|---|---|---|
| db/005 | patient.created | patient_chart_apply | {patient_chart} | 10 | true |
| db/005 | patient.amended | patient_chart_apply | {patient_chart} | 10 | true |
| db/005 | note.added | patient_chart_apply | {patient_chart} | 10 | **false** |
| db/008 | patient.created | surrogate_project_apply | {patient_ref} | 20 | true |
| db/008 | patient.amended | surrogate_project_apply | {patient_ref} | 20 | true |
| db/008 | note.added | surrogate_project_apply | {patient_ref,chart_note_u,chart_note_s} | 20 | true |
| db/010 | demographic.identifier.asserted | patient_identifier_apply | {patient_identifier} | 10 | true |
| db/011 | demographic.field.asserted | patient_demographic_apply | {patient_demographic} | 20 | true |
| db/012 | demographic.field.asserted | patient_name_apply | {patient_name} | 30 | true |
| db/014 | demographic.field.asserted | patient_address_apply | {patient_address} | 10 | true |
| db/018 | identity.link.asserted | patient_link_apply | {patient_link,person_member,link_veto_flag,identity_projection_flag} | 10 | true |
| db/018 | identity.unlink.asserted | patient_link_apply | (same) | 10 | true |
| db/023 | identity.dispute.asserted | chart_dispute_apply | {chart_dispute} | 10 | true |
| db/023 | identity.dispute.resolved | chart_dispute_apply | {chart_dispute} | 10 | true |
| db/024 | identity.pending.asserted | chart_identity_state_apply | {chart_identity_state} | 10 | true |
| db/024 | identity.identify.asserted | chart_identity_state_apply | {chart_identity_state} | 10 | true |
| db/025 | identity.repudiate.asserted | name_repudiation_apply | {name_repudiation} | 10 | true |
| db/031 | clinical.medication.asserted | medication_statement_apply | {medication_statement,medication_patient_conflict_flag} | 20 | true |
| db/031 | clinical.medication-cessation.asserted | medication_cessation_apply | {medication_cessation} | 10 | true |
| db/032 | clinical.medication.asserted | medication_dose_seed_initial | {medication_dose_event} | 10 | true |
| db/032 | clinical.medication-dose-change.asserted | medication_dose_change_apply | {medication_dose_event} | 10 | true |
| db/032 | clinical.medication-dose-correction.asserted | medication_dose_correction_apply | {medication_dose_correction} | 10 | true |
| db/033 | clinical.medication-reconciliation.asserted | medication_reconciliation_apply | {medication_reconciliation,medication_group_member,medication_projection_flag} | 10 | true |
| db/033 | clinical.medication-separation.asserted | medication_reconciliation_apply | (same) | 10 | true |
| db/034 | clinical.medication-attestation.asserted | medication_attestation_apply | {medication_attestation} | 10 | true |

**Counts** (pinned by guard tests): product loader (no db/008) = **22 rows**; the SQL-test
throwaway DB (loads db/008) = **25 rows**. If execution discovers a projection_tables entry
this table missed (each conversion task re-verifies against the fn body + its helpers), fix the
table here AND the registration row — never ship a knowingly-incomplete table list, it is
rebuild-scope metadata.

**The mechanical conversion recipe** (every conversion task uses it):

```sql
-- BEFORE (pattern):
CREATE OR REPLACE FUNCTION <fn>()
RETURNS trigger LANGUAGE plpgsql AS $$ ... NEW.<col> ... RETURN NULL; END $$;
DROP TRIGGER IF EXISTS <trg> ON event_log;
CREATE TRIGGER <trg> AFTER INSERT ON event_log
    FOR EACH ROW WHEN (NEW.event_type = '<type>') EXECUTE FUNCTION <fn>();

-- AFTER:
-- Old trigger-signature pair: dropped explicitly. CREATE OR REPLACE cannot change a
-- signature, so without these DROPs an upgraded-in-place DB keeps BOTH the zero-arg
-- trigger fn and its trigger — double-firing every projection (ADR-0057).
DROP TRIGGER IF EXISTS <trg> ON event_log;
DROP FUNCTION IF EXISTS <fn>();
CREATE OR REPLACE FUNCTION <fn>(e event_log)
RETURNS void LANGUAGE plpgsql AS $$ ... e.<col> ... END $$;   -- body: NEW→e, RETURN NULL→RETURN (or drop)
-- A trigger fn could never be called directly; a plain fn gets PUBLIC EXECUTE by
-- default. Same discipline as every privileged fn in db/005 (Task-1 review finding).
REVOKE EXECUTE ON FUNCTION <fn>(event_log) FROM PUBLIC;
INSERT INTO cairn_projection_apply AS r (event_type, apply_fn, projection_tables, run_order, heal_safe)
VALUES ('<type>', '<fn>', ARRAY['<tbl>', ...], <n>, true)
ON CONFLICT (event_type, apply_fn) DO UPDATE SET
    projection_tables = EXCLUDED.projection_tables,
    run_order         = EXCLUDED.run_order,
    heal_safe         = EXCLUDED.heal_safe
-- #214 + steady-state discipline (copy the exact WHERE idiom from db/031's twin-check
-- registration): converge rows to the migration text, but keep the every-connect
-- replay WRITE-FREE when nothing changed (no dead tuples, no validate-trigger fire).
WHERE (r.projection_tables, r.run_order, r.heal_safe)
      IS DISTINCT FROM (EXCLUDED.projection_tables, EXCLUDED.run_order, EXCLUDED.heal_safe);
```

Files that *redefine* an earlier file's fn (`db/013` → `patient_demographic_apply`,
`db/035` → `medication_dose_correction_apply`) only convert the fn body signature — the DROPs
and the registration row live in the file that first defines it. Every plain `INSERT` inside a
converted fn (or a helper it calls) must be replay-idempotent: if it lacks both a conflict
target and an `ON CONFLICT` clause, add `ON CONFLICT DO NOTHING` on the row's natural key
(adding a unique index if none exists — additive, replay-safe). Known audit points:
`identity_projection_flag`, `link_veto_flag` (db/018), `medication_projection_flag` (db/033),
`medication_patient_conflict_flag` (db/031), `chart_note_u`/`chart_note_s` (db/008).

---

### Task 0: Fold the planning amendments into the spec doc

**Files:**
- Modify: `docs/superpowers/specs/2026-07-20-generic-reprojection-design.md`

**Interfaces:** none (docs only).

- [ ] **Step 1: Edit the spec doc**

In §3/D1: replace the `'*'`-row paragraph with the exact-type registration fact (amendment 1)
and add the `heal_safe` column to the registry table description (amendment 2). In §3/D2: add
the lenient-posture sentence (amendment 3) and note heal-mode's skip semantics. In §2/F-list:
correct "the db/002 twin materialization" — db/002 is the `patient_chart` projection; the twin
is door-materialized on `event_log`.

- [ ] **Step 2: Commit**

```bash
git add docs/superpowers/specs/2026-07-20-generic-reprojection-design.md
git commit -m "docs(#208): spec amendments from planning — exact-type rows, heal_safe, lenient replay posture"
```

---

### Task 1: Registry + dispatcher + eligibility in db/005; convert db/002

**Files:**
- Modify: `db/005_submit.sql` (new objects after the `cairn_event_twin_check` block, ~line 66)
- Modify: `db/002_projection.sql:74-152` (convert `patient_chart_apply`)
- Test: `crates/cairn-node/tests/projection_registry.rs` (new)

**Interfaces:**
- Produces: table `cairn_projection_apply(event_type text, apply_fn text, projection_tables
  text[], run_order int, heal_safe boolean, PRIMARY KEY (event_type, apply_fn))`; function
  `cairn_replay_eligible(e event_log) RETURNS boolean`; function
  `cairn_projection_dispatch() RETURNS trigger` + trigger `cairn_projection_dispatch_trg`
  (AFTER INSERT ON event_log FOR EACH ROW); `patient_chart_apply(e event_log) RETURNS void`.
- Consumed by: every later task.

- [ ] **Step 1: Write the failing tests**

`crates/cairn-node/tests/projection_registry.rs` (mirror the header idiom of
`crates/cairn-node/tests/twin_registry.rs` — `cs()`, `db_msg()`, `test_serial_guard`):

```rust
//! #208/ADR-0057 — the cairn_projection_apply registry: registration is the wiring.
//! DB-gated on $CAIRN_TEST_PG, serialized via db::test_serial_guard.
use cairn_node::db;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

fn db_msg(e: &tokio_postgres::Error) -> String {
    e.as_db_error()
        .map(|d| d.message().to_string())
        .unwrap_or_else(|| e.to_string())
}

/// A registry row naming an apply fn that does not exist with the (event_log)
/// signature is refused at INSERT time — fail closed at registration, like the
/// twin-check registry (ADR-0048).
#[tokio::test]
async fn registry_rejects_missing_apply_fn() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.execute(
        "DELETE FROM cairn_projection_apply WHERE event_type LIKE 'test.%'",
        &[],
    )
    .await
    .unwrap();
    let err = c
        .execute(
            "INSERT INTO cairn_projection_apply \
             (event_type, apply_fn, projection_tables, run_order, heal_safe) \
             VALUES ('test.bogus', 'no_such_apply_fn', ARRAY['patient_chart'], 10, true)",
            &[],
        )
        .await
        .expect_err("bogus apply_fn must be rejected");
    assert!(db_msg(&err).contains("does not exist"), "got: {}", db_msg(&err));
}

/// A registered projection_tables entry naming a table that does not exist is
/// refused too — the list is rebuild-scope metadata; a typo would silently
/// exempt the real table from rebuild's shared-table refusal.
#[tokio::test]
async fn registry_rejects_missing_projection_table() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let err = c
        .execute(
            "INSERT INTO cairn_projection_apply \
             (event_type, apply_fn, projection_tables, run_order, heal_safe) \
             VALUES ('test.bogus2', 'patient_chart_apply', ARRAY['no_such_table'], 10, true)",
            &[],
        )
        .await
        .expect_err("bogus projection table must be rejected");
    assert!(db_msg(&err).contains("does not exist"), "got: {}", db_msg(&err));
}

/// The dispatcher replaces db/002's per-type trigger: a directly-inserted
/// patient.created event still materializes its patient_chart row.
#[tokio::test]
async fn dispatcher_routes_patient_created_to_patient_chart() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let pid = uuid::Uuid::now_v7();
    // Owner-level direct INSERT: the projection trigger path is what's under test,
    // not the door. signed_bytes is synthetic; the content-address CHECK is
    // satisfied by computing the digest in SQL (house rule 6: derived, not literal).
    c.execute(
        "WITH sb AS (SELECT ('reproj-test-' || $1::text)::bytea AS b)
         INSERT INTO event_log (event_id, patient_id, event_type, schema_version,
             hlc_wall, hlc_counter, node_origin, signed_bytes, content_address,
             body, contributors, signer_key_id, plaintext_twin)
         SELECT $1, $1, 'patient.created', 'test-1',
             (extract(epoch from now()) * 1000)::bigint, 0, 'test-node', b,
             '\\x1220'::bytea || digest(b, 'sha256'),
             jsonb_build_object('name', 'Reproject Probe'),
             '[]'::jsonb, 'test-key', 'probe'
         FROM sb",
        &[&pid],
    )
    .await
    .unwrap();
    let row = c
        .query_one(
            "SELECT name FROM patient_chart WHERE patient_id = $1",
            &[&pid],
        )
        .await
        .unwrap();
    let name: String = row.get(0);
    assert_eq!(name, "Reproject Probe");
    // No cleanup: event_log is append-only (BEFORE UPDATE/DELETE guard) — the
    // probe event stays, which is fine on the shared test DB (fresh UUID each run).
}
```

Note: if `uuid` is not already a dev-dependency of cairn-node, reuse however existing tests
mint UUIDs (grep `now_v7\|uuidv7` in `crates/cairn-node/tests/` and copy that idiom, e.g.
selecting `uuidv7()` from the DB) rather than adding a dependency.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p cairn-node --test projection_registry`
Expected: FAIL — `relation "cairn_projection_apply" does not exist`.

- [ ] **Step 3: Implement db/005 additions**

Insert after the `cairn_event_twin_check` block (after its REVOKE, ~line 66), before
`cairn_event_twin`:

```sql
-- ---------------------------------------------------------------------------
-- The projection registry (#208 / ADR-0057): registration IS the wiring.
-- A projection lives only in its registered apply function; ONE dispatcher
-- trigger (below) replaces every per-type projection trigger, and
-- cairn_reproject (db/039) heals/rebuilds by replaying the IDENTICAL dispatch.
-- Same discipline as cairn_event_twin_check above (ADR-0048): register-by-row
-- in the migration that defines the fn, fail closed at registration time.
--
-- heal_safe: TRUE iff replaying an event through this fn over an EXISTING
-- projection converges (insert-or-better winner logic). A counter-shaped
-- projection (patient_chart.note_count) is NOT: replay would increment again.
-- Heal-mode reproject skips heal_safe=false rows with a notice; rebuild mode
-- (truncate-then-replay) handles them. New projections should be idempotent;
-- heal_safe=false needs a comment justifying the shape.
CREATE TABLE IF NOT EXISTS cairn_projection_apply (
    event_type        TEXT    NOT NULL,
    apply_fn          TEXT    NOT NULL,
    projection_tables TEXT[]  NOT NULL,
    run_order         INTEGER NOT NULL DEFAULT 100,
    heal_safe         BOOLEAN NOT NULL DEFAULT TRUE,
    PRIMARY KEY (event_type, apply_fn)
);

-- Fail closed at REGISTRATION time, exactly like cairn_check_twin_registry_fn:
-- the apply fn must exist with the unified (event_log) signature, and every
-- projection_tables entry must be a real relation (it is rebuild-scope metadata
-- — a typo would silently exempt the real table from rebuild's refusal check).
CREATE OR REPLACE FUNCTION cairn_check_projection_registry_fn()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE v_tbl text;
BEGIN
    IF to_regprocedure(NEW.apply_fn || '(event_log)') IS NULL THEN
        RAISE EXCEPTION
            'cairn_projection_apply: apply_fn %(event_log) does not exist (fail closed)',
            NEW.apply_fn;
    END IF;
    FOREACH v_tbl IN ARRAY NEW.projection_tables LOOP
        IF to_regclass(v_tbl) IS NULL THEN
            RAISE EXCEPTION
                'cairn_projection_apply: projection table "%" does not exist (fail closed)',
                v_tbl;
        END IF;
    END LOOP;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS cairn_projection_apply_validate ON cairn_projection_apply;
CREATE TRIGGER cairn_projection_apply_validate
    BEFORE INSERT OR UPDATE ON cairn_projection_apply
    FOR EACH ROW EXECUTE FUNCTION cairn_check_projection_registry_fn();

-- Safety surface: a row pointing a type's projection at a no-op would silently
-- stop materialization. Locked down like cairn_event_twin_check.
REVOKE INSERT, UPDATE, DELETE ON cairn_projection_apply FROM PUBLIC;

-- The #266 safety seam (ADR-0056 decision 4): cairn_reproject routes every
-- candidate event through this predicate. Constantly TRUE today — no deferred
-- events can exist while the remote door still fail-closes on unknown types.
-- #265's explicit deferred marker hooks in HERE and only here, so a manual
-- mid-upgrade reproject can never grant power to an unadjudicated deferred
-- event. The live-insert path needs no filter: an event being inserted through
-- a door was adjudicated by that door.
CREATE OR REPLACE FUNCTION cairn_replay_eligible(e event_log)
RETURNS boolean LANGUAGE sql STABLE AS $$ SELECT TRUE $$;

-- The ONE projection trigger: look up the registered apply fns for this event's
-- type and run each. Deterministic order (run_order, then name — mirrors the
-- old alphabetical trigger-name firing order). Types with no registered rows
-- (e.g. carried-not-projected federation types, ADR-0012) dispatch nothing —
-- the same behavior the old WHEN-filtered triggers gave them.
CREATE OR REPLACE FUNCTION cairn_projection_dispatch()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE r record;
BEGIN
    FOR r IN
        SELECT apply_fn FROM cairn_projection_apply
        WHERE event_type = NEW.event_type
        ORDER BY run_order, apply_fn
    LOOP
        EXECUTE format('SELECT %I($1)', r.apply_fn) USING NEW;
    END LOOP;
    RETURN NULL;
END;
$$;

DROP TRIGGER IF EXISTS cairn_projection_dispatch_trg ON event_log;
CREATE TRIGGER cairn_projection_dispatch_trg
    AFTER INSERT ON event_log
    FOR EACH ROW EXECUTE FUNCTION cairn_projection_dispatch();
```

Then at the END of db/005 (after the existing `cairn_event_twin_check` seed INSERT at ~line
209), register db/002's rows (they live here, not in db/002, because the registry table must
exist first — load order):

```sql
-- db/002's patient_chart projection rows (registered here: db/002 loads before
-- this registry exists). note.added is heal_safe=false BY SHAPE: note_count is
-- a counter — replaying an already-counted event would increment again. It
-- heals only via rebuild (truncate-then-replay). See ADR-0057.
INSERT INTO cairn_projection_apply (event_type, apply_fn, projection_tables, run_order, heal_safe) VALUES
    ('patient.created', 'patient_chart_apply', ARRAY['patient_chart'], 10, TRUE),
    ('patient.amended', 'patient_chart_apply', ARRAY['patient_chart'], 10, TRUE),
    ('note.added',      'patient_chart_apply', ARRAY['patient_chart'], 10, FALSE)
ON CONFLICT (event_type, apply_fn) DO UPDATE SET
    projection_tables = EXCLUDED.projection_tables,
    run_order         = EXCLUDED.run_order,
    heal_safe         = EXCLUDED.heal_safe;
```

- [ ] **Step 4: Convert db/002's trigger to an apply fn**

In `db/002_projection.sql`, apply the mechanical recipe to `patient_chart_apply`
(lines 74–152): add the two DROPs, change the signature to `(e event_log) RETURNS void`,
replace every `NEW.` with `e.`, replace both `RETURN NULL;` with `RETURN;`, and delete the
`CREATE TRIGGER event_log_project` statement, keeping:

```sql
-- The per-type trigger is superseded by cairn_projection_dispatch_trg (db/005,
-- ADR-0057); this fn is now registered in cairn_projection_apply there.
DROP TRIGGER IF EXISTS event_log_project ON event_log;
```

- [ ] **Step 5: Run the new tests + the full suite**

Run: `cargo test -p cairn-node --test projection_registry` → PASS (3 tests).
Run: `cargo test --workspace` → PASS (the existing suites passing unchanged is the
dispatcher-equivalence guard).

- [ ] **Step 6: Commit**

```bash
git add db/005_submit.sql db/002_projection.sql crates/cairn-node/tests/projection_registry.rs
git commit -m "feat(#208): projection registry + single dispatcher; convert db/002 (ADR-0057)"
```

---

### Task 2: db/039 — cairn_reproject, reproject_log, event_type index

**Files:**
- Create: `db/039_projection_registry.sql`
- Modify: `crates/cairn-event/src/schema_generation.rs:44` (`SCHEMA_GENERATION: i32 = 39`)
- Modify: `crates/cairn-node/src/db.rs` (append `("039_projection_registry", include_str!(...))`
  to `SCHEMA` after the 038 entry)
- Modify: `crates/cairn-sync/src/main.rs` (same append to its `SCHEMA` after the 038 entry)
- Test: `crates/cairn-node/tests/projection_registry.rs` (extend)

**Interfaces:**
- Consumes: `cairn_projection_apply`, `cairn_replay_eligible(event_log)`, dispatch order
  `(run_order, apply_fn)` — all from Task 1.
- Produces: `cairn_reproject(p_prefix text DEFAULT '', p_rebuild boolean DEFAULT false,
  p_source text DEFAULT 'manual') RETURNS TABLE(event_type text, events_replayed bigint)`;
  table `reproject_log(id, ran_at, prefix, rebuild, source, events_seen, elapsed_ms,
  skipped_fns text[])`; index `event_log_type_idx`. Tasks 3/4/8 and the loaders call these.

- [ ] **Step 1: Write the failing tests** (append to `projection_registry.rs`)

```rust
/// Heal mode converges a tampered winner row back to the corrected winner —
/// the generic replacement for the per-slice tamper-then-replay heal tests
/// (#214 pattern). Uses the Task-1 probe shape: tamper patient_chart.name.
#[tokio::test]
async fn reproject_heals_tampered_projection_row() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let pid: uuid::Uuid = c
        .query_one("SELECT uuidv7()", &[])
        .await
        .unwrap()
        .get(0);
    c.execute(/* same probe INSERT as dispatcher_routes_patient_created_to_patient_chart */
        "WITH sb AS (SELECT ('heal-test-' || $1::text)::bytea AS b)
         INSERT INTO event_log (event_id, patient_id, event_type, schema_version,
             hlc_wall, hlc_counter, node_origin, signed_bytes, content_address,
             body, contributors, signer_key_id, plaintext_twin)
         SELECT $1, $1, 'patient.created', 'test-1',
             (extract(epoch from now()) * 1000)::bigint, 0, 'test-node', b,
             '\\x1220'::bytea || digest(b, 'sha256'),
             jsonb_build_object('name', 'True Winner'),
             '[]'::jsonb, 'test-key', 'probe'
         FROM sb",
        &[&pid],
    )
    .await
    .unwrap();
    c.execute(
        "UPDATE patient_chart SET name = 'TAMPERED' WHERE patient_id = $1",
        &[&pid],
    )
    .await
    .unwrap();
    c.query(
        "SELECT * FROM cairn_reproject('patient.', false, 'test')",
        &[],
    )
    .await
    .unwrap();
    let name: String = c
        .query_one("SELECT name FROM patient_chart WHERE patient_id = $1", &[&pid])
        .await
        .unwrap()
        .get(0);
    assert_eq!(name, "True Winner", "heal replay must converge the tampered row");
    // The run is recorded (the operational observable the loader-gating test reuses).
    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM reproject_log WHERE source = 'test' AND prefix = 'patient.'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert!(n >= 1);
}

/// Heal mode SKIPS heal_safe=false rows (note_count is a counter): the skipped
/// fn is reported, and note_count is NOT double-incremented.
#[tokio::test]
async fn heal_skips_counter_shaped_rows() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let pid: uuid::Uuid = c.query_one("SELECT uuidv7()", &[]).await.unwrap().get(0);
    for (i, ty) in ["patient.created", "note.added"].iter().enumerate() {
        c.execute(
            "WITH sb AS (SELECT ('skip-test-' || $2::text || $1::text)::bytea AS b)
             INSERT INTO event_log (event_id, patient_id, event_type, schema_version,
                 hlc_wall, hlc_counter, node_origin, signed_bytes, content_address,
                 body, contributors, signer_key_id, plaintext_twin)
             SELECT uuidv7(), $1, $2, 'test-1',
                 (extract(epoch from now()) * 1000)::bigint + $3, 0, 'test-node', b,
                 '\\x1220'::bytea || digest(b, 'sha256'),
                 jsonb_build_object('name', 'Skip Probe'),
                 '[]'::jsonb, 'test-key', 'probe'
             FROM sb",
            &[&pid, &ty, &(i as i64)],
        )
        .await
        .unwrap();
    }
    let before: i32 = c
        .query_one("SELECT note_count FROM patient_chart WHERE patient_id = $1", &[&pid])
        .await
        .unwrap()
        .get(0);
    c.query("SELECT * FROM cairn_reproject('note.', false, 'test')", &[])
        .await
        .unwrap();
    let after: i32 = c
        .query_one("SELECT note_count FROM patient_chart WHERE patient_id = $1", &[&pid])
        .await
        .unwrap()
        .get(0);
    assert_eq!(before, after, "heal must not re-increment a counter projection");
    let skipped: Vec<String> = c
        .query_one(
            "SELECT skipped_fns FROM reproject_log WHERE source='test' AND prefix='note.' \
             ORDER BY id DESC LIMIT 1",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert!(skipped.contains(&"note.added:patient_chart_apply".to_string()));
}

/// Rebuild refuses when the prefix would truncate a table also fed by
/// out-of-prefix types; a full-scope rebuild succeeds and recounts correctly.
#[tokio::test]
async fn rebuild_scope_refusal_and_full_rebuild() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let err = c
        .query("SELECT * FROM cairn_reproject('note.', true, 'test')", &[])
        .await
        .expect_err("narrow rebuild over shared patient_chart must refuse");
    assert!(
        db_msg(&err).contains("also fed by event types outside"),
        "got: {}",
        db_msg(&err)
    );
    // Full-scope rebuild is allowed and heals counters (truncate → replay from zero).
    c.query("SELECT * FROM cairn_reproject('', true, 'test')", &[])
        .await
        .expect("full-scope rebuild must be permitted");
}

/// The #266 seam: an event cairn_replay_eligible rejects is untouched by replay.
#[tokio::test]
async fn replay_respects_eligibility_seam() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let pid: uuid::Uuid = c.query_one("SELECT uuidv7()", &[]).await.unwrap().get(0);
    c.execute(
        "WITH sb AS (SELECT ('elig-test-' || $1::text)::bytea AS b)
         INSERT INTO event_log (event_id, patient_id, event_type, schema_version,
             hlc_wall, hlc_counter, node_origin, signed_bytes, content_address,
             body, contributors, signer_key_id, plaintext_twin)
         SELECT $1, $1, 'patient.created', 'test-1',
             (extract(epoch from now()) * 1000)::bigint, 0, 'test-node', b,
             '\\x1220'::bytea || digest(b, 'sha256'),
             jsonb_build_object('name', 'Eligible Winner'),
             '[]'::jsonb, 'test-key', 'probe'
         FROM sb",
        &[&pid],
    )
    .await
    .unwrap();
    c.execute("UPDATE patient_chart SET name = 'STALE' WHERE patient_id = $1", &[&pid])
        .await
        .unwrap();
    // Stub the seam to reject everything, replay, then restore by reloading schema.
    c.batch_execute(
        "CREATE OR REPLACE FUNCTION cairn_replay_eligible(e event_log) \
         RETURNS boolean LANGUAGE sql STABLE AS $$ SELECT FALSE $$;",
    )
    .await
    .unwrap();
    c.query("SELECT * FROM cairn_reproject('patient.', false, 'test')", &[])
        .await
        .unwrap();
    let name: String = c
        .query_one("SELECT name FROM patient_chart WHERE patient_id = $1", &[&pid])
        .await
        .unwrap()
        .get(0);
    assert_eq!(name, "STALE", "an ineligible event must confer no projection effect");
    // Restore the real predicate (schema replay redefines it).
    drop(c);
    let c2 = db::connect_and_load_schema(&base).await.unwrap();
    c2.query("SELECT * FROM cairn_reproject('patient.', false, 'test')", &[])
        .await
        .unwrap();
    let healed: String = c2
        .query_one("SELECT name FROM patient_chart WHERE patient_id = $1", &[&pid])
        .await
        .unwrap()
        .get(0);
    assert_eq!(healed, "Eligible Winner");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p cairn-node --test projection_registry`
Expected: the four new tests FAIL — `function cairn_reproject(unknown, boolean, unknown) does
not exist`.

- [ ] **Step 3: Create `db/039_projection_registry.sql`**

```sql
-- db/039_projection_registry.sql
-- Cairn — generic reprojection (#208 / ADR-0057): the heal/rebuild entry point.
--
-- The registry + the ONE dispatcher trigger live in db/005 (both loaders carry
-- it). This file adds what only the FULL story needs: the replay entry point,
-- its operational log, the event_type index the replay scan (and #266's
-- reclassification scans) use, and the retirement of the legacy bespoke
-- demographic backfill this mechanism subsumes.
--
-- connect_and_load_schema re-runs every migration each connect: everything
-- below is idempotent.

BEGIN;

-- Prefix-scoped replay + exact-type scans. text_pattern_ops: LIKE 'p%' must
-- use the index regardless of database collation.
CREATE INDEX IF NOT EXISTS event_log_type_idx
    ON event_log (event_type text_pattern_ops);

-- Node-LOCAL operational record of every reproject run (like node_schema:
-- never signed, never on the wire — principle 12). The loader-gating tests and
-- `cairn-node status`-style surfaces read it; the bench reads elapsed_ms.
CREATE TABLE IF NOT EXISTS reproject_log (
    id          BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    ran_at      TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    prefix      TEXT        NOT NULL,
    rebuild     BOOLEAN     NOT NULL,
    source      TEXT        NOT NULL,   -- 'loader' | 'cli' | 'test' | 'manual'
    events_seen BIGINT      NOT NULL,
    elapsed_ms  BIGINT      NOT NULL,
    -- heal-mode rows skipped as heal_safe=false, as 'event_type:apply_fn'.
    -- Honest degradation is only honest if it is VISIBLE (spec §3.13 spirit).
    skipped_fns TEXT[]      NOT NULL DEFAULT '{}'
);

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'cairn_node') THEN
        CREATE ROLE cairn_node NOLOGIN;
    END IF;
END $$;
GRANT SELECT ON reproject_log TO cairn_node;

-- The generic heal/rebuild replay (ADR-0057 decision D2).
--
--  * HEAL (default): no deletes. Every apply is insert-or-better, and every
--    projection is arrival-order-independent (set-union sync requires it), so
--    replaying the log through the SAME registered fns the live dispatcher
--    uses converges every winner row — a stale winner is itself some event's
--    tuple, dominated by the true maximum under the corrected comparison.
--    heal_safe=false rows (counter shapes) are SKIPPED and reported.
--  * REBUILD (wrote-garbage defects): TRUNCATE each in-scope projection table
--    first — refusing if any table is also fed by an out-of-prefix type, so a
--    narrow rebuild can never silently wipe another type's rows.
--
-- Runs in the LENIENT apply posture (cairn.remote_apply = on, transaction-
-- local): replayed events include remotely-admitted ones, and the clamp-vs-
-- raise helpers (#192 patient consistency, the component-size caps) must
-- clamp-and-flag here exactly as they did at admission. Events already
-- admitted by a door stay admitted: replay heals, doors refuse (ADR-0056's
-- gate-effect-not-presence, applied to replay).
--
-- Owner-only: it can TRUNCATE projections. The loader and the `cairn-node
-- reproject` CLI connect with owner privileges; the runtime role cannot call it.
CREATE OR REPLACE FUNCTION cairn_reproject(
    p_prefix  text    DEFAULT '',
    p_rebuild boolean DEFAULT false,
    p_source  text    DEFAULT 'manual'
) RETURNS TABLE(event_type text, events_replayed bigint)
LANGUAGE plpgsql
-- Pinned like cairn_event_twin's dynamic dispatch (Task-1 review): the %I EXECUTE
-- below must never resolve into an attacker-shadowed schema, regardless of caller.
SET search_path = public
AS $$
DECLARE
    v_started timestamptz := clock_timestamp();
    v_tbl     text;
    v_type    text;
    v_fns     text[];
    v_fn      text;
    v_e       event_log;
    v_n       bigint;
    v_total   bigint := 0;
    v_skipped text[] := '{}';
    v_skip    text[];
BEGIN
    PERFORM set_config('cairn.remote_apply', 'on', true);  -- true = SET LOCAL

    IF p_rebuild THEN
        FOR v_tbl IN
            SELECT DISTINCT unnest(r.projection_tables)
            FROM cairn_projection_apply r
            WHERE r.event_type LIKE p_prefix || '%'
        LOOP
            IF EXISTS (
                SELECT 1 FROM cairn_projection_apply r
                WHERE v_tbl = ANY (r.projection_tables)
                  AND r.event_type NOT LIKE p_prefix || '%'
            ) THEN
                RAISE EXCEPTION
                    'cairn_reproject: rebuilding prefix "%" would truncate projection table "%", which is also fed by event types outside that prefix. Widen the prefix or use heal mode.',
                    p_prefix, v_tbl;
            END IF;
            EXECUTE format('TRUNCATE %I', v_tbl);
        END LOOP;
    END IF;

    FOR v_type, v_fns, v_skip IN
        SELECT r.event_type,
               array_agg(r.apply_fn ORDER BY r.run_order, r.apply_fn)
                   FILTER (WHERE p_rebuild OR r.heal_safe),
               array_agg(r.event_type || ':' || r.apply_fn ORDER BY r.apply_fn)
                   FILTER (WHERE NOT (p_rebuild OR r.heal_safe))
        FROM cairn_projection_apply r
        WHERE r.event_type LIKE p_prefix || '%'
        GROUP BY r.event_type
        ORDER BY r.event_type
    LOOP
        v_skipped := v_skipped || COALESCE(v_skip, '{}');
        IF COALESCE(v_skip, '{}') <> '{}' THEN
            RAISE NOTICE 'cairn_reproject: heal mode skipping non-heal-safe %', v_skip;
        END IF;
        v_n := 0;
        IF v_fns IS NOT NULL THEN
            FOR v_e IN
                SELECT el.* FROM event_log el
                WHERE el.event_type = v_type AND cairn_replay_eligible(el)
                ORDER BY el.hlc_wall, el.hlc_counter, el.node_origin
            LOOP
                FOREACH v_fn IN ARRAY v_fns LOOP
                    EXECUTE format('SELECT %I($1)', v_fn) USING v_e;
                END LOOP;
                v_n := v_n + 1;
            END LOOP;
        END IF;
        event_type      := v_type;
        events_replayed := v_n;
        v_total         := v_total + v_n;
        RETURN NEXT;
    END LOOP;

    INSERT INTO reproject_log (prefix, rebuild, source, events_seen, elapsed_ms, skipped_fns)
    VALUES (p_prefix, p_rebuild, p_source, v_total,
            (extract(epoch FROM clock_timestamp() - v_started) * 1000)::bigint,
            v_skipped);
END;
$$;

REVOKE EXECUTE ON FUNCTION cairn_reproject(text, boolean, text) FROM PUBLIC;

COMMIT;
```

(Note: all plpgsql references inside the fn body are table-qualified (`r.`, `el.`) because the
OUT parameter is named `event_type` — an unqualified reference would bind the variable.)

- [ ] **Step 4: Wire the file into both loaders + bump the generation**

`crates/cairn-event/src/schema_generation.rs`: `pub const SCHEMA_GENERATION: i32 = 39;`
(update the doc-comment example numbers alongside). `crates/cairn-node/src/db.rs` and
`crates/cairn-sync/src/main.rs`: append after their 038 entries:

```rust
    // #208/ADR-0057: cairn_reproject + reproject_log + the event_type index. In
    // BOTH lists: each loader's gated heal step (generation change) calls it.
    (
        "039_projection_registry",
        include_str!("../../../db/039_projection_registry.sql"),
    ),
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p cairn-node --test projection_registry` → PASS.
Run: `cargo test --workspace` → PASS (cairn-event's fs-derived generation guard and
cairn-sync's subset-shape tests must both agree with 39).

Note: db/013 (unconverted until Task 4) still defines and calls
`cairn_demographic_backfill()` on every replay — deliberately untouched here so the two
existing tests that call it keep passing mid-branch. Its retirement (definition + call deleted,
a `DROP FUNCTION IF EXISTS` tombstone left in db/013 so upgraded-in-place databases shed the
fn) happens atomically with the demographics conversion in Task 4. Do NOT put the DROP in
db/039: db/013 loads *before* db/039 in every replay, so a 039-side DROP would win every
replay and break the legacy-calling tests before Task 4 lands.

- [ ] **Step 6: Commit**

```bash
git add db/039_projection_registry.sql crates/cairn-event/src/schema_generation.rs \
    crates/cairn-node/src/db.rs crates/cairn-sync/src/main.rs \
    crates/cairn-node/tests/projection_registry.rs
git commit -m "feat(#208): cairn_reproject heal/rebuild + reproject_log + event_type index (db/039)"
```

---

### Task 3: Loader gating (both loaders) + `cairn-node reproject` CLI

**Files:**
- Modify: `crates/cairn-node/src/db.rs` (`connect_and_load_schema`, after the stamp, before the
  unlock, ~line 380)
- Modify: `crates/cairn-sync/src/main.rs` (`load_schema_under_lock`, after its stamp)
- Modify: `crates/cairn-node/src/main.rs` (`Cmd` enum ~line 360; `match cli.cmd` ~line 855)
- Test: `crates/cairn-node/tests/projection_registry.rs` (extend)

**Interfaces:**
- Consumes: `cairn_reproject(text, boolean, text)`, `reproject_log`, the `recorded` /
  `embedded` generation values already computed in each loader.
- Produces: CLI `cairn-node reproject [--prefix P] [--rebuild]`; loader behavior "heal replay
  iff generation changed".

- [ ] **Step 1: Write the failing test** (append to `projection_registry.rs`)

```rust
/// The loader runs a full heal replay ONLY when the recorded generation differs
/// from the embedded one — never on an ordinary same-generation reconnect.
/// This is the #208 headline fix: the old db/013 backfill ran on EVERY connect.
#[tokio::test]
async fn loader_heals_on_generation_change_only() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    // Connect once so the DB is at the embedded generation.
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let n0: i64 = c
        .query_one("SELECT count(*) FROM reproject_log WHERE source = 'loader'", &[])
        .await
        .unwrap()
        .get(0);
    drop(c);
    // Same-generation reconnect: NO new loader-sourced run.
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let n1: i64 = c
        .query_one("SELECT count(*) FROM reproject_log WHERE source = 'loader'", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(n0, n1, "same-generation reconnect must not reproject");
    // Simulate an upgrade: knock the recorded generation back one.
    c.execute("UPDATE node_schema SET version = version - 1", &[])
        .await
        .unwrap();
    drop(c);
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let n2: i64 = c
        .query_one("SELECT count(*) FROM reproject_log WHERE source = 'loader'", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(n2, n1 + 1, "generation change must trigger exactly one heal replay");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p cairn-node --test projection_registry loader_heals` — FAILS: no
loader-sourced rows exist at all (`n2 == n1`).

- [ ] **Step 3: Implement the gated heal in both loaders**

`crates/cairn-node/src/db.rs`, in `connect_and_load_schema` immediately after the
`node_schema` stamp `execute` and before the `pg_advisory_unlock`:

```rust
    // #208/ADR-0057: heal replay on generation CHANGE only. New projection
    // capability (and any projection-logic fix) arrives only via a code-plane
    // update — i.e. a generation change — so an unchanged generation means
    // there is nothing to heal and the connect path does zero reprojection
    // work (the old db/013 every-connect backfill is retired). An UNKNOWN
    // recorded generation (fresh DB: free no-op; hand-built rig: converges
    // once) errs toward healing. Runs inside SCHEMA_LOAD_LOCK: concurrent
    // loaders serialize, and the second sees the stamped generation.
    if recorded != Some(embedded) {
        client
            .execute("SELECT count(*) FROM cairn_reproject('', false, 'loader')", &[])
            .await
            .map_err(|e| anyhow::anyhow!("post-upgrade heal replay: {e}"))?;
    }
```

`crates/cairn-sync/src/main.rs`, in `load_schema_under_lock`: the `recorded` value is
currently read inside an `if let` — hoist it into a local
(`let mut recorded: Option<i32> = None;` set inside the existing check) so the same
condition can run after its stamp:

```rust
    // #208/ADR-0057: same gated heal as cairn-node's loader. On this SUBSET
    // database only the subset-registered projections exist (db/002's rows);
    // the registry makes that automatic — replay heals exactly what is
    // registered here, nothing more.
    if recorded != Some(embedded) {
        client.execute("SELECT count(*) FROM cairn_reproject('', false, 'loader')", &[])?;
    }
```

- [ ] **Step 4: Add the CLI subcommand**

`crates/cairn-node/src/main.rs` — in `enum Cmd`:

```rust
    /// Replay event_log through the registered projection apply fns (#208 /
    /// ADR-0057): heal a projection after a logic fix, or rebuild after a
    /// wrote-garbage defect. Needs an owner-privileged --conn (like `init`) —
    /// the runtime role deliberately cannot execute it.
    Reproject {
        /// Event-type prefix to replay ('' = everything).
        #[arg(long, default_value = "")]
        prefix: String,
        /// TRUNCATE the in-scope projection tables first (refuses if a table
        /// is also fed by out-of-prefix types). Default is heal (no deletes).
        #[arg(long)]
        rebuild: bool,
    },
```

In the `match cli.cmd`:

```rust
        Cmd::Reproject { prefix, rebuild } => {
            let db = cairn_node::db::connect_and_load_schema(&cli.conn).await?;
            let rows = db
                .query(
                    "SELECT event_type, events_replayed FROM cairn_reproject($1, $2, 'cli')",
                    &[&prefix, &rebuild],
                )
                .await?;
            let mut total: i64 = 0;
            for r in &rows {
                let ty: String = r.get(0);
                let n: i64 = r.get(1);
                total += n;
                println!("{ty:<55} {n:>10}");
            }
            let log = db
                .query_one(
                    "SELECT elapsed_ms, skipped_fns FROM reproject_log ORDER BY id DESC LIMIT 1",
                    &[],
                )
                .await?;
            let ms: i64 = log.get(0);
            let skipped: Vec<String> = log.get(1);
            println!("replayed {total} events in {ms} ms");
            if !skipped.is_empty() {
                println!("skipped (heal_safe = false — rebuild to heal these): {skipped:?}");
            }
        }
```

- [ ] **Step 5: Run tests**

Run: `cargo test --workspace` → PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/cairn-node/src/db.rs crates/cairn-sync/src/main.rs \
    crates/cairn-node/src/main.rs crates/cairn-node/tests/projection_registry.rs
git commit -m "feat(#208): generation-gated loader heal + cairn-node reproject CLI"
```

---

### Task 4: Convert demographics (db/010–014); retire the bespoke backfill

**Files:**
- Modify: `db/010_demographics.sql` (fn ~105–163), `db/011_demographics_fields.sql` (fn
  135–200), `db/012_demographics_names.sql` (fn 40–102), `db/013_demographics_sex_gender.sql`
  (fn 39–117 + DELETE the `cairn_demographic_backfill` fn at 134–201 and the
  `SELECT cairn_demographic_backfill();` call at 204), `db/014_demographics_address.sql` (fn
  141–197)
- Modify: `crates/cairn-node/tests/demographics_sex_gender.rs:346,356`,
  `crates/cairn-node/tests/projection_collation_convergence.rs:363-368`

**Interfaces:**
- Consumes: registry + recipe (Task 1), `cairn_reproject` (Task 2).
- Produces: `patient_identifier_apply(e event_log)`, `patient_demographic_apply(e event_log)`,
  `patient_name_apply(e event_log)`, `patient_address_apply(e event_log)` — registered per the
  inventory table.

- [ ] **Step 1: Update the two tests that call the legacy backfill (failing first)**

In both files replace `SELECT cairn_demographic_backfill()` with
`SELECT count(*) FROM cairn_reproject('demographic.field', false, 'test')` and update the
surrounding comments (the carried-not-projected catch-up and the ADR-0045 heal are now the
generic mechanism's jobs). Run:
`cargo test -p cairn-node --test demographics_sex_gender --test projection_collation_convergence`
Expected: still PASS (the legacy fn coexists mid-task) — these pin behavior through the switch.

- [ ] **Step 2: Convert the five files**

Apply the recipe to each (db/011 shown as the worked example — the others are the same
transformation):

- **db/011**: `DROP TRIGGER IF EXISTS patient_demographic_apply_trg ON event_log;` +
  `DROP FUNCTION IF EXISTS patient_demographic_apply();` before the definition; signature →
  `(e event_log) RETURNS void`; `NEW.` → `e.` throughout (declares `p jsonb := e.body;`);
  `RETURN NULL;` → `RETURN;`; delete the `CREATE TRIGGER` block; append the registration row
  (inventory table: `run_order` 20, tables `{patient_demographic}`).
- **db/010**: same, `patient_identifier_apply`, run_order 10, `{patient_identifier}`.
- **db/012**: same, `patient_name_apply`, run_order 30, `{patient_name}`.
- **db/014**: same, `patient_address_apply`, run_order 10, `{patient_address}`.
- **db/013**: converts the `patient_demographic_apply` REDEFINITION only (no DROPs — db/011
  owns them; no registration — db/011's row covers it; keep its defensive
  `DROP TRIGGER IF EXISTS patient_demographic_apply_trg ON event_log;` as a tombstone, delete
  its `CREATE TRIGGER`). Delete the whole `cairn_demographic_backfill` definition AND the
  `SELECT cairn_demographic_backfill();` call, replaced by:

```sql
-- The one-time carried-not-projected catch-up that lived here (a bespoke
-- cairn_demographic_backfill(), run on EVERY connect) is retired by ADR-0057:
-- the generic cairn_reproject (db/039) replays through the SAME apply fn the
-- trigger uses (one winner-logic implementation, zero drift), and the loader
-- runs it exactly when new projection capability can arrive — on a schema-
-- generation change — instead of on every connect (#208). The DROP below is
-- the tombstone that sheds the fn from upgraded-in-place databases.
DROP FUNCTION IF EXISTS cairn_demographic_backfill();
```

During each file's edit, verify the fn (and any helper it PERFORMs) has no plain INSERT
lacking a conflict clause (audit list in the recipe section); none are expected in 010–014.

- [ ] **Step 3: Run the demographics + convergence suites, then everything**

Run: `cargo test -p cairn-node --test demographics --test demographics_fields --test demographics_names --test demographics_sex_gender --test demographics_address --test projection_collation_convergence` → PASS.
Run: `cargo test --workspace` → PASS.

- [ ] **Step 4: Commit**

```bash
git add db/010_demographics.sql db/011_demographics_fields.sql db/012_demographics_names.sql \
    db/013_demographics_sex_gender.sql db/014_demographics_address.sql \
    crates/cairn-node/tests/demographics_sex_gender.rs \
    crates/cairn-node/tests/projection_collation_convergence.rs
git commit -m "refactor(#208): demographics projections to registered apply fns; retire bespoke backfill"
```

---

### Task 5: Convert identity projections (db/018, 023, 024, 025)

**Files:**
- Modify: `db/018_identity_linkage.sql` (fn ~229–375 + its component-recompute helper),
  `db/023_identity_dispute.sql` (fn 114–193), `db/024_identity_identify.sql` (fn 117–175),
  `db/025_identity_repudiate.sql` (fn 151–208)

**Interfaces:**
- Consumes: registry + recipe.
- Produces: `patient_link_apply(e event_log)`, `chart_dispute_apply(e event_log)`,
  `chart_identity_state_apply(e event_log)`, `name_repudiation_apply(e event_log)` —
  registered per the inventory (018/023/024 register two rows each, same fn).

- [ ] **Step 1: Convert, register, audit idempotency**

Apply the recipe to each file. Registration rows exactly as the inventory table (the two-row
files use one INSERT with two VALUES tuples and the shared `ON CONFLICT` clause). Idempotency
audit (required, in-fn AND helpers): `identity_projection_flag` and `link_veto_flag` inserts —
if a plain INSERT can run twice for one replayed event, add `ON CONFLICT DO NOTHING` on the
natural key (creating a unique index if the table lacks one, e.g.
`CREATE UNIQUE INDEX IF NOT EXISTS link_veto_flag_pair_idx ON link_veto_flag (low, high, content_address);`
— confirm exact columns against the table DDL while editing). Each such change gets a one-line
comment naming ADR-0057 replay-idempotency as the reason.

- [ ] **Step 2: Run the identity suites, then everything**

Run: `cargo test -p cairn-node --test identity_linkage --test identity_dispute --test identity_identify --test identity_repudiate --test link_veto_floor --test apply_proposal` → PASS.
Run: `cargo test --workspace` → PASS.

- [ ] **Step 3: Commit**

```bash
git add db/018_identity_linkage.sql db/023_identity_dispute.sql \
    db/024_identity_identify.sql db/025_identity_repudiate.sql
git commit -m "refactor(#208): identity projections to registered apply fns"
```

---

### Task 6: Convert medication projections (db/031–035) + spike db/008

**Files:**
- Modify: `db/031_medication.sql` (fns 170–228, 247–291), `db/032_medication_dose.sql` (fns
  160–186, 189–221, 305–348), `db/033_medication_reconciliation.sql` (fn 182–252 + helper),
  `db/034_medication_attestation.sql` (fn 232–272),
  `db/035_medication_dose_effective_correction.sql` (fn 148+ — redefinition only, like db/013),
  `db/008_surrogate_projection.sql` (fn 142–163)

**Interfaces:**
- Consumes: registry + recipe.
- Produces: the seven medication apply fns + `surrogate_project_apply(e event_log)`, registered
  per the inventory (db/008 registers its own three rows in-file — it loads only on rigs that
  load it, which is exactly right).

- [ ] **Step 1: Convert, register, audit**

Recipe per file. db/035 converts the `medication_dose_correction_apply` redefinition only
(db/032 owns DROPs + registration). db/032's `medication_dose_seed_initial` already carries
`ON CONFLICT (dose_event_id) DO NOTHING` — the model the audit enforces elsewhere:
`medication_projection_flag` (db/033 helper), `medication_patient_conflict_flag` (db/031
helper), `chart_note_u`/`chart_note_s` (db/008) get `ON CONFLICT DO NOTHING` on their natural
keys if absent (same unique-index-if-needed rule as Task 5).

- [ ] **Step 2: Run the medication suites, then everything, then the SQL mirrors**

Run: `cargo test -p cairn-node --test medication --test medication_dose --test medication_reconciliation --test medication_attestation --test medication_remote_apply --test medication_authorship --test medication_patient_consistency` → PASS.
Run: `cargo test --workspace` → PASS.
Run: `PGHOST=127.0.0.1 PGPORT=5532 scripts/run-db-sql-tests.sh` → PASS (this exercises db/008).

- [ ] **Step 3: Commit**

```bash
git add db/031_medication.sql db/032_medication_dose.sql db/033_medication_reconciliation.sql \
    db/034_medication_attestation.sql db/035_medication_dose_effective_correction.sql \
    db/008_surrogate_projection.sql
git commit -m "refactor(#208): medication + spike surrogate projections to registered apply fns"
```

---

### Task 7: Structural guards — catalog test, row counts, SQL mirror

**Files:**
- Test: `crates/cairn-node/tests/projection_registry.rs` (extend)
- Create: `db/tests/039_projection_registry_test.sql`

**Interfaces:**
- Consumes: the completed conversion (Tasks 1–6). These guards are what make the ADR-0057 rule
  *structural*: an unregistered projection cannot exist, and the registry's membership is
  pinned in two independently-derived places (the #212 discipline).

- [ ] **Step 1: Write the Rust guards (failing only if Tasks 1–6 left a stray)**

```rust
/// ADR-0057's rule, enforced structurally: the dispatcher is the ONLY row-level
/// AFTER INSERT trigger on event_log. A slice that adds a bespoke projection
/// trigger instead of registering an apply fn fails here.
#[tokio::test]
async fn dispatcher_is_the_only_event_log_insert_trigger() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let rows = c
        .query(
            "SELECT t.tgname FROM pg_trigger t
             JOIN pg_class cl ON cl.oid = t.tgrelid
             WHERE cl.relname = 'event_log' AND NOT t.tgisinternal
               AND pg_get_triggerdef(t.oid) LIKE '%AFTER INSERT%'",
            &[],
        )
        .await
        .unwrap();
    let names: Vec<String> = rows.iter().map(|r| r.get(0)).collect();
    assert_eq!(
        names,
        vec!["cairn_projection_dispatch_trg".to_string()],
        "unexpected event_log INSERT triggers: {names:?}"
    );
}

/// Registry membership pinned (product loader — no spike db/008): 22 rows.
/// A new projection slice bumps this AND db/tests/039_projection_registry_test.sql
/// (the #212 two-places discipline; a missed bump fails CI, not drifts).
#[tokio::test]
async fn registry_row_count_is_pinned() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.execute(
        "DELETE FROM cairn_projection_apply WHERE event_type LIKE 'test.%'",
        &[],
    )
    .await
    .unwrap();
    let n: i64 = c
        .query_one("SELECT count(*) FROM cairn_projection_apply", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 22, "cairn_projection_apply row count drifted");
}
```

- [ ] **Step 2: Write the SQL mirror** — `db/tests/039_projection_registry_test.sql`:

```sql
-- db/tests/039_projection_registry_test.sql — SQL mirror of the ADR-0057
-- structural guards (#212 discipline: the Rust guard and this file must both
-- be bumped by a new projection slice; the throwaway DB loads spike db/008, so
-- the expected count here is the product count 22 + db/008's 3 = 25).
DO $$
DECLARE n bigint;
BEGIN
    SELECT count(*) INTO n FROM cairn_projection_apply;
    IF n <> 25 THEN
        RAISE EXCEPTION 'cairn_projection_apply: expected 25 rows on the sqltest DB, found %', n;
    END IF;

    SELECT count(*) INTO n
    FROM pg_trigger t JOIN pg_class cl ON cl.oid = t.tgrelid
    WHERE cl.relname = 'event_log' AND NOT t.tgisinternal
      AND pg_get_triggerdef(t.oid) LIKE '%AFTER INSERT%';
    IF n <> 1 THEN
        RAISE EXCEPTION 'event_log: expected exactly the dispatcher AFTER INSERT trigger, found %', n;
    END IF;

    -- Registration-time fail-closed still bites (bogus fn refused).
    BEGIN
        INSERT INTO cairn_projection_apply (event_type, apply_fn, projection_tables, run_order, heal_safe)
        VALUES ('test.sql.bogus', 'no_such_fn_sql', ARRAY['patient_chart'], 10, true);
        RAISE EXCEPTION 'bogus apply_fn was accepted';
    EXCEPTION WHEN raise_exception THEN
        IF SQLERRM = 'bogus apply_fn was accepted' THEN RAISE; END IF;
        -- expected: the validation trigger raised — swallowed, test passes.
    END;
END $$;
```

- [ ] **Step 3: Run both**

Run: `cargo test -p cairn-node --test projection_registry` → PASS.
Run: `PGHOST=127.0.0.1 PGPORT=5532 scripts/run-db-sql-tests.sh` → PASS (auto-globs the new
file). If either count differs, reconcile against the inventory table — the counts are the
deliverable, do not just edit the number without knowing which row appeared/vanished.

- [ ] **Step 4: Full verification sweep**

Run: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets` → clean.
Run: `cargo test --workspace` → PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-node/tests/projection_registry.rs db/tests/039_projection_registry_test.sql
git commit -m "test(#208): structural guards — sole dispatcher trigger + pinned registry counts"
```

---

### Task 8: The Bet-B-volume measurement (Mac now; Pi follow-on)

**Files:**
- Create: `scripts/bench-reproject.sh`

**Interfaces:**
- Consumes: the finished mechanism; a throwaway DB (same preamble as
  `scripts/run-db-sql-tests.sh`).
- Produces: the two numbers ADR-0057 records: (a) full `cairn_reproject('')` wall-clock at
  ~2M events, (b) per-insert write-path p95 with the dispatcher (the hot-path delta vs. the
  Bet-B B1 baseline p95 3.99 ms @ 2M — a *cross-hardware sanity bound*, not a same-rig A/B;
  the honest same-rig comparison is the Pi re-run follow-on).

- [ ] **Step 1: Write the script**

```bash
#!/usr/bin/env bash
# scripts/bench-reproject.sh — the #208/ADR-0057 measured numbers, reproducibly.
#
# Loads the current db/*.sql into a THROWAWAY database (same safety rules as
# run-db-sql-tests.sh), bulk-generates ~2,004,000 synthetic events (the Bet-B
# volume) as the table owner — the projection/replay path is what is being
# measured, so door-grade signatures are unnecessary; the content-address CHECK
# is satisfied by computing the digest in SQL — then measures:
#   1. per-insert dispatcher write-path latency (p50/p95 over 2,000 single-row
#      inserts against the full log — the B1-shaped number), and
#   2. one full heal replay: SELECT count(*) FROM cairn_reproject('').
# Run on the Mac dev box now; re-run unchanged on the Pi5 rig for the
# authoritative number (the follow-on issue filed at PR time).
set -euo pipefail
cd "$(dirname "$0")/.."

DBNAME="${1:-cairn_reproject_bench}"
case "$DBNAME" in
    cairn_test*)
        echo "refusing: cairn_test* databases belong to the test suites" >&2; exit 2;;
esac

echo "== recreating throwaway database ${DBNAME}"
dropdb --if-exists "$DBNAME"
createdb "$DBNAME"
psql -d "$DBNAME" -v ON_ERROR_STOP=1 -q -c "CREATE EXTENSION cairn_pgx;"

echo "== loading db/*.sql (product set — skipping spike-only 008, like the loaders)"
for f in db/[0-9]*.sql; do
    [ "$(basename "$f")" = "008_surrogate_projection.sql" ] && continue
    psql -d "$DBNAME" -v ON_ERROR_STOP=1 -q -f "$f"
done

echo "== generating ~2,004,000 events (Bet-B volume; 200 patients like B1)"
psql -d "$DBNAME" -v ON_ERROR_STOP=1 -q <<'SQL'
-- Deterministic synthetic corpus: 200 patient.created + per-patient note.added
-- filler + a demographic.field.asserted stream, so replay exercises both the
-- patient_chart path and the winner-table path. Direct owner INSERT: the
-- dispatcher trigger (the measured path) fires exactly as it would at a door.
INSERT INTO event_log (event_id, patient_id, event_type, schema_version,
    hlc_wall, hlc_counter, node_origin, signed_bytes, content_address,
    body, contributors, signer_key_id, plaintext_twin)
SELECT uuidv7(), pid, ty, 'bench-1', 1700000000000 + gs, 0, 'bench-node', sb,
       '\x1220'::bytea || digest(sb, 'sha256'),
       CASE WHEN ty = 'demographic.field.asserted'
            THEN jsonb_build_object('field', 'dob', 'value', '1970-01-01',
                                    'provenance', 'patient-reported')
            ELSE jsonb_build_object('name', 'Bench P' || (gs % 200), 'note', gs) END,
       '[]'::jsonb, 'bench-key', 'bench twin ' || gs
FROM (
    SELECT gs,
           ('00000000-0000-7000-8000-' || lpad(to_hex(gs % 200), 12, '0'))::uuid AS pid,
           CASE WHEN gs <= 200 THEN 'patient.created'
                WHEN gs % 10 = 0 THEN 'demographic.field.asserted'
                ELSE 'note.added' END AS ty,
           ('bench-' || gs)::bytea AS sb
    FROM generate_series(1, 2004000) AS gs
) g;
ANALYZE event_log;
SQL

echo "== (1) dispatcher write-path latency: 2,000 single-row inserts at full log"
psql -d "$DBNAME" -v ON_ERROR_STOP=1 <<'SQL'
DO $$
DECLARE
    t0 timestamptz; deltas double precision[] := '{}'; i int;
    pid uuid := '00000000-0000-7000-8000-000000000001';
    sb bytea;
BEGIN
    FOR i IN 1..2000 LOOP
        sb := ('bench-tail-' || i)::bytea;
        t0 := clock_timestamp();
        INSERT INTO event_log (event_id, patient_id, event_type, schema_version,
            hlc_wall, hlc_counter, node_origin, signed_bytes, content_address,
            body, contributors, signer_key_id, plaintext_twin)
        VALUES (uuidv7(), pid, 'note.added', 'bench-1', 1800000000000 + i, 0,
            'bench-node', sb, '\x1220'::bytea || digest(sb, 'sha256'),
            jsonb_build_object('note', i), '[]'::jsonb, 'bench-key', 'tail');
        deltas := deltas || extract(epoch FROM clock_timestamp() - t0) * 1000.0;
    END LOOP;
    RAISE NOTICE 'write-path ms  p50=%  p95=%  max=%',
        (SELECT percentile_cont(0.5)  WITHIN GROUP (ORDER BY d) FROM unnest(deltas) d),
        (SELECT percentile_cont(0.95) WITHIN GROUP (ORDER BY d) FROM unnest(deltas) d),
        (SELECT max(d) FROM unnest(deltas) d);
END $$;
SQL

echo "== (2) full heal replay at Bet-B volume"
psql -d "$DBNAME" -v ON_ERROR_STOP=1 -c "\\timing on" \
     -c "SELECT * FROM cairn_reproject('', false, 'manual');" \
     -c "SELECT prefix, events_seen, elapsed_ms, skipped_fns FROM reproject_log ORDER BY id DESC LIMIT 1;"

echo "== done — record BOTH numbers (env: $(uname -srm), PG $(psql -d "$DBNAME" -tAc 'show server_version'))"
```

`chmod +x scripts/bench-reproject.sh`. If any column/NOT-NULL in the synthetic INSERT
mismatches `db/001` (e.g. `actor_id` gained a constraint), fix against the current DDL — the
CHECK constraint formula (`'\x1220' || digest(signed_bytes,'sha256')`) is the invariant part.

- [ ] **Step 2: Run it on the Mac (:5532)**

Run: `PGHOST=127.0.0.1 PGPORT=5532 PGUSER=hherb scripts/bench-reproject.sh`
Expected: completes; record (a) the `elapsed_ms` of the full replay, (b) the write-path
p50/p95, (c) events/second = 2,004,000 / elapsed. Sanity gates: write-path p95 well under the
4 ms Bet-B budget (Mac is far faster than the Pi); full replay in minutes, not hours. If the
full replay exceeds ~30 min on the Mac, STOP and reopen the design's Approach-C fallback
(per-type set-based override) before proceeding — that is the measurement doing its job.

- [ ] **Step 3: Commit**

```bash
git add scripts/bench-reproject.sh
git commit -m "bench(#208): reproducible reproject + dispatcher write-path measurement at Bet-B volume"
```

---

### Task 9: ADR-0057 + spec prose

**Files:**
- Create: `docs/spec/decisions/0057-generic-reprojection-registered-apply-dispatch.md`
- Modify: `docs/spec/language-substrate.md` (§9.6 — the validated write surface; follow
  ADR-0048's existing §9.6 text as the anchor), `docs/spec/sync.md` (§6.5 — one cross-reference sentence
  in the reclassification paragraph), `docs/spec/index.md` (spec version → v0.59, ADR list row)

**Interfaces:**
- Consumes: the measured numbers from Task 8; the follow-on issue numbers from Task 10 are
  cross-referenced the other way (issue bodies cite the ADR), so no forward reference is
  needed here.

- [ ] **Step 1: Write ADR-0057**

Follow the house ADR shape (status/date/context/decision/consequences; symbol-level references
only, never `file:line` — ADRs are immutable and lines move; the PR #271 review made exactly
this correction to ADR-0056). Content = the approved spec doc's decisions D1–D5 plus the three
planning amendments; decisions numbered:

1. **One code path — registration is the wiring** (registry `cairn_projection_apply`;
   `heal_safe` semantics; exact-type rows, no wildcard; the ONE dispatcher trigger; ADR-0048
   discipline: fail-closed registration, locked rows, two-place count guards).
2. **`cairn_reproject` heal/rebuild semantics** (heal converges by order-independence +
   insert-or-better, never downgrades because a stale winner is itself a candidate; skip
   semantics for heal-unsafe rows; rebuild's shared-table refusal; lenient replay posture —
   `cairn.remote_apply` — with the ADR-0056 gate-effect-not-presence argument; the
   `cairn_replay_eligible` seam and why a manual reproject cannot promote a deferred event).
3. **Loader heal on generation change, never per connect** (subsumes and retires the db/013
   bespoke backfill; unknown generation errs toward healing; both loaders, subset included).
4. **The written rule** — *a projection lives only in its registered apply function; healing
   is generic replay, run by the loader on generation change* — and its structural
   enforcement (sole-trigger catalog guard + pinned counts).
5. **Measured cost** — the Task-8 Mac numbers, hardware named, with the Pi5 re-run recorded
   as the authoritative follow-on (issue filed in Task 10).

Refines ADR-0048/0045; upholds ADR-0056 decision 4; load-bearing for #266.

- [ ] **Step 2: Spec prose**

§9.6: after the ADR-0048 registry paragraph, add the projection-registry paragraph (the rule,
the dispatcher, heal/rebuild, the loader gate, `heal_safe` honesty, the eligibility seam).
§6.5: one sentence in the reclassification path — reclassification re-adjudicates the deferred
gates and then reprojects *via the §9.6 generic mechanism*. `index.md`: version v0.59 + ADR
row. Build check:
`uv run --with-requirements docs/requirements.txt -- mkdocs build` → clean.

- [ ] **Step 3: Commit**

```bash
git add docs/spec/decisions/0057-generic-reprojection-registered-apply-dispatch.md \
    docs/spec/api-layering.md docs/spec/sync.md docs/spec/index.md
git commit -m "docs(#208): ADR-0057 — generic reprojection, registration is the wiring (spec v0.59)"
```

---

### Task 10: Housekeeping — ROADMAP, HANDOVER, follow-on issue, PR

**Files:**
- Modify: `docs/ROADMAP.md` (Slice 49), `docs/HANDOVER.md` (regenerate per its own rules;
  ADR-index row for 0057; move #208 to closed; #266 note now points at the landed mechanism)

**Interfaces:** none (process).

- [ ] **Step 1: File the Pi follow-on issue**

```bash
gh issue create --title "bench(#208): re-run scripts/bench-reproject.sh on the Pi5 rig for the authoritative reproject cost" \
  --body "ADR-0057 decision 5 records Mac-measured numbers (hardware named). Re-run scripts/bench-reproject.sh unchanged on the Bet-B Pi5/NVMe rig (PG18) and fold the authoritative full-replay + dispatcher write-path numbers into a dated addendum note beside the ADR (ADRs are immutable — the addendum is a new dated section in HANDOVER + a spec §9.6 number, not an ADR edit). Numbers to capture: full cairn_reproject('') elapsed at 2,004,000 events; dispatcher write-path p50/p95 vs the B1 baseline (p95 3.99 ms). Related: #208 (mechanism, closed by ADR-0057), #266 (consumes cairn_reproject)."
```

Also drop a one-line comment on #266: `cairn_reproject` + `cairn_replay_eligible` landed
(ADR-0057) — the reclassification flow consumes them.

- [ ] **Step 2: Update ROADMAP (Slice 49) + regenerate HANDOVER**

Slice 49 = this arc (one paragraph: mechanism, the heal_safe/counter finding, the lenient-
posture finding, the numbers, follow-ons). HANDOVER: new session block; ADR table row 0057;
P6 queue now `#216/#217 remain`; keep both files ≤ ~500 lines by pruning per their standing
rules (HANDOVER-fixes-in-work-PR memory: bundle these in this PR).

- [ ] **Step 3: Final verification + PR**

Run: `cargo test --workspace` → PASS. Run: `PGHOST=127.0.0.1 PGPORT=5532
scripts/run-db-sql-tests.sh` → PASS. Run the mkdocs build → clean. Then:

```bash
git add docs/ROADMAP.md docs/HANDOVER.md
git commit -m "docs(#208): ROADMAP Slice 49 + HANDOVER regeneration"
git push -u origin design/208-generic-reprojection
gh pr create --title "#208/ADR-0057: generic reprojection — registration is the wiring" \
  --body "$(cat <<'EOF'
Closes #208. Design session + implementation + measurement per
docs/superpowers/specs/2026-07-20-generic-reprojection-design.md and ADR-0057.

- One code path: ~15 per-type projection triggers become registered apply fns
  behind a single dispatcher (`cairn_projection_apply`, ADR-0048 discipline).
- `cairn_reproject(prefix, rebuild)`: generic heal/rebuild replay through the
  IDENTICAL dispatch; heal_safe skip semantics for counter-shaped projections;
  shared-table rebuild refusal; lenient replay posture; `cairn_replay_eligible`
  seam keeps ADR-0056 decision 4 safe for #266.
- Loader heals on generation change only — the db/013 every-connect backfill is
  retired (the #208 headline cost).
- Structural guards: sole-dispatcher catalog test + two-place row-count pins.
- Measured at Bet-B volume on the Mac (numbers in ADR-0057 §5); Pi5 re-run
  filed as a follow-on.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## Self-review notes (done at plan-writing time)

- **Spec coverage:** D1→Tasks 1/4/5/6; D2→Task 2; D3→Tasks 2 (seam + test); D4→Task 3;
  D5→Tasks 7/9; §4 flags→hot-path number in Task 8 + sibling-read check resolved in the
  inventory table; §5 testing list→Tasks 1–7; §5 measurement→Task 8; §6 follow-ons→Task 10.
- **Deliberate deviations from the approved spec, all listed in the amendments section:**
  no `'*'` rows; `heal_safe`; lenient posture. Task 0 folds them back into the spec doc.
- **Counts (22/25) derive from the inventory table**; Task 7 instructs reconciliation, not
  blind edits, if they drift.
- **The `recorded` hoist in cairn-sync (Task 3)** is the one place the plan changes existing
  control flow beyond an append; it is confined to `load_schema_under_lock`.
