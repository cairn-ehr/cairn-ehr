//! ADR-0056 decisions 1 + 4 (issues #265/#266): the clinical remote door admits an event
//! whose `event_type` it cannot classify — stored verbatim, no projection rows, no power —
//! and power is granted only after the classification-gated floor checks are re-run.
//!
//! Before this slice the door RAISED on an unclassifiable type, so the event was never
//! stored at all. A phone-tier node carrying a chart between two upgraded facilities (the
//! §6.1 sneakernet path, the case Cairn exists for) acquired NOTHING past the first
//! unknown-type event — not unrendered, absent. These tests pin the contract that replaces
//! it: custody is total, power is deferred.
//!
//! Real Postgres, gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.
use cairn_node::db;
use tokio_postgres::Client;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// The Postgres error message for a failed statement (Display renders only "db error";
/// the RAISE text lives in the DbError payload — project convention, see identity_linkage.rs).
fn db_msg(e: &tokio_postgres::Error) -> String {
    e.as_db_error()
        .map(|d| d.message().to_string())
        .unwrap_or_else(|| e.to_string())
}

/// A projection may only be registered for a CLASSIFIED type.
///
/// This guard is load-bearing, not hygiene. The deferred marker row is written AFTER the
/// `event_log` INSERT, but the AFTER-INSERT projection dispatcher fires DURING it. So a type
/// registered for projection without an `event_type_class` row would be projected at
/// admission — granting exactly the power the marker exists to withhold. Making that state
/// unreachable at migration time is also one of the two legs (the other is
/// `cairn_replay_eligible`) holding up the guarantee that no projection apply fn ever sees a
/// deferred row, which is what lets db/018 and db/034 keep trusting `event_log.attester_key`.
#[tokio::test]
async fn projection_registration_requires_a_classified_type() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c: Client = db::connect_and_load_schema(&base).await.unwrap();
    // `patient_chart_apply` exists with the right signature and `patient_chart` is a real
    // relation, so the two pre-existing registration guards pass — only the classification
    // one can fire, which is what makes this test discriminate.
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

/// The marker table is the EXPLICIT deferred state ADR-0056's corollary demands ("never
/// inferred from a null classification lookup falling through the gates by three-valued
/// logic"). Pin its shape so a later edit cannot quietly drop the column that records WHY a
/// re-adjudication failed — the whole of decision 4's "flagged legibly".
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
