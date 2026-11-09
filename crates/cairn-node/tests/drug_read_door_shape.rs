//! Allergy slice 3, ruling T2R-b: a drugref whose `allergy_check(uuid, uuid)` has another result
//! shape is "unavailable", never a fault.
//!
//! The facts door (db/076) reads `allergy_check`'s columns by name. A drugref release that
//! reshaped it would make that read raise, so `cairn_drug_available()` also requires the
//! function's result to be EXACTLY drugref db/068's. This is the Rust mirror of section 3c of
//! `db/tests/076_drug_read_door_test.sql`; it lives apart from `drug_read_door.rs` only because
//! that file is at the 500-line limit.
//!
//! drugref is simulated by `tests/fixtures/drug_stub.sql`, loaded only inside a transaction this
//! file rolls back. DB-gated on `$CAIRN_TEST_PG` (a skipped run prints `skipped:`), serialized via
//! `db::test_serial_guard`.
use cairn_node::db;

mod common;

/// The drugref stand-in. Never load it outside a transaction that rolls back.
const STUB: &str = include_str!("fixtures/drug_stub.sql");

#[tokio::test]
async fn a_reshaped_allergy_check_is_unavailable() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let tx = c.transaction().await.unwrap();
    tx.batch_execute(STUB).await.unwrap();
    let available = || async {
        tx.query_one("SELECT cairn_drug_available()", &[])
            .await
            .unwrap()
            .get::<_, bool>(0)
    };
    assert!(available().await, "control: the stub has db/068's shape");

    // Same name and arguments, another RETURNS TABLE.
    tx.batch_execute(
        "DROP FUNCTION drugref.allergy_check(uuid, uuid);
         CREATE FUNCTION drugref.allergy_check(p_prescribed uuid, p_allergen uuid)
         RETURNS TABLE (verdict text, group_name text) LANGUAGE sql STABLE
         AS $$ SELECT 'match', NULL::text $$;",
    )
    .await
    .unwrap();
    assert!(!available().await, "a reshaped allergy_check: unavailable");
    // Every door answers nothing, and none raises (an Err here would fail the unwrap).
    let rows: i64 = tx
        .query_one(
            "WITH pen AS (SELECT class_uuid AS u FROM drugref.substance_class \
                           WHERE class_name = 'Penicillins'), \
                  amox AS (SELECT moiety_uuid AS u FROM drugref.substance_moiety \
                           WHERE display_name = 'amoxicillin') \
             SELECT (SELECT count(*) FROM cairn_drug_search('amox', 20)) \
                  + (SELECT count(*) FROM cairn_drug_facts(ARRAY(SELECT u FROM pen), \
                                                           ARRAY(SELECT u FROM amox))) \
                  + (SELECT count(*) FROM cairn_drug_group_status(ARRAY(SELECT u FROM pen)))",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(rows, 0, "reshaped: no options, no facts, no group status");
    tx.rollback().await.unwrap();
}
