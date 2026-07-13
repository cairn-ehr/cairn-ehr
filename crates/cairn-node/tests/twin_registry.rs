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
