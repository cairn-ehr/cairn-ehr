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
