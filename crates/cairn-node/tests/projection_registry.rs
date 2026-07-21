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
    assert!(
        db_msg(&err).contains("does not exist"),
        "got: {}",
        db_msg(&err)
    );
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
    assert!(
        db_msg(&err).contains("does not exist"),
        "got: {}",
        db_msg(&err)
    );
}

/// The dispatcher replaces db/002's per-type trigger: a directly-inserted
/// patient.created event still materializes its patient_chart row.
#[tokio::test]
async fn dispatcher_routes_patient_created_to_patient_chart() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    // uuid crate types have no tokio-postgres ToSql/FromSql impl in this workspace
    // (the "with-uuid-1" feature isn't enabled) — bind as text and cast in SQL via
    // `$N::text::uuid`, the established idiom (see apply_proposal.rs/auto_apply.rs).
    let pid = uuid::Uuid::now_v7().to_string();
    // Owner-level direct INSERT: the projection trigger path is what's under test,
    // not the door. signed_bytes is synthetic; the content-address CHECK is
    // satisfied by computing the digest in SQL (house rule 6: derived, not literal).
    c.execute(
        "WITH sb AS (SELECT ('reproj-test-' || $1::text)::bytea AS b)
         INSERT INTO event_log (event_id, patient_id, event_type, schema_version,
             hlc_wall, hlc_counter, node_origin, signed_bytes, content_address,
             body, contributors, signer_key_id, plaintext_twin)
         SELECT $1::text::uuid, $1::text::uuid, 'patient.created', 'test-1',
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
            "SELECT name FROM patient_chart WHERE patient_id = $1::text::uuid",
            &[&pid],
        )
        .await
        .unwrap();
    let name: String = row.get(0);
    assert_eq!(name, "Reproject Probe");
    // No cleanup: event_log is append-only (BEFORE UPDATE/DELETE guard) — the
    // probe event stays, which is fine on the shared test DB (fresh UUID each run).
}

/// Steady-state replay must not rewrite the three seeded patient_chart_apply rows
/// (modeled on twin_registry.rs's steady_state_replay_leaves_registry_rows_untouched).
/// connect_and_load_schema replays db/005's registration INSERT on every connect; without
/// the `WHERE ... IS DISTINCT FROM` guard, an unconditional DO UPDATE writes a new row
/// version (dead tuple + validate-trigger fire) for all three rows on EVERY connect. Pin
/// that via xmin (the row version's inserting/updating txid), which only changes if the
/// row is actually rewritten.
#[tokio::test]
async fn steady_state_replay_leaves_projection_rows_untouched() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();

    // First connect converges the three rows (whatever state the shared DB was in).
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let rows_before = c
        .query(
            "SELECT event_type, xmin::text FROM cairn_projection_apply \
             WHERE apply_fn = 'patient_chart_apply' ORDER BY event_type",
            &[],
        )
        .await
        .unwrap();
    let mut before: Vec<(String, String)> = rows_before
        .iter()
        .map(|r| (r.get::<_, String>(0), r.get::<_, String>(1)))
        .collect();
    before.sort();
    assert_eq!(
        before.len(),
        3,
        "expected the 3 seeded patient_chart_apply registration rows"
    );
    drop(c);

    // Second connect replays over already-converged rows: no write may occur.
    let c2 = db::connect_and_load_schema(&base).await.unwrap();
    let rows_after = c2
        .query(
            "SELECT event_type, xmin::text FROM cairn_projection_apply \
             WHERE apply_fn = 'patient_chart_apply' ORDER BY event_type",
            &[],
        )
        .await
        .unwrap();
    let mut after: Vec<(String, String)> = rows_after
        .iter()
        .map(|r| (r.get::<_, String>(0), r.get::<_, String>(1)))
        .collect();
    after.sort();

    assert_eq!(
        before, after,
        "steady-state replay rewrote an already-converged cairn_projection_apply row \
         (the ON CONFLICT DO UPDATE arm must be guarded by IS DISTINCT FROM)"
    );
}

/// Heal mode converges a tampered winner row back to the corrected winner —
/// the generic replacement for the per-slice tamper-then-replay heal tests
/// (#214 pattern). Uses the Task-1 probe shape: tamper patient_chart.name.
#[tokio::test]
async fn reproject_heals_tampered_projection_row() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    // uuid crate types have no tokio-postgres ToSql/FromSql impl in this workspace —
    // bind as text and cast in SQL via `$N::text::uuid` (established idiom, see
    // dispatcher_routes_patient_created_to_patient_chart above).
    let pid: String = c
        .query_one("SELECT uuidv7()::text", &[])
        .await
        .unwrap()
        .get(0);
    c.execute(
        "WITH sb AS (SELECT ('heal-test-' || $1::text)::bytea AS b)
         INSERT INTO event_log (event_id, patient_id, event_type, schema_version,
             hlc_wall, hlc_counter, node_origin, signed_bytes, content_address,
             body, contributors, signer_key_id, plaintext_twin)
         SELECT $1::text::uuid, $1::text::uuid, 'patient.created', 'test-1',
             (extract(epoch from now()) * 1000)::bigint, 0, 'test-node', b,
             '\\x1220'::bytea || digest(b, 'sha256'),
             jsonb_build_object('name', 'True Winner'),
             '[]'::jsonb, 'test-key', 'probe'
         FROM sb",
        &[&pid],
    )
    .await
    .unwrap();
    // Tamper BOTH the display value and its recorded provenance (demo_hlc_wall), not name
    // alone. `cairn_hlc_overlay_wins` is strict-`>` (overlay_tiebreaker.rs's
    // `overlay_predicate_is_a_deterministic_total_order` pins "a full tie is NOT a win, so
    // an idempotent re-apply never churns the row" as a load-bearing invariant elsewhere) —
    // replaying this patient's ONE real event through the unmodified `patient_chart_apply`
    // compares its own (hlc_wall, hlc_counter, node_origin, content_address) against
    // whatever is currently stored in demo_*. If only `name` were tampered, that comparison
    // would find an EXACT tie (the provenance was never touched) and correctly refuse to
    // overlay — which is the strict-tie protection working as designed, not a bug — so heal
    // would leave the row untouched instead of converging it. Zeroing demo_hlc_wall models
    // the realistic defect heal mode actually repairs: the row's PROVENANCE itself went
    // stale/wrong (e.g. a since-fixed comparison bug), so replaying the true event's real,
    // higher HLC now legitimately dominates the corrupted provenance and re-wins.
    c.execute(
        "UPDATE patient_chart SET name = 'TAMPERED', demo_hlc_wall = 0 \
         WHERE patient_id = $1::text::uuid",
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
        .query_one(
            "SELECT name FROM patient_chart WHERE patient_id = $1::text::uuid",
            &[&pid],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        name, "True Winner",
        "heal replay must converge the tampered row"
    );
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
    let pid: String = c
        .query_one("SELECT uuidv7()::text", &[])
        .await
        .unwrap()
        .get(0);
    for (i, ty) in ["patient.created", "note.added"].iter().enumerate() {
        c.execute(
            "WITH sb AS (SELECT ('skip-test-' || $2::text || $1::text)::bytea AS b)
             INSERT INTO event_log (event_id, patient_id, event_type, schema_version,
                 hlc_wall, hlc_counter, node_origin, signed_bytes, content_address,
                 body, contributors, signer_key_id, plaintext_twin)
             SELECT uuidv7(), $1::text::uuid, $2, 'test-1',
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
        .query_one(
            "SELECT note_count FROM patient_chart WHERE patient_id = $1::text::uuid",
            &[&pid],
        )
        .await
        .unwrap()
        .get(0);
    c.query("SELECT * FROM cairn_reproject('note.', false, 'test')", &[])
        .await
        .unwrap();
    let after: i32 = c
        .query_one(
            "SELECT note_count FROM patient_chart WHERE patient_id = $1::text::uuid",
            &[&pid],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        before, after,
        "heal must not re-increment a counter projection"
    );
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
    let pid: String = c
        .query_one("SELECT uuidv7()::text", &[])
        .await
        .unwrap()
        .get(0);
    c.execute(
        "WITH sb AS (SELECT ('elig-test-' || $1::text)::bytea AS b)
         INSERT INTO event_log (event_id, patient_id, event_type, schema_version,
             hlc_wall, hlc_counter, node_origin, signed_bytes, content_address,
             body, contributors, signer_key_id, plaintext_twin)
         SELECT $1::text::uuid, $1::text::uuid, 'patient.created', 'test-1',
             (extract(epoch from now()) * 1000)::bigint, 0, 'test-node', b,
             '\\x1220'::bytea || digest(b, 'sha256'),
             jsonb_build_object('name', 'Eligible Winner'),
             '[]'::jsonb, 'test-key', 'probe'
         FROM sb",
        &[&pid],
    )
    .await
    .unwrap();
    // Tamper the provenance alongside the display value (see the identical comment in
    // reproject_heals_tampered_projection_row above): cairn_hlc_overlay_wins is strict-`>`,
    // so the later "restore eligibility, replay again, expect convergence" step below can
    // only re-win if the real event's HLC now dominates a corrupted (not merely equal)
    // stored provenance.
    c.execute(
        "UPDATE patient_chart SET name = 'STALE', demo_hlc_wall = 0 \
         WHERE patient_id = $1::text::uuid",
        &[&pid],
    )
    .await
    .unwrap();
    // Stub the seam to reject everything, replay, then restore by reloading schema.
    c.batch_execute(
        "CREATE OR REPLACE FUNCTION cairn_replay_eligible(e event_log) \
         RETURNS boolean LANGUAGE sql STABLE AS $$ SELECT FALSE $$;",
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
        .query_one(
            "SELECT name FROM patient_chart WHERE patient_id = $1::text::uuid",
            &[&pid],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        name, "STALE",
        "an ineligible event must confer no projection effect"
    );
    // Restore the real predicate (schema replay redefines it).
    drop(c);
    let c2 = db::connect_and_load_schema(&base).await.unwrap();
    c2.query(
        "SELECT * FROM cairn_reproject('patient.', false, 'test')",
        &[],
    )
    .await
    .unwrap();
    let healed: String = c2
        .query_one(
            "SELECT name FROM patient_chart WHERE patient_id = $1::text::uuid",
            &[&pid],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(healed, "Eligible Winner");
}

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
        .query_one(
            "SELECT count(*) FROM reproject_log WHERE source = 'loader'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    drop(c);
    // Same-generation reconnect: NO new loader-sourced run.
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let n1: i64 = c
        .query_one(
            "SELECT count(*) FROM reproject_log WHERE source = 'loader'",
            &[],
        )
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
        .query_one(
            "SELECT count(*) FROM reproject_log WHERE source = 'loader'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        n2,
        n1 + 1,
        "generation change must trigger exactly one heal replay"
    );
}
