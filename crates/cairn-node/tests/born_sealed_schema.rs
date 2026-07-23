//! ADR-0052 custody-plane schema: tables exist, are locked down, and the
//! clear-payload helper resolves sealed vs unsealed rows.
//! DB-gated on $CAIRN_TEST_PG, serialized via db::test_serial_guard.
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
async fn custody_plane_tables_exist_and_are_locked() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    for t in [
        "node_unwrap_key",
        "event_dek",
        "event_clear",
        "erasure_shred_log",
    ] {
        let n: i64 = c
            .query_one(
                "SELECT count(*) FROM information_schema.tables WHERE table_name = $1",
                &[&t],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(n, 1, "table {t} missing");
    }
    // The mutable custody tables are door-managed: cairn_agent has no direct DML.
    // (db/004 unconditionally creates cairn_agent ahead of db/037 in migration
    // order, so it is always present by the time connect_and_load_schema returns.)
    for t in [
        "event_dek",
        "event_clear",
        "erasure_shred_log",
        "node_unwrap_key",
    ] {
        let ok: bool = c
            .query_one(
                "SELECT has_table_privilege('cairn_agent', $1, 'INSERT')",
                &[&t],
            )
            .await
            .unwrap()
            .get(0);
        assert!(!ok, "cairn_agent must not INSERT into {t} directly");
    }
    // The two custody SECURITY DEFINER functions must NOT be PUBLIC-executable:
    // Postgres grants EXECUTE to PUBLIC by default, and every role (including
    // cairn_agent) is a member of PUBLIC, so an ungated SECURITY DEFINER function
    // is a below-the-floor door bypass — cairn_agent could call it directly with
    // raw SQL instead of going through submit_event/apply_remote_event. db/037
    // must explicitly REVOKE EXECUTE FROM PUBLIC on both.
    for sig in [
        "cairn_execute_shred(uuid, uuid, text)",
        "cairn_register_unwrap_key(bytea)",
    ] {
        let ok: bool = c
            .query_one(
                "SELECT has_function_privilege('cairn_agent', $1, 'EXECUTE')",
                &[&sig],
            )
            .await
            .unwrap()
            .get(0);
        assert!(
            !ok,
            "cairn_agent must not EXECUTE {sig} directly (floor bypass)"
        );
    }
}

#[tokio::test]
async fn register_unwrap_key_is_idempotent_and_rejects_rotation() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.execute("DELETE FROM node_unwrap_key", &[]).await.unwrap(); // test reset
    let pub_a: Vec<u8> = (0u8..32).map(|i| i.wrapping_mul(5)).collect();
    let pub_b: Vec<u8> = (0u8..32).map(|i| i.wrapping_mul(7)).collect();
    c.execute("SELECT cairn_register_unwrap_key($1)", &[&pub_a])
        .await
        .unwrap();
    c.execute("SELECT cairn_register_unwrap_key($1)", &[&pub_a])
        .await
        .unwrap(); // idempotent
    let err = c
        .execute("SELECT cairn_register_unwrap_key($1)", &[&pub_b])
        .await
        .unwrap_err();
    assert!(db_msg(&err).contains("rotation"), "got: {}", db_msg(&err));
}

#[tokio::test]
async fn erasure_shred_type_is_registered_and_twin_checked() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM event_type_class WHERE event_type = 'erasure.shred.asserted'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 1);
    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM cairn_event_twin_check WHERE event_type = 'erasure.shred.asserted'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 1);
}

/// Task 6: every medication projection trigger now reads NEW.body through
/// cairn_clear_payload(NEW) instead of directly. This pins the helper's two
/// branches (Task 5, db/037) that those triggers now depend on, using a
/// composite-type cast against a SYNTHESIZED (never-inserted) event_log row —
/// no event_log INSERT is needed, so it's safe to run standalone. Column order
/// is transcribed from `\d event_log` (db/001 + its ALTER ADD COLUMNs + db/036's
/// `seq` + db/040's trailing `clock_grade`). The GENERATED ALWAYS seq column does not fight this: identity
/// generation is an INSERT-time constraint, not enforced on a bare composite
/// cast (confirmed empirically against CAIRN_TEST_PG before writing this in).
#[tokio::test]
async fn clear_payload_resolves_unsealed_to_body_and_sealed_to_shadow() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    // Unsealed row (sealed = FALSE): cairn_clear_payload must return body unchanged —
    // this is the regression gate for every trigger edited in this task. Cast to
    // ::text and parse (rather than fetching jsonb directly) so this doesn't depend
    // on the tokio-postgres serde_json feature.
    let body_text: String = c
        .query_one(
            "SELECT cairn_clear_payload(ROW(gen_random_uuid(), gen_random_uuid(),
                'clinical.medication.asserted', 'clinical.medication/1', 0, 0, 'n', NULL,
                '\\x00'::bytea, '\\x00'::bytea, '{\"k\":1}'::jsonb, '[]'::jsonb, 'k', 'stub',
                FALSE, NULL, '[]'::jsonb, clock_timestamp(), NULL, NULL, NULL, NULL,
                'unknown')::event_log)::text",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    let body: serde_json::Value = serde_json::from_str(&body_text).unwrap();
    assert_eq!(
        body,
        serde_json::json!({"k": 1}),
        "unsealed row must resolve to NEW.body unchanged"
    );

    // Sealed row with NO event_clear shadow (this node holds no custody): must
    // resolve NULL — the honest-degradation path every edited trigger now checks
    // for via `IF p IS NULL THEN RETURN NULL; END IF;` right after BEGIN.
    let is_null: bool = c
        .query_one(
            "SELECT cairn_clear_payload(ROW(gen_random_uuid(), gen_random_uuid(),
                'clinical.medication.asserted', 'clinical.medication/1', 0, 0, 'n', NULL,
                '\\x00'::bytea, '\\x00'::bytea, '{}'::jsonb, '[]'::jsonb, 'k', 'stub',
                TRUE, NULL, '[]'::jsonb, clock_timestamp(), NULL, NULL, NULL, NULL,
                'unknown')::event_log) IS NULL",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        is_null,
        "sealed row with no event_clear shadow must resolve NULL"
    );
}
