//! The node-local gesture-timing aggregates (#288 / §1.2): the table holds running
//! estimates and NOTHING that identifies a person, a patient or a moment.
use cairn_node::db;
use cairn_node::ui_timing::{read_aggregates, record_gesture};

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

#[tokio::test]
async fn recording_a_gesture_creates_then_updates_one_cell() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    // Guard BEFORE connect: every DB-gated suite in this crate takes the cluster-wide
    // serial lock first, and taking it in the other order lets two suites interleave their
    // schema loads.
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE ui_gesture_timing").await.unwrap();

    record_gesture(&c, "signoff", 5, 1_200).await.unwrap();
    record_gesture(&c, "signoff", 5, 1_400).await.unwrap();

    let aggregates = read_aggregates(&c).await.unwrap();
    assert_eq!(aggregates.len(), 1, "same kind and bucket -> one cell");
    let ((kind, bucket), agg) = aggregates.into_iter().next().unwrap();
    assert_eq!(kind, "signoff");
    assert_eq!(bucket, "4-8");
    assert_eq!(agg.n, 2);
}

#[tokio::test]
async fn different_buckets_are_separate_cells() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE ui_gesture_timing").await.unwrap();

    record_gesture(&c, "signoff", 2, 900).await.unwrap();
    record_gesture(&c, "signoff", 12, 3_000).await.unwrap();

    assert_eq!(read_aggregates(&c).await.unwrap().len(), 2);
}

/// The privacy shape, asserted rather than merely documented: the table must carry no
/// column that could identify a person, a patient or a moment.
#[tokio::test]
async fn the_table_carries_no_identifying_column() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    let columns: Vec<String> = c
        .query(
            "SELECT column_name FROM information_schema.columns \
             WHERE table_name = 'ui_gesture_timing'",
            &[],
        )
        .await
        .unwrap()
        .iter()
        .map(|r| r.get::<_, String>(0))
        .collect();

    let mut expected = vec!["gesture_kind", "size_bucket", "n", "p50_ms", "p95_ms"];
    expected.sort();
    let mut actual: Vec<&str> = columns.iter().map(String::as_str).collect();
    actual.sort();
    assert_eq!(
        actual, expected,
        "ui_gesture_timing gained a column. If it identifies a person, a patient or a \
         moment, it must not exist — read the module header before changing this test."
    );
}

/// The closed vocabulary is enforced by the DATABASE, not merely by the caller. A future
/// gesture kind must add itself to the CHECK deliberately; smuggling free text into this
/// table is how an identifier eventually lands in it.
#[tokio::test]
async fn an_unknown_gesture_kind_is_refused_by_the_floor() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    let refused = record_gesture(&c, "dr-vega-was-here", 1, 100).await;
    assert!(
        refused.is_err(),
        "an unrecognised gesture kind must be refused by the CHECK constraint"
    );
}
