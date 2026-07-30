# ADR-0056 floor: admit uninterpreted, re-adjudicate before power — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The clinical remote door admits a verifiable event whose `event_type` it cannot classify — storing it verbatim, projecting nothing, conferring no power — and grants that power only after re-running the classification-gated floor checks when classifying code arrives.

**Architecture:** A node-local `event_deferred` sidecar table records the admitted-uninterpreted state explicitly. `apply_remote_event` stops raising on an unknown type and writes a marker row instead. `cairn_replay_eligible` — the seam ADR-0057 built for exactly this — becomes "no marker row", so no reprojection path can grant power to an unadjudicated event. `cairn_readjudicate_deferred()` re-runs the three deferred gates and deletes the marker on pass / records the reason on fail; the loader calls it every connect, before reprojection.

**Tech Stack:** PostgreSQL 18 + PL/pgSQL (the in-DB floor, ADR-0001), Rust (`cairn-node` loader + CLI), `tokio-postgres`. DB-gated tests need `CAIRN_TEST_PG` (PG18 + `cairn_pgx`).

**Design doc:** [`docs/superpowers/specs/2026-07-29-adr-0056-admit-uninterpreted-floor-design.md`](../specs/2026-07-29-adr-0056-admit-uninterpreted-floor-design.md)
**Issues:** [#265](https://github.com/cairn-ehr/cairn-ehr/issues/265), [#266](https://github.com/cairn-ehr/cairn-ehr/issues/266)
**ADR:** [ADR-0056](../../spec/decisions/0056-unknown-event-types-admitted-uninterpreted.md) decisions 1 and 4

## Global Constraints

- **Licence:** AGPL-3.0. No new dependencies in this slice.
- **TDD:** the failing test comes first, always. No production code without a test that drove it.
- **Junior-readable comments:** every non-trivial function/block explains *why* it exists and how it fits, not what the next line does.
- **File size:** keep files under 500 lines where feasible. `db/005_submit.sql` is already 948 lines — this slice must not grow it materially (it adds ~12 lines to existing functions; no new function goes there).
- **Never hard-code cryptographic material in tests** — derive keys at runtime (`generate_key()`), never byte literals (house rule 6, issue #146).
- **Full-workspace verification:** `cargo test --workspace`, never a per-crate run. Slice 6b's lesson: only the full run catches guard-scope gaps, and `cargo test | tail` masks cargo's exit code — never pipe it.
- **Test env:** `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test"`. DB-gated tests self-skip without it.
- **Both loader lists:** `db/043` is floor machinery, not medication-specific, so it must be added to **both** `crates/cairn-node/src/db.rs::SCHEMA` and `crates/cairn-sync/src/main.rs::SCHEMA` (issue #284's drift hazard — cairn-sync loads `db/020` and would otherwise call a function whose helper it never loaded).
- **`SCHEMA_GENERATION`** is `42` today and must become `43` in the same commit that creates `db/043` — `cairn-event`'s fs-derived guard test asserts `constant == newest db/*.sql prefix`.

## Paper-parity (§1.2)

Paper-parity: not clinical-surface — this slice changes only which events a node retains and when their power is granted; it adds no human act at any layer and exposes no runnable clinical surface. Its paper counterpart (a referral letter you cannot fully read still stays in the folder, visible and forwardable) motivates the change rather than being a workflow it introduces, and the change strictly increases what the chart holds, so no workflow becomes slower, harder, or impossible.

## File Structure

| File | Responsibility | Change |
|---|---|---|
| `db/001_envelope.sql` | the wire envelope + `event_log` | **Modify** — add the `event_deferred` 1:1 sidecar table. It lives here, not in this slice's own `db/043`, because `db/005`'s `cairn_replay_eligible` and `cairn_suppression_author_ok` are `LANGUAGE sql`, which resolves table names at **CREATE** time (unlike PL/pgSQL's late binding) — so the table must exist before `db/005` loads. |
| `db/005_submit.sql` | the strict door + the registries + shared predicates | **Modify** — three surgical edits: the projection-registration guard, `cairn_replay_eligible`'s real body, and `cairn_suppression_author_ok`'s deferred-target neutrality. The strict door itself is **unchanged** (ADR-0056 decision 2). |
| `db/020_apply_remote_event.sql` | the clinical remote door | **Modify** — the classification lookup stops raising; steps 4 and 5 become conditional; the marker row is written. |
| `db/043_deferred_readjudication.sql` | **Create** — `cairn_readjudicate_deferred()` + grants | the reclassification pass. New file, so it carries the generation bump. |
| `db/tests/043_deferred_readjudication_test.sql` | **Create** — the pure-SQL mirror | run by `scripts/run-db-sql-tests.sh` in CI since PR #251. |
| `crates/cairn-node/src/db.rs` | the schema loader | **Modify** — `SCHEMA` gains `043`; `connect_and_load_schema` calls the pass before reprojection. |
| `crates/cairn-sync/src/main.rs` | the clinical puller (its own subset loader) | **Modify** — `SCHEMA` gains `043` only. No behaviour change in this slice. |
| `crates/cairn-node/src/main.rs` | the CLI | **Modify** — the `Deferred` subcommand. |
| `crates/cairn-event/src/schema_generation.rs` | the repo-wide generation constant | **Modify** — `42` → `43`. |
| `crates/cairn-node/tests/deferred_admission.rs` | **Create** — the slice's integration tests | door + gate + re-adjudication + the security test. |

---

### Task 1: The `event_deferred` marker table + the projection-registration guard

**Files:**
- Modify: `db/001_envelope.sql` (append a table before `COMMIT;`)
- Modify: `db/005_submit.sql:103-125` (`cairn_check_projection_registry_fn`)
- Test: `crates/cairn-node/tests/deferred_admission.rs` (create)

**Interfaces:**
- Consumes: nothing.
- Produces: table `event_deferred(event_id UUID PK, event_type TEXT, admitted_at TIMESTAMPTZ, adjudication_error TEXT, last_attempt_at TIMESTAMPTZ)`. Every later task reads or writes it.

**Why the registration guard is load-bearing, not hygiene:** the marker row is written *after* the `event_log` INSERT, but the AFTER-INSERT projection dispatcher fires *during* it. So a type that were projection-registered without being classified would project a deferred event at admission — granting exactly the power the marker exists to withhold. This guard makes that state unreachable at migration time. It is also one of the two legs holding up the design-doc §4.2 audit conclusion that no projection apply fn can ever see a deferred row.

- [ ] **Step 1: Write the failing test**

Create `crates/cairn-node/tests/deferred_admission.rs`:

```rust
//! ADR-0056 decisions 1 + 4 (issues #265/#266): the clinical remote door admits an event
//! whose `event_type` it cannot classify — stored verbatim, no projection rows, no power —
//! and power is granted only after the classification-gated floor checks are re-run.
//!
//! Real Postgres, gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.
use cairn_node::db;
use tokio_postgres::Client;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// The Postgres error message for a failed statement (Display renders only "db error";
/// the RAISE text lives in the DbError payload — project convention).
fn db_msg(e: &tokio_postgres::Error) -> String {
    e.as_db_error()
        .map(|d| d.message().to_string())
        .unwrap_or_else(|| e.to_string())
}

/// A projection may only be registered for a CLASSIFIED type. Without this guard the
/// AFTER-INSERT dispatcher would project an event the remote door admitted uninterpreted,
/// granting the very power the deferred marker exists to withhold (ADR-0056 decision 4).
#[tokio::test]
async fn projection_registration_requires_a_classified_type() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c: Client = db::connect_and_load_schema(&base).await.unwrap();
    // patient_chart_apply exists and patient_chart is a real relation, so the two
    // pre-existing registration guards pass — only the classification one can fire.
    let err = c
        .execute(
            "INSERT INTO cairn_projection_apply (event_type, apply_fn, projection_tables) \
             VALUES ('unclassified.for.test', 'patient_chart_apply', ARRAY['patient_chart'])",
            &[],
        )
        .await
        .expect_err("registering a projection for an unclassified type must fail closed");
    assert!(
        db_msg(&err).contains("not classified in event_type_class"),
        "got: {}",
        db_msg(&err)
    );
}

/// The marker table is the explicit deferred state ADR-0056's corollary demands
/// ("never inferred from a null classification lookup"). Pin its shape.
#[tokio::test]
async fn event_deferred_table_has_the_designed_shape() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c: Client = db::connect_and_load_schema(&base).await.unwrap();
    let cols: Vec<String> = c
        .query(
            "SELECT column_name::text FROM information_schema.columns \
             WHERE table_name = 'event_deferred' ORDER BY column_name",
            &[],
        )
        .await
        .unwrap()
        .iter()
        .map(|r| r.get(0))
        .collect();
    assert_eq!(
        cols,
        vec![
            "adjudication_error",
            "admitted_at",
            "event_id",
            "event_type",
            "last_attempt_at"
        ],
        "event_deferred shape drifted from the design"
    );
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" \
  cargo test -p cairn-node --test deferred_admission
```

Expected: both FAIL — `event_deferred` does not exist (empty column list), and the projection registration *succeeds* instead of erroring.

- [ ] **Step 3: Add the table to `db/001_envelope.sql`**

Insert immediately before the file's closing `COMMIT;`:

```sql
-- ── The admitted-uninterpreted marker (ADR-0056 decision 1, issue #265) ────────────
-- A 1:1 node-local sidecar of event_log: one row means "this event was admitted by the
-- remote door WITHOUT its type being classified, so it holds NO power". Node-local
-- derived state — never signed, never on the wire (principle 12), like reproject_log
-- and node_schema.
--
-- WHY EXPLICIT, not inferred from a missing event_type_class row: ADR-0056's corollary
-- forbids inferring deferral from a null classification lookup falling through the
-- gates by three-valued logic. An inferred marker also has nowhere to record WHY a
-- re-adjudication attempt failed, which decision 4 requires to be flagged legibly.
--
-- WHY IT LIVES IN db/001 rather than this slice's own db/043: db/005's
-- cairn_replay_eligible and cairn_suppression_author_ok are LANGUAGE sql, and SQL-language
-- function bodies resolve table names at CREATE time (unlike PL/pgSQL's late binding).
-- The table must therefore exist before db/005 loads. Its consumers are documented in
-- db/043_deferred_readjudication.sql.
--
-- The row is DELETED on promotion, never flagged as resolved: its presence IS the
-- invariant ("powerless, classification-gated checks not yet passed"), and a resolved-row
-- history would be a second, drift-prone source of truth for the same fact.
CREATE TABLE IF NOT EXISTS event_deferred (
    event_id           UUID PRIMARY KEY REFERENCES event_log(event_id) ON DELETE CASCADE,
    -- Denormalized from event_log so the reclassification scan can select its candidates
    -- by joining against event_type_class alone, and the CLI listing needs no join.
    event_type         TEXT        NOT NULL,
    -- Node-local operational stamp (clock_timestamp, like reproject_log.ran_at). NEVER a
    -- clinical time: t_recorded/t_effective live on event_log and are the only times that
    -- mean anything about the record.
    admitted_at        TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    -- NULL until a re-adjudication attempt has run and FAILED; then the verbatim refusal.
    -- This is decision 4's "flagged legibly".
    adjudication_error TEXT,
    last_attempt_at    TIMESTAMPTZ
);

-- The reclassification scan joins event_deferred → event_type_class on this column.
CREATE INDEX IF NOT EXISTS event_deferred_type_idx ON event_deferred (event_type);
```

- [ ] **Step 4: Add the classification check to `db/005_submit.sql`**

In `cairn_check_projection_registry_fn`, immediately after the `to_regprocedure` check and before the `FOREACH v_tbl` loop:

```sql
    -- ADR-0056 decision 4 (issue #266): a projection-registered type MUST be classified.
    -- The remote door admits an UNCLASSIFIED type uninterpreted and records an
    -- event_deferred marker AFTER the event_log INSERT — but the AFTER-INSERT dispatcher
    -- fires DURING that INSERT. So a type registered here without an event_type_class row
    -- would be projected at admission, granting exactly the power the marker withholds.
    -- Making that unreachable at migration time is cheaper and safer than defending
    -- against it at runtime, and it is one of the two legs (the other is
    -- cairn_replay_eligible) of the guarantee that NO projection apply fn ever sees a
    -- deferred row — which is what lets db/018 and db/034 keep trusting
    -- event_log.attester_key.
    IF NOT EXISTS (SELECT 1 FROM event_type_class WHERE event_type = NEW.event_type) THEN
        RAISE EXCEPTION
            'cairn_projection_apply: event_type "%" is not classified in event_type_class (fail closed) — classify it before registering a projection, or the dispatcher would project an event admitted uninterpreted',
            NEW.event_type;
    END IF;
```

- [ ] **Step 5: Verify no existing migration registers a projection before its class row**

This guard fires at INSERT time, so any migration whose `cairn_projection_apply` INSERT precedes its `event_type_class` INSERT will now fail to load. Check every file:

```bash
for f in db/0*.sql; do
  cls=$(grep -n "INSERT INTO event_type_class" "$f" | head -1 | cut -d: -f1)
  prj=$(grep -n "INSERT INTO cairn_projection_apply" "$f" | head -1 | cut -d: -f1)
  if [ -n "$cls" ] && [ -n "$prj" ] && [ "$prj" -lt "$cls" ]; then
    echo "ORDER PROBLEM: $f registers a projection (line $prj) before classifying (line $cls)"
  fi
done
```

Expected: no output. If a file *does* report, move its `event_type_class` INSERT above its `cairn_projection_apply` INSERT — never weaken the guard.

- [ ] **Step 6: Run the tests to verify they pass**

```bash
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" \
  cargo test -p cairn-node --test deferred_admission
```

Expected: PASS (2 tests). Then confirm nothing else regressed on the schema load:

```bash
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" \
  cargo test -p cairn-node --test projection_registry
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add db/001_envelope.sql db/005_submit.sql crates/cairn-node/tests/deferred_admission.rs
git commit -m "$(cat <<'EOF'
feat(ADR-0056): the event_deferred marker + a classified-before-projected guard

The marker is the EXPLICIT deferred state ADR-0056's corollary demands, never
inferred from a null classification lookup. It lives in db/001 because db/005's
LANGUAGE sql predicates resolve table names at CREATE time.

The registration guard is load-bearing, not hygiene: the marker row is written
after the event_log INSERT while the projection dispatcher fires during it, so
an unclassified-but-projection-registered type would be projected at admission.
Making that unreachable at migration time is what lets db/018 and db/034 keep
trusting event_log.attester_key.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: The remote door admits uninterpreted

**Files:**
- Modify: `db/020_apply_remote_event.sql` (the DECLARE block, step 3, steps 4-5, and after the substitution guard)
- Modify: `crates/cairn-node/tests/apply_remote_event.rs:150-170` (the existing fail-closed test inverts)
- Test: `crates/cairn-node/tests/deferred_admission.rs` (extend)

**Interfaces:**
- Consumes: `event_deferred` from Task 1.
- Produces: `apply_remote_event` writes an `event_deferred` row for an unclassified type and stores the travelling attestation token unverified on `event_log`.

- [ ] **Step 1: Write the failing tests**

Append to `crates/cairn-node/tests/deferred_admission.rs`. Add these imports at the top of the file:

```rust
use cairn_event::{generate_key, sign, EventBody, Hlc, SigningKey};
use uuid::Uuid;
```

and this shared scaffolding plus the tests:

```rust
/// A realistic HLC wall (ms since epoch, ≈ 2026-06-21) so the t_effective ceiling compares
/// against a sane "recorded" instant rather than 1970.
const WALL_2026: i64 = 1_782_000_000_000;

/// A type no migration classifies. The door must ADMIT it, not refuse it.
const UNKNOWN_TYPE: &str = "clinical.medication.recall";

/// Truncate the clinical tables and enroll one agent signer + one human attester.
/// `TRUNCATE event_log ... CASCADE` clears event_deferred through its FK.
async fn setup(c: &Client) -> (SigningKey, String, SigningKey, String) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, patient_identifier, \
         patient_demographic, patient_link, person_member, identity_projection_flag, \
         t_effective_ceiling_flag CASCADE",
    )
    .await
    .unwrap();
    c.batch_execute("UPDATE hlc_state SET hlc_wall = 0, hlc_counter = 0")
        .await
        .unwrap();
    let (sk_a, kid_a) = generate_key().unwrap();
    let (sk_h, kid_h) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('agent', '{\"model\":\"sync-peer-stub\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
        &[&kid_a],
    ).await.unwrap();
    c.execute(
        "SELECT enroll_actor('human', '{\"role\":\"clinician\"}', $1)",
        &[&kid_h],
    )
    .await
    .unwrap();
    (sk_a, kid_a, sk_h, kid_h)
}

/// Build a signed event of an arbitrary type "arriving from a peer".
fn peer_event(kid: &str, patient: Uuid, ty: &str, wall: i64) -> EventBody {
    EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: ty.into(),
        schema_version: "future/1".into(),
        hlc: Hlc {
            wall,
            counter: 0,
            node_origin: "upgraded-peer".into(),
        },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: serde_json::json!({"reason": "batch recall"}),
        // No authored twin: the skeleton fallback must carry it (ADR-0039).
        plaintext_twin: None,
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
    }
}

/// ADR-0056 decision 1: an unclassifiable type is ADMITTED — stored verbatim, no projection
/// rows, no power — and marked deferred. This is the §6.1 sneakernet case: a carrier node
/// must stop being a propagation barrier.
#[tokio::test]
async fn unknown_type_is_admitted_and_marked_deferred() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let p = Uuid::now_v7();
    let b = peer_event(&kid, p, UNKNOWN_TYPE, WALL_2026);
    let signed = sign(&b, &sk).unwrap();

    c.execute("SELECT apply_remote_event($1)", &[&signed.signed_bytes.to_vec()])
        .await
        .expect("an unclassifiable type must be ADMITTED, not refused (ADR-0056 decision 1)");

    // Stored verbatim.
    let stored: i64 = c
        .query_one(
            "SELECT count(*) FROM event_log WHERE event_type = $1",
            &[&UNKNOWN_TYPE],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(stored, 1, "the event must be in event_log verbatim");

    // Marked deferred — explicitly, not inferred.
    let marked: i64 = c
        .query_one(
            "SELECT count(*) FROM event_deferred WHERE event_type = $1",
            &[&UNKNOWN_TYPE],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(marked, 1, "an admitted-uninterpreted event must carry a marker");

    // No power: the skeleton twin renders it, and no chart row was created for it.
    let twin: String = c
        .query_one(
            "SELECT plaintext_twin FROM event_log WHERE event_type = $1",
            &[&UNKNOWN_TYPE],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        !twin.trim().is_empty(),
        "the skeleton twin must render an unregistered type (ADR-0039 honest degradation)"
    );
}

/// ADR-0056 decision 2: the STRICT door keeps failing closed. A node may CARRY a type it
/// has no code for; it may never AUTHOR one. This is the regression pin for the slice's
/// whole risk — over-relaxing.
#[tokio::test]
async fn strict_door_still_refuses_an_unclassifiable_type() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let p = Uuid::now_v7();
    let b = peer_event(&kid, p, UNKNOWN_TYPE, WALL_2026);
    let signed = sign(&b, &sk).unwrap();

    let err = c
        .execute("SELECT submit_event($1)", &[&signed.signed_bytes.to_vec()])
        .await
        .expect_err("submit_event must still refuse a type this node cannot classify");
    assert!(
        db_msg(&err).contains("fail closed") || db_msg(&err).contains("unknown event_type"),
        "got: {}",
        db_msg(&err)
    );
    let marked: i64 = c
        .query_one("SELECT count(*) FROM event_deferred", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(marked, 0, "a refused local author must leave no marker");
}
```

- [ ] **Step 2: Run to verify they fail**

```bash
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" \
  cargo test -p cairn-node --test deferred_admission unknown_type_is_admitted
```

Expected: FAIL with `apply_remote_event: unknown event_type clinical.medication.recall (no classification — fail closed)`.

- [ ] **Step 3: Add the deferred flag to `db/020`'s DECLARE block**

After the existing `v_twin_stub TEXT;` line:

```sql
    -- ADR-0056 decision 1 (issue #265): true when this node holds no classification for
    -- the event's type. The event is ADMITTED anyway — custody is total, power is deferred.
    v_deferred      BOOLEAN := false;
```

- [ ] **Step 4: Replace `db/020` step 3's fail-closed raise**

Replace the block currently reading:

```sql
    -- 3. Classify (fail closed on unknown type; ADR-0010/ADR-0012 — an older node
    --    refuses a type it cannot classify rather than guessing its mode).
    SELECT mode, targets_other_author INTO v_mode, v_targets_other
        FROM event_type_class WHERE event_type = v_type;
    IF v_mode IS NULL THEN
        RAISE EXCEPTION 'apply_remote_event: unknown event_type % (no classification — fail closed)', v_type;
    END IF;
```

with:

```sql
    -- 3. Classify — and ADMIT-AND-DEFER when we cannot (ADR-0056 decision 1, issue #265).
    --    This door used to RAISE here. That made §6.5's lossless-forwarding invariant
    --    FALSE for unknown types: a phone-tier node carrying a chart between two upgraded
    --    facilities (the §6.1 sneakernet path, the case Cairn exists for) acquired NOTHING
    --    past the first unknown-type event — the event was not merely unrendered, it was
    --    absent. Admission cannot hide anything; refusal can.
    --
    --    A deferred event is stored verbatim, re-propagated, exported, and rendered by the
    --    skeleton twin (step 8 needs no change: cairn_event_twin finds no registry row for
    --    an unregistered type, so it falls through to cairn_twin_skeleton and never raises).
    --    It yields NO projection rows and confers NO power — see the marker INSERT below and
    --    cairn_replay_eligible (db/005).
    --
    --    The STRICT door (db/005) deliberately still fails closed: a node may CARRY a type
    --    it has no code for, never AUTHOR one (decision 2, ADR-0051's strict-submit/
    --    lenient-apply asymmetry applied to types).
    SELECT mode, targets_other_author INTO v_mode, v_targets_other
        FROM event_type_class WHERE event_type = v_type;
    v_deferred := (v_mode IS NULL);
```

- [ ] **Step 5: Make steps 4 and 5 conditional, and store the travelling token**

Replace step 4's opening gate

```sql
    IF v_mode = 'suppressing' OR v_bears THEN
```

with

```sql
    -- The DEFERRED arm: store the travelling attestation token WITHOUT gating on it.
    --
    -- This is not an optimisation, it is what keeps admit-and-defer from silently
    -- degrading into a slower fail-closed. A suppressing event's attestation token
    -- TRAVELS with it on the sync wire, and the gate below is the only thing that used to
    -- store it. Skip the gate naively and the token is dropped — so when classifying code
    -- later arrives, cairn_readjudicate_deferred (db/043) has nothing to verify and the
    -- event can NEVER gain power. Storing it costs nothing and keeps re-adjudication
    -- possible.
    --
    -- INVARIANT, and the reason db/005's cairn_suppression_author_ok had to change: an
    -- attestation on a row that carries an event_deferred marker is CARRIED, NOT VOUCHED.
    -- Nothing has verified it. Every reader must therefore either be unreachable for
    -- deferred rows (the projection apply fns in db/018 and db/034 — see the registration
    -- guard in db/005) or exclude them explicitly (cairn_suppression_author_ok, which
    -- reads the TARGET's attester_key and is reachable).
    IF v_deferred THEN
        v_att     := p_attestation;
        v_att_key := p_attester_key;
    END IF;

    IF NOT v_deferred AND (v_mode = 'suppressing' OR v_bears) THEN
```

Then guard step 5 by replacing

```sql
    IF v_targets_other THEN
```

with

```sql
    -- Deferred events skip this too — v_targets_other is NULL for an unclassified type, so
    -- the branch would short-circuit anyway, but relying on three-valued logic is exactly
    -- what ADR-0056's corollary forbids. Make the skip EXPLICIT so the reader can see that
    -- the overlay-target-exists check and the ADR-0043 owner-gate are DEFERRED WITH the
    -- interpretation, not waived by it — cairn_readjudicate_deferred re-runs both before
    -- any power is granted (decision 4).
    IF NOT v_deferred AND v_targets_other THEN
```

- [ ] **Step 6: Write the marker row**

Immediately after the substitution-guard block (the `IF v_rows = 0 THEN ... END IF;` that ends with the `substitution refused` RAISE) and before `PERFORM cairn_learn_attachment_refs(b);`:

```sql
    -- Record the deferred state EXPLICITLY (ADR-0056 decision 4's corollary): a node
    -- records that an event was admitted uninterpreted, and that marker — not the absence
    -- of an event_type_class row — is what reclassification consumes. Written AFTER the
    -- log INSERT because of the FK; the AFTER-INSERT projection dispatcher that fires
    -- during that INSERT can therefore not see it, which is why db/005's registration
    -- guard (classified-before-projected) is what actually keeps a deferred event
    -- unprojected at admission.
    -- ON CONFLICT DO NOTHING: an idempotent re-apply of the same event must stay a silent
    -- no-op (set-union), and must never reset a recorded adjudication_error.
    IF v_deferred THEN
        INSERT INTO event_deferred (event_id, event_type)
        VALUES (v_event_id, v_type)
        ON CONFLICT (event_id) DO NOTHING;
    END IF;
```

- [ ] **Step 7: Invert the existing fail-closed test**

In `crates/cairn-node/tests/apply_remote_event.rs`, the test asserting `db_msg(&err).contains("fail closed")` for `mystery.op` now contradicts the ratified contract. Replace its body's assertion so it pins ADMISSION, and rename it:

```rust
/// ADR-0056 decision 1 (issue #265): the remote door ADMITS a type it cannot classify.
/// This test previously pinned the opposite (`fail closed`) — the behaviour that made
/// §6.5's lossless-forwarding invariant false for unknown types. Full coverage of the
/// deferred lifecycle lives in tests/deferred_admission.rs.
#[tokio::test]
async fn unknown_type_is_admitted_uninterpreted() {
    // ... unchanged setup through `let signed = sign(&b, &sk).unwrap();` ...
    apply(&c, &signed.signed_bytes)
        .await
        .expect("an unclassifiable type is admitted uninterpreted, not refused");
    let deferred: i64 = c
        .query_one(
            "SELECT count(*) FROM event_deferred WHERE event_type = 'mystery.op'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(deferred, 1, "admitted uninterpreted must be marked deferred");
}
```

- [ ] **Step 8: Run to verify they pass**

```bash
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" \
  cargo test -p cairn-node --test deferred_admission --test apply_remote_event
```

Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add db/020_apply_remote_event.sql crates/cairn-node/tests/
git commit -m "$(cat <<'EOF'
feat(#265): the clinical remote door admits an unclassifiable type

ADR-0056 decision 1. The door stored NOTHING for a type it could not classify,
so a carrier node acquired nothing past the first unknown-type event — the
§6.1 sneakernet failure the ADR exists to remove. It now admits verbatim,
projects nothing, confers nothing, and records an explicit marker.

Steps 4 and 5 are skipped EXPLICITLY rather than left to short-circuit on a
NULL classification, per the ADR's corollary. The deferred arm stores the
travelling attestation token unverified — without it re-adjudication would
have nothing to verify and a deferred event could never gain power, turning
admit-and-defer into a slower fail-closed.

The strict door is untouched: carry what you cannot author (decision 2).

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: The replay gate + owner-gate neutrality for a carried token

**Files:**
- Modify: `db/005_submit.sql:143-144` (`cairn_replay_eligible`), `db/005_submit.sql:267-285` (`cairn_suppression_author_ok`)
- Test: `crates/cairn-node/tests/deferred_admission.rs` (extend)

**Interfaces:**
- Consumes: `event_deferred` (Task 1), the door's marker write (Task 2).
- Produces: `cairn_replay_eligible(e event_log) → boolean` now returns FALSE for a deferred event. `cairn_suppression_author_ok(p_target, p_attester_key)` ignores a deferred target's `attester_key`.

**This task carries the slice's security test.** Design doc §4.2: `cairn_suppression_author_ok` reads the *target's* `event_log.attester_key` and unions it into the target's human-author set — the ADR-0043 owner-gate's whole basis. It is called by both doors on the target of any `targets_other_author` event, so unlike the projection apply fns it *is* reachable for a deferred row. Left alone, a hostile peer ships an unknown-type event carrying a forged token naming Mallory, the node admits it deferred, and Mallory is now inside that event's permitted-suppressor set on the strength of a token nothing checked. The function's own header promises the opposite: *"Wrong direction is over-refusal, never over-permission."*

- [ ] **Step 1: Write the failing tests**

Append to `crates/cairn-node/tests/deferred_admission.rs`:

```rust
/// A deferred event is invisible to replay. This is the ADR-0057 seam doing the job it was
/// built for: even a hand-run mid-upgrade `cairn_reproject` cannot grant power to an event
/// whose classification-gated checks have never been run.
#[tokio::test]
async fn reproject_does_not_touch_a_deferred_event() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let p = Uuid::now_v7();
    // Admit an event of an unclassified type that SHARES a name with a projected one only
    // after we classify it below — here it is still unknown, so it defers.
    let b = peer_event(&kid, p, UNKNOWN_TYPE, WALL_2026);
    let signed = sign(&b, &sk).unwrap();
    c.execute("SELECT apply_remote_event($1)", &[&signed.signed_bytes.to_vec()])
        .await
        .unwrap();

    let eligible: bool = c
        .query_one(
            "SELECT cairn_replay_eligible(el) FROM event_log el WHERE el.event_type = $1",
            &[&UNKNOWN_TYPE],
        )
        .await
        .unwrap()
        .get(0);
    assert!(!eligible, "a deferred event must never be replay-eligible");
}

/// THE SECURITY TEST (design doc §4.2). A deferred event's attestation token is CARRIED,
/// NOT VOUCHED — nothing verified it. It must not widen the ADR-0043 owner-gate, whose own
/// contract is "over-refusal, never over-permission".
///
/// Scenario: a hostile peer ships an unknown-type event carrying a token naming an enrolled
/// human. Before the fix, that human counted as an author of the target and could suppress
/// it. The gate must compute exactly as if no token had travelled.
#[tokio::test]
async fn a_carried_token_does_not_widen_the_owner_gate() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_agent, kid_agent, sk_human, kid_human) = setup(&c).await;
    let p = Uuid::now_v7();

    // A HUMAN-signed event of an unknown type, so its signer alone populates the target's
    // human-author set and the gate is genuinely restrictive (not the vacuous
    // "no human authors ⇒ anyone may suppress" branch).
    let b = peer_event(&kid_human, p, UNKNOWN_TYPE, WALL_2026);
    let target_id = b.event_id.clone();
    let signed = sign(&b, &sk_human).unwrap();
    // Carry a token from a DIFFERENT enrolled human (the agent key stands in for "some
    // other key the peer attached"); nothing verifies it on the deferred path.
    let att = cairn_event::sign_attestation(
        &cairn_event::event_address(&signed.signed_bytes),
        &kid_agent,
        &sk_agent,
    )
    .unwrap();
    c.execute(
        "SELECT apply_remote_event($1, $2, $3)",
        &[
            &signed.signed_bytes.to_vec(),
            &att.signed_bytes.to_vec(),
            &sk_agent.verifying_key().to_bytes().to_vec(),
        ],
    )
    .await
    .expect("a deferred event carrying a token is still admitted");

    // The carried token must NOT put its key inside the target's author set.
    let widened: bool = c
        .query_one(
            "SELECT cairn_suppression_author_ok($1::text::uuid, $2)",
            &[
                &target_id,
                &sk_agent.verifying_key().to_bytes().to_vec(),
            ],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        !widened,
        "a CARRIED (unverified) token on a deferred target must not widen the ADR-0043 \
         owner-gate — the gate must compute as if no token had travelled"
    );

    // Sanity: the event's genuine human signer is still an author of it.
    let genuine: bool = c
        .query_one(
            "SELECT cairn_suppression_author_ok($1::text::uuid, $2)",
            &[
                &target_id,
                &sk_human.verifying_key().to_bytes().to_vec(),
            ],
        )
        .await
        .unwrap()
        .get(0);
    assert!(genuine, "the target's real signer must still count as its author");
}
```

- [ ] **Step 2: Run to verify they fail**

```bash
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" \
  cargo test -p cairn-node --test deferred_admission reproject_does_not_touch \
  && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" \
  cargo test -p cairn-node --test deferred_admission a_carried_token
```

Expected: both FAIL — `cairn_replay_eligible` still returns TRUE, and the owner-gate returns TRUE for the carried key.

- [ ] **Step 3: Give `cairn_replay_eligible` its real body**

Replace `db/005_submit.sql`'s stub — both the comment and the function:

```sql
-- The #266 safety seam (ADR-0056 decision 4): cairn_reproject (db/039) routes every
-- candidate event through this predicate, so NO reprojection path — the loader's heal, the
-- `cairn-node reproject` CLI, or a hand-run mid-upgrade replay — can grant power to an
-- event whose classification-gated floor checks have never been run.
--
-- A deferred event is one the remote door admitted UNINTERPRETED (db/020, issue #265). Its
-- marker is deleted by cairn_readjudicate_deferred (db/043) only after the deferred gates
-- pass, so "no marker row" IS "adjudicated". An event that FAILS adjudication keeps its
-- marker (with the reason recorded) and stays powerless — never silently promoted.
--
-- The live-insert path needs no filter: an event being inserted through a door was
-- adjudicated by that door.
--
-- LANGUAGE sql (not plpgsql) deliberately: it inlines into cairn_reproject's per-type scan
-- as an anti-join rather than costing a function call per replayed event. event_deferred is
-- created in db/001 precisely so this body resolves at CREATE time.
CREATE OR REPLACE FUNCTION cairn_replay_eligible(e event_log)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT NOT EXISTS (SELECT 1 FROM event_deferred d WHERE d.event_id = e.event_id)
$$;
```

- [ ] **Step 4: Make `cairn_suppression_author_ok` neutral for a deferred target**

Replace the `human_authors` CTE's second arm:

```sql
    human_authors AS (
        SELECT t.signer_key_id AS kid FROM tgt t
        WHERE EXISTS (SELECT 1 FROM actor_event ae
                      WHERE ae.signing_key_id = t.signer_key_id
                        AND ae.op IN ('enroll','supersede')
                        AND ae.kind = 'human')
        UNION
        -- ADR-0056 (issue #265): a DEFERRED target's attester_key is CARRIED, NOT VOUCHED.
        -- The remote door stores the travelling token without verifying it (it cannot —
        -- the gate that verifies it is deferred with the interpretation), so unioning it
        -- here would let a hostile peer put any key it likes inside the target's
        -- human-author set by attaching a forged token to an unknown-type event. That is
        -- over-permission on the ADR-0043 floor, which this function's header forbids.
        --
        -- Note the fix is NEUTRAL, not merely stricter: for a deferred target signed by an
        -- agent, dropping this arm empties human_authors and the gate OPENS (the
        -- agent-advisory-is-dismissable rule). That is correct — an unverified token must
        -- not move the gate in EITHER direction. Promotion deletes the marker, after which
        -- the now-verified token counts normally.
        SELECT encode(t.attester_key, 'hex') FROM tgt t
        WHERE t.attester_key IS NOT NULL
          AND NOT EXISTS (SELECT 1 FROM event_deferred d WHERE d.event_id = p_target)
    )
```

- [ ] **Step 5: Run to verify they pass**

```bash
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" \
  cargo test -p cairn-node --test deferred_admission --test suppression_owner_gate \
  --test projection_registry
```

Expected: PASS. `suppression_owner_gate` is the pre-existing ADR-0043 suite and must be unaffected — no target in it carries a deferred marker.

- [ ] **Step 6: Commit**

```bash
git add db/005_submit.sql crates/cairn-node/tests/deferred_admission.rs
git commit -m "$(cat <<'EOF'
feat(#266): the replay gate goes live, and a carried token stops widening ADR-0043

cairn_replay_eligible was a constantly-TRUE stub ADR-0057 built for this slice;
it now means "carries no deferred marker", so no reprojection path can grant
power to an unadjudicated event.

The second change is a security fix the reader-audit surfaced.
cairn_suppression_author_ok reads the TARGET's attester_key and unions it into
the target's human-author set — and unlike the projection apply fns it IS
reachable for a deferred row. A hostile peer could ship an unknown-type event
carrying a forged token naming Mallory, and Mallory would land inside that
event's permitted-suppressor set on the strength of a token nothing checked.
The gate now computes as if no token had travelled; promotion restores it.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: `cairn_readjudicate_deferred()` — re-adjudicate, then promote

**Files:**
- Create: `db/043_deferred_readjudication.sql`
- Modify: `crates/cairn-event/src/schema_generation.rs:44` (`42` → `43`)
- Modify: `crates/cairn-node/src/db.rs::SCHEMA` (append `043`)
- Modify: `crates/cairn-sync/src/main.rs::SCHEMA` (append `043`)
- Test: `crates/cairn-node/tests/deferred_admission.rs` (extend)

**Interfaces:**
- Consumes: `event_deferred`, `cairn_body`, `cairn_attestation_ok`, `cairn_responsibility_bound`, `cairn_suppression_target_id`, `cairn_suppression_author_ok`, `actor_current`.
- Produces: `cairn_readjudicate_deferred() RETURNS TABLE(promoted_type text, promoted_count bigint)` — one row per event type that gained at least one promoted event. Task 5's loader consumes `promoted_type`.

- [ ] **Step 1: Write the failing tests**

Append to `crates/cairn-node/tests/deferred_admission.rs`:

```rust
/// Classification arrival PROMOTES a deferred event that passes the deferred gates: the
/// marker is deleted and it becomes replay-eligible. `note.added` is additive and targets
/// nobody, so all three gates are trivially satisfied once it is classified.
#[tokio::test]
async fn classification_promotes_a_passing_deferred_event() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let p = Uuid::now_v7();
    let b = peer_event(&kid, p, UNKNOWN_TYPE, WALL_2026);
    let signed = sign(&b, &sk).unwrap();
    c.execute("SELECT apply_remote_event($1)", &[&signed.signed_bytes.to_vec()])
        .await
        .unwrap();

    // The code-plane update lands: the type is now classified (a migration would do this).
    c.execute(
        "INSERT INTO event_type_class (event_type, mode, targets_other_author) \
         VALUES ($1, 'additive', FALSE) ON CONFLICT DO NOTHING",
        &[&UNKNOWN_TYPE],
    )
    .await
    .unwrap();

    let rows = c
        .query(
            "SELECT promoted_type, promoted_count FROM cairn_readjudicate_deferred()",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 1, "exactly one type should have been promoted");
    let ty: String = rows[0].get(0);
    let n: i64 = rows[0].get(1);
    assert_eq!(ty, UNKNOWN_TYPE);
    assert_eq!(n, 1);

    let still: i64 = c
        .query_one("SELECT count(*) FROM event_deferred", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(still, 0, "a promoted event's marker must be DELETED");

    let eligible: bool = c
        .query_one(
            "SELECT cairn_replay_eligible(el) FROM event_log el WHERE el.event_type = $1",
            &[&UNKNOWN_TYPE],
        )
        .await
        .unwrap()
        .get(0);
    assert!(eligible, "a promoted event must become replay-eligible");
}

/// ADR-0056 decision 4: an event that FAILS re-adjudication stays powerless and is flagged
/// legibly — never silently promoted. Here the type turns out to be SUPPRESSING, and the
/// event carries no attestation, so the deferred attestation gate refuses it.
#[tokio::test]
async fn failed_readjudication_stays_powerless_and_flagged() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let p = Uuid::now_v7();
    let b = peer_event(&kid, p, UNKNOWN_TYPE, WALL_2026);
    let signed = sign(&b, &sk).unwrap();
    // Admitted with NO attestation token — legal for a deferred event, since the gate that
    // would demand one is deferred with the interpretation.
    c.execute("SELECT apply_remote_event($1)", &[&signed.signed_bytes.to_vec()])
        .await
        .unwrap();

    // The code plane classifies it as SUPPRESSING — so the attestation gate now applies.
    c.execute(
        "INSERT INTO event_type_class (event_type, mode, targets_other_author) \
         VALUES ($1, 'suppressing', FALSE) ON CONFLICT DO NOTHING",
        &[&UNKNOWN_TYPE],
    )
    .await
    .unwrap();

    let rows = c
        .query(
            "SELECT promoted_type, promoted_count FROM cairn_readjudicate_deferred()",
            &[],
        )
        .await
        .unwrap();
    assert!(rows.is_empty(), "an un-attested suppress must NOT be promoted");

    let (kept, err): (i64, Option<String>) = {
        let r = c
            .query_one(
                "SELECT count(*)::bigint, max(adjudication_error) FROM event_deferred",
                &[],
            )
            .await
            .unwrap();
        (r.get(0), r.get(1))
    };
    assert_eq!(kept, 1, "a failing event keeps its marker — powerless");
    let err = err.expect("the failure reason must be recorded");
    assert!(
        err.contains("attestation"),
        "the flag must be legible; got: {err}"
    );

    let eligible: bool = c
        .query_one(
            "SELECT cairn_replay_eligible(el) FROM event_log el WHERE el.event_type = $1",
            &[&UNKNOWN_TYPE],
        )
        .await
        .unwrap()
        .get(0);
    assert!(!eligible, "a failed event must stay replay-ineligible");
}

/// The §4.1 trap, pinned: the travelling attestation token survives defer → promote. If the
/// door dropped it, this event could never be adjudicated and admit-and-defer would be a
/// slower fail-closed.
#[tokio::test]
async fn a_travelling_token_survives_defer_then_promote() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (_sk_a, _kid_a, sk_h, kid_h) = setup(&c).await;
    let p = Uuid::now_v7();
    // Signed by the HUMAN attester, so cairn_responsibility_bound is satisfied by the same
    // key that signs the token.
    let mut b = peer_event(&kid_h, p, UNKNOWN_TYPE, WALL_2026);
    b.contributors =
        serde_json::json!([{"actor_id": kid_h, "role": "authored",
                            "responsibility": {"held_by": kid_h}}]);
    let signed = sign(&b, &sk_h).unwrap();
    let att = cairn_event::sign_attestation(
        &cairn_event::event_address(&signed.signed_bytes),
        &kid_h,
        &sk_h,
    )
    .unwrap();
    c.execute(
        "SELECT apply_remote_event($1, $2, $3)",
        &[
            &signed.signed_bytes.to_vec(),
            &att.signed_bytes.to_vec(),
            &sk_h.verifying_key().to_bytes().to_vec(),
        ],
    )
    .await
    .unwrap();

    let stored: Option<Vec<u8>> = c
        .query_one(
            "SELECT attestation FROM event_log WHERE event_type = $1",
            &[&UNKNOWN_TYPE],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        stored.is_some(),
        "the travelling token must be STORED on the deferred path, or re-adjudication has \
         nothing to verify and the event can never gain power"
    );

    c.execute(
        "INSERT INTO event_type_class (event_type, mode, targets_other_author) \
         VALUES ($1, 'additive', FALSE) ON CONFLICT DO NOTHING",
        &[&UNKNOWN_TYPE],
    )
    .await
    .unwrap();
    let rows = c
        .query("SELECT promoted_type FROM cairn_readjudicate_deferred()", &[])
        .await
        .unwrap();
    assert_eq!(
        rows.len(),
        1,
        "the carried token must now VERIFY and promote the event"
    );
}
```

- [ ] **Step 2: Run to verify they fail**

```bash
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" \
  cargo test -p cairn-node --test deferred_admission
```

Expected: the three new tests FAIL — `function cairn_readjudicate_deferred() does not exist`.

- [ ] **Step 3: Create `db/043_deferred_readjudication.sql`**

```sql
-- db/043_deferred_readjudication.sql
-- Cairn — reclassification is RE-ADJUDICATION FIRST, backfill second (ADR-0056 decision 4,
-- issue #266).
--
-- WHAT: `cairn_readjudicate_deferred` — the pass that turns an admitted-uninterpreted event
-- (db/020, issue #265) into a fully-powered one, but only after re-running the floor checks
-- that classification gates.
--
-- WHY THE ORDER IS LOAD-BEARING. Admitting an event uninterpreted necessarily SKIPS every
-- refusal derived from its mode or its target relationship — in db/020 all three sit
-- downstream of the classification lookup:
--
--   * the suppressing⇒attestation gate,
--   * the overlay-target-exists refusal,
--   * the ADR-0043 cross-author-suppression refusal.
--
-- Those are DEFERRED WITH the interpretation, not waived by it. If classification arrival
-- only rebuilt projection rows, a deferred event would gain power having never passed the
-- gate that exists to bound it. Re-running them here, BEFORE cairn_reproject, is what makes
-- "no unattested suppression" hold at EVERY INSTANT rather than being violated-then-repaired.
--
-- The marker table itself lives in db/001 (next to event_log): db/005's LANGUAGE sql
-- predicates cairn_replay_eligible and cairn_suppression_author_ok read it, and SQL-language
-- bodies resolve table names at CREATE time.
--
-- connect_and_load_schema re-runs every migration each connect: everything below is
-- idempotent.

BEGIN;

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'cairn_node') THEN
        CREATE ROLE cairn_node NOLOGIN;
    END IF;
END $$;

-- Read-only for the runtime role: the `cairn-node deferred` listing and any future
-- operator surface. The pass itself is owner-only (see the REVOKE below) — it grants
-- power, so it belongs to the same privilege tier as cairn_reproject.
GRANT SELECT ON event_deferred TO cairn_node;

CREATE OR REPLACE FUNCTION cairn_readjudicate_deferred()
RETURNS TABLE(promoted_type text, promoted_count bigint)
LANGUAGE plpgsql
-- Pinned like every dynamic/dispatching function in this schema: the helper calls below
-- must never resolve into an attacker-shadowed schema, regardless of caller.
SET search_path = public
AS $$
DECLARE
    r          record;
    b          jsonb;
    v_bears    boolean;
    v_target   uuid;
    v_err      text;
    -- type → count of events promoted this run. A jsonb accumulator rather than a temp
    -- table: the deferred set is tiny by construction (it is empty on a healthy node) and
    -- this keeps the function free of any object a concurrent caller could collide on.
    v_promoted jsonb := '{}'::jsonb;
BEGIN
    FOR r IN
        SELECT d.event_id, d.event_type, el.signed_bytes, el.content_address,
               el.attestation, el.attester_key, c.mode, c.targets_other_author
          FROM event_deferred d
          JOIN event_log el       ON el.event_id   = d.event_id
          -- Only rows whose type this node can NOW classify are candidates. A still-unknown
          -- type simply stays deferred, untouched and unflagged.
          JOIN event_type_class c ON c.event_type  = d.event_type
         -- HLC (causal) order, so a deferred overlay is adjudicated AFTER the deferred
         -- target it points at — the target is promoted first, and the owner-gate below
         -- then sees its now-vouched attester_key rather than the carried one. Collation-
         -- independent on node_origin (ADR-0045): every node must pick the same order.
         ORDER BY el.hlc_wall, el.hlc_counter, el.node_origin COLLATE "C"
    LOOP
        v_err := NULL;
        -- Per-row subtransaction. A failure here must NEVER propagate: this pass runs
        -- inside connect_and_load_schema, and a raise would abort the whole schema load and
        -- wedge the node on one bad event — precisely the failure mode ADR-0056 exists to
        -- remove. The refusal is captured and recorded instead.
        BEGIN
            -- Re-derive the envelope from the SIGNED BYTES, never from the projection
            -- columns: the predicates below must see exactly what the door saw, and a
            -- reconstruction from columns would drift from db/020 on the next edit.
            b := cairn_body(r.signed_bytes);
            IF b IS NULL THEN
                RAISE EXCEPTION 'stored signed bytes no longer parse';
            END IF;

            -- Deferred gate 1 — the suppressing⇒attestation gate (db/020 step 4).
            v_bears := EXISTS (
                SELECT 1 FROM jsonb_array_elements(b -> 'contributors') AS e
                WHERE e ? 'responsibility');
            IF r.mode = 'suppressing' OR v_bears THEN
                IF r.attestation IS NULL OR r.attester_key IS NULL THEN
                    RAISE EXCEPTION
                        '% requires attestation (no token travelled with the event) — un-vouched suppress/responsibility refused',
                        r.event_type;
                END IF;
                IF NOT cairn_attestation_ok(r.attestation, r.content_address, r.attester_key) THEN
                    RAISE EXCEPTION 'attestation token invalid or not bound to this event';
                END IF;
                IF NOT EXISTS (SELECT 1 FROM actor_current
                               WHERE signing_key_id = encode(r.attester_key,'hex')
                                 AND kind = 'human') THEN
                    RAISE EXCEPTION 'attester is not an enrolled human actor (forged human author refused)';
                END IF;
                IF NOT cairn_responsibility_bound(b, r.attester_key) THEN
                    RAISE EXCEPTION 'a contributor claims responsibility for an actor other than the verified attester (issue #195)';
                END IF;
            END IF;

            -- Deferred gates 2 and 3 — overlay-target-exists and the ADR-0043 owner-gate
            -- (db/020 step 5). Gate 2 can legitimately fail on a target still in flight from
            -- another peer; that is why the loader runs this pass on EVERY connect, not only
            -- on a generation change (see connect_and_load_schema).
            IF r.targets_other_author THEN
                v_target := cairn_suppression_target_id(b);
                IF NOT EXISTS (SELECT 1 FROM event_log WHERE event_id = v_target) THEN
                    RAISE EXCEPTION 'overlay targets unknown event %', v_target;
                END IF;
                IF r.mode = 'suppressing'
                   AND NOT cairn_suppression_author_ok(v_target, r.attester_key) THEN
                    RAISE EXCEPTION
                        'cross-author suppression refused — a suppress of another human''s event may not be admitted; disagreement is additive. (ADR-0043)';
                END IF;
            END IF;
        EXCEPTION WHEN OTHERS THEN
            v_err := SQLERRM;
        END;

        IF v_err IS NULL THEN
            -- PROMOTED. Deleting the marker is the whole promotion: cairn_replay_eligible
            -- reads its absence, so the event becomes visible to the reprojection the
            -- caller runs next.
            DELETE FROM event_deferred WHERE event_id = r.event_id;
            v_promoted := jsonb_set(
                v_promoted, ARRAY[r.event_type],
                to_jsonb(COALESCE((v_promoted ->> r.event_type)::bigint, 0) + 1));
        ELSE
            -- STILL POWERLESS, and now flagged legibly (decision 4). The marker stays, so
            -- the event remains replay-ineligible and is retried on the next connect.
            UPDATE event_deferred
               SET adjudication_error = v_err,
                   last_attempt_at    = clock_timestamp()
             WHERE event_id = r.event_id;
        END IF;
    END LOOP;

    RETURN QUERY
        SELECT k, v::bigint FROM jsonb_each_text(v_promoted) AS t(k, v);
END;
$$;

-- Owner-only, exactly like cairn_reproject (db/039): this function GRANTS POWER to events
-- that were admitted without it. The loader and CLI connect with owner privileges; the
-- runtime role must not be able to promote anything.
REVOKE EXECUTE ON FUNCTION cairn_readjudicate_deferred() FROM PUBLIC;

COMMIT;
```

- [ ] **Step 4: Register the migration in both loaders and bump the generation**

In `crates/cairn-event/src/schema_generation.rs`, change `pub const SCHEMA_GENERATION: i32 = 42;` to `43`.

Append to `crates/cairn-node/src/db.rs`'s `SCHEMA` array (after the `042_medication_coding_overlay` entry):

```rust
    // db/043 (ADR-0056 decision 4 / #266): cairn_readjudicate_deferred — the pass that
    // re-runs the classification-gated floor checks before power is granted. In BOTH lists:
    // cairn-sync loads db/020, whose door writes the event_deferred marker, so a cairn-sync
    // database missing this file would accumulate deferred rows nothing could ever promote
    // (the #284 drift hazard, made concrete).
    (
        "043_deferred_readjudication",
        include_str!("../../../db/043_deferred_readjudication.sql"),
    ),
```

Append the identical entry to `crates/cairn-sync/src/main.rs`'s `SCHEMA` array.

- [ ] **Step 5: Run to verify they pass**

```bash
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" \
  cargo test -p cairn-node --test deferred_admission
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" \
  cargo test -p cairn-event
```

Expected: PASS, including `cairn-event`'s fs-derived guard that `SCHEMA_GENERATION == newest db/*.sql prefix`, and `cairn-node`'s unit test that the full list embeds that newest file.

- [ ] **Step 6: Commit**

```bash
git add db/043_deferred_readjudication.sql crates/cairn-event/src/schema_generation.rs \
        crates/cairn-node/src/db.rs crates/cairn-sync/src/main.rs \
        crates/cairn-node/tests/deferred_admission.rs
git commit -m "$(cat <<'EOF'
feat(#266): cairn_readjudicate_deferred — re-adjudicate, then promote

ADR-0056 decision 4's load-bearing half. Admitting an event uninterpreted
skips the attestation gate, the overlay-target-exists check and the ADR-0043
owner-gate; this pass re-runs all three before the marker is deleted, so a
reprojection can never grant power that never passed a gate.

Failure is captured per row, never raised: the pass runs inside
connect_and_load_schema, and a raise would wedge the node on one bad event.
An event that fails stays powerless with the reason recorded.

Registered in BOTH loader lists — cairn-sync loads db/020, whose door writes
the marker, so a cairn-sync database without this file would accumulate
deferred rows nothing could promote (#284's hazard, made concrete).

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 5: The loader runs the pass every connect

**Files:**
- Modify: `crates/cairn-node/src/db.rs::connect_and_load_schema` (the block between the migration replay and the `node_schema` stamp)
- Test: `crates/cairn-node/tests/deferred_admission.rs` (extend)

**Interfaces:**
- Consumes: `cairn_readjudicate_deferred() → TABLE(promoted_type text, promoted_count bigint)` (Task 4).
- Produces: no new API — `connect_and_load_schema` gains the behaviour.

**Why every connect, not only on a generation change:** classification only arrives with a code-plane update, so a generation change is the right trigger for *reclassification*. But re-adjudication can fail for a reason that later resolves with no code change — the sharp case is `overlay targets unknown event`, where the target is still in flight from another peer. Under a generation-only trigger, that event stays powerless until the next code update, potentially months. `event_deferred` is empty on a healthy node, so the pass costs one indexed probe.

- [ ] **Step 1: Write the failing test**

Append to `crates/cairn-node/tests/deferred_admission.rs`:

```rust
/// The loader runs the pass on EVERY connect and reprojects what it promotes. Pinned with a
/// type that has a real projection, so "promoted" is observable as a projection row rather
/// than only as a deleted marker.
#[tokio::test]
async fn connect_promotes_and_reprojects_a_deferred_event() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let p = Uuid::now_v7();

    // Admit a patient.created event as if its type were unknown: de-classify the type,
    // apply, then restore. This gives a deferred event of a type that DOES have a
    // registered projection (patient_chart), which is what makes the reproject observable.
    // Deleting the class row is safe here because the projection registration guard
    // validates on INSERT to cairn_projection_apply, not on delete from event_type_class.
    c.execute(
        "DELETE FROM event_type_class WHERE event_type = 'patient.created'",
        &[],
    )
    .await
    .unwrap();
    let mut b = peer_event(&kid, p, "patient.created", WALL_2026);
    b.payload = serde_json::json!({"name": "Deferred Then Promoted"});
    let signed = sign(&b, &sk).unwrap();
    c.execute("SELECT apply_remote_event($1)", &[&signed.signed_bytes.to_vec()])
        .await
        .unwrap();

    let deferred: i64 = c
        .query_one("SELECT count(*) FROM event_deferred", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(deferred, 1, "precondition: the event is deferred");
    let projected: i64 = c
        .query_one(
            "SELECT count(*) FROM patient_chart WHERE patient_id = $1",
            &[&p],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(projected, 0, "a deferred event must project nothing");

    // A fresh connect replays every migration (restoring the class row) and must then
    // re-adjudicate and reproject.
    drop(c);
    let c2 = db::connect_and_load_schema(&base).await.unwrap();

    let deferred: i64 = c2
        .query_one("SELECT count(*) FROM event_deferred", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(deferred, 0, "connect must promote the now-classified event");
    let name: Option<String> = c2
        .query_opt(
            "SELECT name FROM patient_chart WHERE patient_id = $1",
            &[&p],
        )
        .await
        .unwrap()
        .map(|r| r.get(0));
    assert_eq!(
        name.as_deref(),
        Some("Deferred Then Promoted"),
        "connect must reproject what it promoted"
    );
}
```

- [ ] **Step 2: Run to verify it fails**

```bash
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" \
  cargo test -p cairn-node --test deferred_admission connect_promotes
```

Expected: FAIL — the marker is still present after reconnect (the loader never calls the pass).

- [ ] **Step 3: Add the pass to `connect_and_load_schema`**

Replace the existing generation-gated heal block with:

```rust
    // ADR-0056 decision 4 (#266): RE-ADJUDICATE FIRST, REPROJECT SECOND.
    //
    // A deferred event (db/020 admitted it uninterpreted, #265) skipped the floor checks
    // that classification gates — the attestation gate, overlay-target-exists, and the
    // ADR-0043 owner-gate. Those are deferred WITH the interpretation, not waived by it, so
    // they are re-run here BEFORE anything reprojects. A reprojection that merely rebuilt
    // rows would grant power that never passed a gate.
    //
    // WHY EVERY CONNECT, not only on a generation change: classification itself only
    // arrives with a code-plane update, so a generation change is the right trigger for
    // RECLASSIFICATION. But re-adjudication can fail for a reason that later resolves with
    // no code change at all — `overlay targets unknown event`, where the target is still in
    // flight from another peer. Gated on the generation, such an event would stay powerless
    // until the next code update, potentially months. event_deferred is empty on a healthy
    // node, so this costs one indexed probe per connect.
    //
    // Ordered before the stamp for the same reason the heal below is: a failure here must
    // leave the recorded generation at its OLD value so the next connect retries.
    let promoted: Vec<String> = client
        .query("SELECT promoted_type FROM cairn_readjudicate_deferred()", &[])
        .await
        .map_err(|e| anyhow::anyhow!("re-adjudicating deferred events: {e}"))?
        .iter()
        .map(|r| r.get(0))
        .collect();

    // #208/ADR-0057: heal replay on generation CHANGE only, and BEFORE the stamp below. New
    // projection capability (and any projection-logic fix) arrives only via a code-plane
    // update — i.e. a generation change — so an unchanged generation means there is nothing
    // to heal and the connect path does zero reprojection work. An UNKNOWN recorded
    // generation (fresh DB: free no-op; hand-built rig: converges once) errs toward healing.
    // Runs inside SCHEMA_LOAD_LOCK: concurrent loaders serialize, and the second sees the
    // stamped generation.
    //
    // Ordered BEFORE the stamp deliberately: if the heal query below errors, the stamp never
    // runs, so the recorded generation stays at its OLD (pre-upgrade) value and the `?`
    // propagates the failure up to the caller. The NEXT connect attempt then sees the same
    // stale `recorded`, so it retries the FULL replay-then-heal — exactly the loud,
    // self-retrying failure mode a broken migration file already has in this loader.
    // Stamp-then-heal would invert this: a heal failure AFTER the stamp leaves the
    // generation already advanced, so the next connect reads `recorded == embedded`, skips
    // the heal entirely, and the projections stay SILENTLY stale — the worst failure mode,
    // and the reason this order is load-bearing, not cosmetic.
    if recorded != Some(embedded) {
        // A full heal already covers every type the pass just promoted, so no targeted
        // replay is needed on this path.
        client
            .execute(
                "SELECT count(*) FROM cairn_reproject('', false, 'loader')",
                &[],
            )
            .await
            .map_err(|e| anyhow::anyhow!("post-upgrade heal replay: {e}"))?;
    } else {
        // No generation change, but the pass promoted something (the target-arrived-later
        // case). Heal ONLY those types — an exact event_type is a valid prefix for
        // cairn_reproject's LIKE scan. Heal mode, never rebuild: a narrow rebuild would hit
        // db/039's refusal on any projection table shared with an out-of-prefix type.
        for ty in &promoted {
            client
                .execute(
                    "SELECT count(*) FROM cairn_reproject($1, false, 'readjudicate')",
                    &[ty],
                )
                .await
                .map_err(|e| anyhow::anyhow!("post-readjudication heal replay for {ty}: {e}"))?;
        }
    }
```

- [ ] **Step 4: Run to verify it passes**

```bash
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" \
  cargo test -p cairn-node --test deferred_admission
```

Expected: PASS (all tests in the file).

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-node/src/db.rs crates/cairn-node/tests/deferred_admission.rs
git commit -m "$(cat <<'EOF'
feat(#266): the loader re-adjudicates every connect, then reprojects

Re-adjudication runs on every connect rather than only on a generation change.
Classification arrives only with a code-plane update, but re-adjudication can
FAIL for a reason that resolves without one — an overlay whose target was
still in flight. Generation-gated, that event would stay powerless until the
next code update. event_deferred is empty on a healthy node, so the pass costs
one indexed probe.

A generation change reprojects everything as before; otherwise only the types
the pass actually promoted are healed, in heal mode (a narrow rebuild would
hit db/039's shared-table refusal).

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 6: `cairn-node deferred` — the operator listing

**Files:**
- Modify: `crates/cairn-node/src/main.rs` (the `Cmd` enum, after `Reproject`; and the `match cli.cmd` arm)
- Test: `crates/cairn-node/tests/deferred_admission.rs` (extend — a query-shape test, mirroring how the repo tests other read-only CLI surfaces)

**Interfaces:**
- Consumes: `event_deferred` (Task 1).
- Produces: `Cmd::Deferred` — no arguments.

- [ ] **Step 1: Write the failing test**

Append to `crates/cairn-node/tests/deferred_admission.rs`:

```rust
/// The listing query the `cairn-node deferred` subcommand runs. Pinned here so a schema
/// change that breaks the operator surface fails a test rather than only failing in the
/// field — decision 4's "flagged legibly" is only legible if something reads it.
#[tokio::test]
async fn deferred_listing_query_returns_the_operator_columns() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let p = Uuid::now_v7();
    let b = peer_event(&kid, p, UNKNOWN_TYPE, WALL_2026);
    let signed = sign(&b, &sk).unwrap();
    c.execute("SELECT apply_remote_event($1)", &[&signed.signed_bytes.to_vec()])
        .await
        .unwrap();

    let rows = c
        .query(
            "SELECT event_id::text, event_type, admitted_at, \
                    COALESCE(adjudication_error, '(not yet re-adjudicated)') \
               FROM event_deferred ORDER BY admitted_at",
            &[],
        )
        .await
        .expect("the deferred listing query must run");
    assert_eq!(rows.len(), 1);
    let ty: String = rows[0].get(1);
    let reason: String = rows[0].get(3);
    assert_eq!(ty, UNKNOWN_TYPE);
    assert_eq!(reason, "(not yet re-adjudicated)");
}
```

- [ ] **Step 2: Run to verify it fails**

```bash
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" \
  cargo test -p cairn-node --test deferred_admission deferred_listing
```

Expected: PASS already if Tasks 1-2 landed (the query is pure SQL over an existing table). That is fine and expected — this test guards the *query the CLI depends on*, and Step 3 adds the CLI that uses it. If it fails, the table or door is wrong; fix that before proceeding.

- [ ] **Step 3: Add the subcommand**

In `crates/cairn-node/src/main.rs`, add to the `Cmd` enum immediately after the `Reproject { .. }` variant:

```rust
    /// List events this node admitted UNINTERPRETED — stored verbatim but holding no
    /// power because it has no code that classifies their type (ADR-0056 decision 1).
    /// A row with a reason has been re-adjudicated and REFUSED; it stays powerless
    /// until the refusal resolves. A healthy node lists nothing.
    Deferred,
```

and the matching arm at the end of the `match cli.cmd` block:

```rust
        Cmd::Deferred => {
            let db = cairn_node::db::connect_and_load_schema(&cli.conn).await?;
            let rows = db
                .query(
                    "SELECT event_id::text, event_type, admitted_at, \
                            COALESCE(adjudication_error, '(not yet re-adjudicated)') \
                       FROM event_deferred ORDER BY admitted_at",
                    &[],
                )
                .await?;
            if rows.is_empty() {
                println!("no deferred events — every admitted event's type is classified");
            }
            for r in &rows {
                let id: String = r.get(0);
                let ty: String = r.get(1);
                let at: std::time::SystemTime = r.get(2);
                let reason: String = r.get(3);
                let at: chrono::DateTime<chrono::Utc> = at.into();
                println!("{id}  {ty:<40}  {}  {reason}", at.to_rfc3339());
            }
        }
```

If `chrono` is not already a `cairn-node` dependency, print the timestamp with `{at:?}` instead of adding a dependency — this slice adds none. Check first:

```bash
grep -n "^chrono" crates/cairn-node/Cargo.toml
```

- [ ] **Step 4: Verify it builds and the help text reads correctly**

```bash
cargo build -p cairn-node
./target/debug/cairn-node deferred --help
```

Expected: builds clean; help shows the description.

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-node/src/main.rs crates/cairn-node/tests/deferred_admission.rs
git commit -m "$(cat <<'EOF'
feat(#265): `cairn-node deferred` lists admitted-uninterpreted events

ADR-0056 decision 4 requires a failed re-adjudication to be "flagged legibly".
A flag nothing reads is not legible, so this is the operator surface for it —
mirroring `cairn-sync quarantine`. A healthy node lists nothing.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 7: The SQL mirror, the spec prose, and the docs

**Files:**
- Create: `db/tests/043_deferred_readjudication_test.sql`
- Modify: `docs/spec/sync.md` (§6.3 and §6.5 — the "current limits" prose)
- Modify: `docs/HANDOVER.md`, `docs/ROADMAP.md`

**Interfaces:**
- Consumes: everything above.
- Produces: nothing code-facing.

- [ ] **Step 1: Write the pure-SQL mirror**

CI runs `scripts/run-db-sql-tests.sh` over `db/tests/*.sql` (since PR #251), so the floor is pinned without Rust. Create `db/tests/043_deferred_readjudication_test.sql` following the existing style in `db/tests/039_projection_registry_test.sql` (a `DO $$ ... RAISE EXCEPTION ... $$` block per assertion):

```sql
-- db/tests/043_deferred_readjudication_test.sql
-- Pure-SQL mirror of the ADR-0056 floor (issues #265/#266). No Rust, no signing: these
-- assertions cover the parts of the contract that are pure schema — the marker table's
-- existence, the replay gate reading it, and the classified-before-projected guard.
-- Signature-dependent behaviour (the door admitting a real signed event) is covered by
-- crates/cairn-node/tests/deferred_admission.rs, which can sign.

-- T1: the marker table exists with its primary key on event_id.
DO $$
BEGIN
    IF to_regclass('public.event_deferred') IS NULL THEN
        RAISE EXCEPTION 'event_deferred is missing — ADR-0056 has no explicit deferred state';
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_index i
        JOIN pg_class c ON c.oid = i.indrelid
        WHERE c.relname = 'event_deferred' AND i.indisprimary
    ) THEN
        RAISE EXCEPTION 'event_deferred has no primary key — the marker must be 1:1 with event_log';
    END IF;
END $$;

-- T2: the projection registry refuses an unclassified event type (fail closed). Rolled
-- back so the registry is untouched.
DO $$
DECLARE v_ok boolean := false;
BEGIN
    BEGIN
        INSERT INTO cairn_projection_apply (event_type, apply_fn, projection_tables)
        VALUES ('unclassified.sql.mirror', 'patient_chart_apply', ARRAY['patient_chart']);
    EXCEPTION WHEN OTHERS THEN
        IF SQLERRM LIKE '%not classified in event_type_class%' THEN
            v_ok := true;
        ELSE
            RAISE EXCEPTION 'wrong refusal for an unclassified projection registration: %', SQLERRM;
        END IF;
    END;
    IF NOT v_ok THEN
        RAISE EXCEPTION 'cairn_projection_apply accepted an UNCLASSIFIED event_type — the dispatcher could project a deferred event';
    END IF;
END $$;

-- T3: cairn_replay_eligible reads the marker. Insert a synthetic event_log row + marker,
-- assert ineligible, delete the marker, assert eligible. Rolled back at the end.
BEGIN;
DO $$
DECLARE
    v_id  uuid := uuidv7();
    v_sb  bytea;
    v_elig boolean;
BEGIN
    v_sb := ('replay-gate-' || v_id::text)::bytea;
    INSERT INTO event_log (event_id, patient_id, event_type, schema_version,
        hlc_wall, hlc_counter, node_origin, signed_bytes, content_address,
        body, contributors, signer_key_id, plaintext_twin)
    VALUES (v_id, v_id, 'replay.gate.probe', 'test-1',
        (extract(epoch from now()) * 1000)::bigint, 0, 'test-node', v_sb,
        '\x1220'::bytea || digest(v_sb, 'sha256'),
        '{}'::jsonb, '[]'::jsonb, 'test-key', 'probe');
    INSERT INTO event_deferred (event_id, event_type) VALUES (v_id, 'replay.gate.probe');

    SELECT cairn_replay_eligible(el) INTO v_elig FROM event_log el WHERE el.event_id = v_id;
    IF v_elig THEN
        RAISE EXCEPTION 'cairn_replay_eligible returned TRUE for a DEFERRED event — reprojection could grant unadjudicated power';
    END IF;

    DELETE FROM event_deferred WHERE event_id = v_id;
    SELECT cairn_replay_eligible(el) INTO v_elig FROM event_log el WHERE el.event_id = v_id;
    IF NOT v_elig THEN
        RAISE EXCEPTION 'cairn_replay_eligible returned FALSE for a NON-deferred event — replay would skip healthy events';
    END IF;
END $$;
ROLLBACK;
```

- [ ] **Step 2: Run the SQL mirror**

```bash
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" \
  ./scripts/run-db-sql-tests.sh
```

Expected: all files pass, including the new one. (Read the script first to confirm the env var it reads — match it.)

- [ ] **Step 3: Correct the spec prose**

`docs/spec/sync.md` §6.5 states the lossless-forwarding invariant and §6.3 the failure modes. Both currently carry the honest "current limits" wording ADR-0056 added. Update **only** what this slice made true:

- §6.5: the invariant now holds for unknown *types* as well as fields; name the admitted-uninterpreted state and the `event_deferred` marker; state that power is granted at reclassification via re-adjudication-then-reprojection.
- §6.3: the unknown-type row is no longer a refusal. Leave the *remaining* honest limits standing and unmodified — door refusals are still not penned (#267), a frozen clinical watermark still exits success (#270), and the node plane still skips-and-advances (#268) and still fails closed on an unknown node type (the new issue filed for db/007). Do not let this slice's prose over-claim.

Read the two sections first and edit surgically; do not restructure them.

- [ ] **Step 4: Run the docs build**

```bash
uv run --with-requirements docs/requirements.txt -- mkdocs build
```

Expected: builds clean. Never commit the generated `site/`.

- [ ] **Step 5: Full-workspace verification**

```bash
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" \
CAIRN_TEST_PG2="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test2" \
CAIRN_TEST_PG3="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test3" \
  cargo test --workspace
echo "cargo exit: $?"
cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings
```

Expected: all green. **Do not pipe `cargo test` into `tail`** — it masks the exit code (a recorded lesson). The whole-workspace run is the only thing that catches guard-scope gaps.

- [ ] **Step 6: Update HANDOVER and ROADMAP**

Add a ROADMAP slice entry (next number after Slice 57) covering: what shipped, the two design decisions that were not forced by the ADR (every-connect re-adjudication; the registration guard), the security finding in `cairn_suppression_author_ok`, and what is explicitly NOT done (#267/#268/#269/#270 and the node-plane issue). Rewrite HANDOVER's ⇒ NEXT block so the next session's candidates are current. Prune both under 500 lines.

- [ ] **Step 7: Commit and open the PR**

```bash
git add db/tests/043_deferred_readjudication_test.sql docs/
git commit -m "docs(ADR-0056): SQL mirror, sync.md §6.3/§6.5 currency, ROADMAP + HANDOVER"
git push -u origin feat/adr-0056-admit-uninterpreted-floor-265-266
gh pr create --base main --title "feat(ADR-0056): admit uninterpreted, re-adjudicate before power (#265, #266)" --body "..."
```

The PR body must link `Closes #265` and `Closes #266`, reference the new node-plane issue as explicitly out of scope, and name the security finding so a reviewer looks at it first.

---

## Self-Review

**Spec coverage:** design §3 (marker table) → Task 1. §4 (door admits) → Task 2. §4.1 (travelling token) → Task 2 Step 5 + Task 4's round-trip test. §4.2 (the reader audit and its one broken reader) → Task 3. §5.1 (replay gate) → Task 3. §5.2 (the pass) → Task 4. §5.3 (when it runs) → Task 5. §5.4 (registration guard) → Task 1. §6 (legibility) → Task 6. §7 (testing) → every task's Step 1, with the security test in Task 3 and the token round-trip in Task 4. §8 (scope) → the node-plane issue is filed; #267-#270 are named as out of scope in Task 7's PR body. §9 (paper-parity) → the plan header.

**Placeholder scan:** no TBD/TODO. Two steps deliberately say "read the file first" rather than quoting content — Task 7 Step 3 (spec prose, which must be edited surgically against text not reproduced here) and Task 7 Step 6 (ROADMAP/HANDOVER, whose content depends on what actually happened during implementation). Both name exactly what must change and what must NOT be over-claimed.

**Type consistency:** `cairn_readjudicate_deferred()` returns `TABLE(promoted_type text, promoted_count bigint)` in Task 4's SQL, Task 4's tests, and Task 5's loader query — the OUT parameter names are deliberately *not* `event_type`, which would collide with `event_deferred.event_type` inside the function body and raise an ambiguous-reference error. `cairn_replay_eligible(e event_log) → boolean` keeps its existing signature. `event_deferred`'s five columns are identical in Task 1's DDL, Task 1's shape test, Task 6's listing query, and Task 7's SQL mirror.
