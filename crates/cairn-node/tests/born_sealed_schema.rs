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
/// branches (Task 5, db/037) that those triggers now depend on, against a
/// SYNTHESIZED (never-inserted) event_log row — no event_log INSERT is needed, so it's
/// safe to run standalone. The GENERATED ALWAYS `seq` column does not fight this:
/// identity generation is an INSERT-time constraint, not enforced on a synthesized
/// composite (confirmed empirically against CAIRN_TEST_PG).
///
/// BY NAME, NOT BY POSITION (#296). This used to build the row with a positional
/// `ROW(...)::event_log` literal whose element order was transcribed from `\d event_log`.
/// That made the test hostage to the physical attribute order of a SHARED test database:
/// any other test that dropped and re-added an `event_log` column (the migrations use
/// `ADD COLUMN IF NOT EXISTS`, which appends at the END) permanently shifted the tail, and
/// this test then bound the wrong value into the wrong column — surfacing as
/// `invalid input syntax for type bigint: "unknown"` on the SECOND run against the same
/// database, far from its cause, in an unrelated crate. jsonb_populate_record binds by
/// COLUMN NAME, so column order is irrelevant and the whole class of failure is gone. Any
/// future synthesized event_log row here must keep using a by-name construction.
///
/// Omitted keys default to NULL, so only the columns this helper actually reads
/// (`sealed`, `body`, `event_id`) plus the NOT NULL ones need naming.
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
            "SELECT cairn_clear_payload(jsonb_populate_record(NULL::event_log, jsonb_build_object(
                'event_id', gen_random_uuid(), 'patient_id', gen_random_uuid(),
                'event_type', 'clinical.medication.asserted',
                'schema_version', 'clinical.medication/1',
                'hlc_wall', 0, 'hlc_counter', 0, 'node_origin', 'n',
                'signed_bytes', '\\x00'::bytea, 'content_address', '\\x00'::bytea,
                'body', '{\"k\":1}'::jsonb, 'contributors', '[]'::jsonb,
                'signer_key_id', 'k', 'plaintext_twin', 'stub', 'sealed', FALSE,
                'attachments', '[]'::jsonb, 'clock_grade', 'unknown')))::text",
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
            "SELECT cairn_clear_payload(jsonb_populate_record(NULL::event_log, jsonb_build_object(
                'event_id', gen_random_uuid(), 'patient_id', gen_random_uuid(),
                'event_type', 'clinical.medication.asserted',
                'schema_version', 'clinical.medication/1',
                'hlc_wall', 0, 'hlc_counter', 0, 'node_origin', 'n',
                'signed_bytes', '\\x00'::bytea, 'content_address', '\\x00'::bytea,
                'body', '{}'::jsonb, 'contributors', '[]'::jsonb,
                'signer_key_id', 'k', 'plaintext_twin', 'stub', 'sealed', TRUE,
                'attachments', '[]'::jsonb, 'clock_grade', 'unknown'))) IS NULL",
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
