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
