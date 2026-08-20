//! #385 — what `cairn_event_thread` (db/048 §10) costs, and what it must not stop answering.
//!
//! # The shape of the problem
//!
//! `cairn_event_thread` resolves one event to its medication thread by `UNION ALL` over five
//! projection tables, filtered on `content_address`. None of the five indexed that column, so
//! every call was five sequential scans — and `cairn_effective_sensitivity` calls it once per
//! thread through a `LATERAL`, making a chart report *O(threads on chart × every medication
//! row on the node)*. That is precisely the shape db/048 §6 cites [#336] to avoid.
//!
//! Worse, it was paid unconditionally: the function never asked whether the event's TYPE
//! could belong to a medication thread at all, so a note, a demographic edit or a
//! registration ran all five scans to learn what its type already guaranteed. The chart-wide
//! reading in `chart_sensitivity` always resolves off a registration event, so it paid the
//! full five, per chart, every time.
//!
//! # What is asserted here, and what is deliberately not
//!
//! **Asserted:** the five indexes exist; the type short-circuit genuinely fires; and the
//! predicate it keys on still answers "might have a thread" for every medication event type,
//! which is the precondition that keeps the short-circuit from swallowing a real resolution.
//!
//! **Not asserted: a query plan.** A planner assertion (`EXPLAIN` showing an Index Scan)
//! would be a lie at test scale — on a table of a few dozen rows Postgres will correctly
//! choose a sequential scan, so the test would fail on a correct index or, worse, be made to
//! "pass" by forcing `enable_seqscan = off`, which proves only that the index is usable and
//! nothing about whether it is used. The index's presence is the checkable fact; the
//! magnitude of the win is a volume measurement, and it belongs with the reprojection
//! benchmark on the Pi rig (#272), not in a unit gate.
//!
//! Every test here self-skips without `$CAIRN_TEST_PG` (see `common::cs`); a skipped run
//! prints `ok` and proves nothing. CI sets it.
mod common;
use common::{
    content_address_of, cs, setup, submit_registration, submit_signed_with_id, EventSpec,
};
use uuid::Uuid;

/// The five tables `cairn_event_thread`'s `UNION ALL` reads, in its order. Kept as data so
/// the index guard names the offender rather than failing on an anonymous count.
const THREAD_PROJECTIONS: [&str; 5] = [
    "medication_statement",
    "medication_cessation",
    "medication_coding",
    "medication_dose_event",
    "medication_dose_correction",
];

/// Every medication event type that writes one of those five tables. The short-circuit is
/// safe only while `cairn_event_type_has_no_thread` answers FALSE for all of them.
const MEDICATION_EVENT_TYPES: [&str; 6] = [
    "clinical.medication.asserted",
    "clinical.medication-cessation.asserted",
    "clinical.medication-coding.asserted",
    "clinical.medication-coding-correction.asserted",
    "clinical.medication-dose-change.asserted",
    "clinical.medication-dose-correction.asserted",
];

/// The `content_address` lookup the five-way UNION performs must be index-backed on each arm.
///
/// Asserted against `pg_index` rather than against the migration text: an index dropped by a
/// later migration, or never created because its `CREATE INDEX` sat behind a guard that did
/// not fire, is invisible to a grep of db/*.sql and is exactly the regression that matters.
#[tokio::test]
async fn every_medication_thread_projection_indexes_content_address() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();

    let mut missing = Vec::new();
    for table in THREAD_PROJECTIONS {
        // An index whose FIRST key column is content_address. Leading-column, not
        // "mentions the column anywhere": only a leading key serves an equality lookup on
        // it, so a composite index that merely happens to include content_address later
        // must not read as coverage.
        let indexed: bool = c
            .query_one(
                "SELECT EXISTS (
                     SELECT 1 FROM pg_index i
                       JOIN pg_class t ON t.oid = i.indrelid
                       JOIN pg_attribute a
                         ON a.attrelid = t.oid AND a.attnum = i.indkey[0]
                      WHERE t.relname = $1 AND a.attname = 'content_address')",
                &[&table],
            )
            .await
            .unwrap()
            .get(0);
        if !indexed {
            missing.push(table);
        }
    }

    assert!(
        missing.is_empty(),
        "these medication projections have no index leading on content_address, so \
         cairn_event_thread scans them sequentially — once per thread, per chart open \
         (#385): {missing:?}"
    );
}

/// The type short-circuit fires: a thread-free event resolves to NULL **even when a
/// medication projection row carries its exact content address**.
///
/// The decoy row is what makes this a real test rather than a restatement. Without the
/// short-circuit `cairn_event_thread` would scan the five tables, find the seeded address and
/// return that `medication_id`; with it, the function never looks. Delete the short-circuit
/// and this test fails — which is the only way to pin an optimisation whose whole point is
/// that it changes no answer in normal use.
///
/// The seeded row is not a realistic clinical state (no real medication projection would
/// carry a `note.added` event's address). It is a probe, and it is honest about it: the test
/// asserts the probe is genuinely present before concluding anything from the NULL, so
/// "short-circuit worked" can never be confused with "the fixture never landed".
#[tokio::test]
async fn a_thread_free_event_type_never_reaches_the_medication_scans() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &["medication_statement"]).await;

    // #345: a chart's first event must be its registration, so the note needs one first.
    let patient = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, patient, 0).await;

    let note = Uuid::now_v7();
    submit_signed_with_id(
        &c,
        &sk,
        &kid,
        note,
        EventSpec {
            patient,
            event_type: "note.added",
            schema_version: "1",
            payload: serde_json::json!({"text": "a note carries no medication thread"}),
            plaintext_twin: None,
            wall: 10,
        },
    )
    .await
    .expect("note accepted");
    let address = content_address_of(&c, note).await;

    // The decoy: a medication_statement row whose content_address IS the note's.
    let thread = Uuid::now_v7();
    c.execute(
        "INSERT INTO medication_statement
             (medication_id, patient_id, term, info_source, hlc_wall, hlc_counter, origin,
              content_address)
         VALUES ($1::text::uuid, $2::text::uuid, 'decoy', 'patient-reported', 1, 0, 'n', $3)",
        &[&thread.to_string(), &patient.to_string(), &address],
    )
    .await
    .expect("decoy projection row seeded");

    let planted: i64 = c
        .query_one(
            "SELECT count(*) FROM medication_statement WHERE content_address = $1",
            &[&address],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        planted, 1,
        "the probe must be present, or the NULL below proves nothing"
    );

    // UUID BINDING: cairn-node does not enable tokio-postgres's `with-uuid-1`, so a Uuid has
    // no ToSql/FromSql — bind as text, cast in SQL, and read the result back as text.
    let resolved: Option<String> = c
        .query_one(
            "SELECT cairn_event_thread($1::text::uuid)::text",
            &[&note.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        resolved.is_none(),
        "a note.added is a type db/048 §10b has confirmed thread-free, so cairn_event_thread \
         must return without scanning the medication projections — it found {resolved:?}"
    );
}

/// The precondition the short-circuit rests on, pinned so widening §10b's list cannot
/// silently disable resolution for a type that genuinely has a thread.
///
/// Note the safe asymmetry this does NOT need to guard: if a future edit added a
/// thread-BEARING type to the list, both this short-circuit and §11's conservative bound key
/// on the same predicate, so resolution and the bound would move together — toward
/// over-protection, never toward exposure. This test exists because over-protecting the whole
/// medication stream would still be a serious defect, not because it would leak.
#[tokio::test]
async fn no_medication_event_type_is_classified_thread_free() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();

    for event_type in MEDICATION_EVENT_TYPES {
        let has_no_thread: bool = c
            .query_one(
                "SELECT cairn_event_type_has_no_thread($1)",
                &[&event_type.to_string()],
            )
            .await
            .unwrap()
            .get(0);
        assert!(
            !has_no_thread,
            "{event_type} writes one of the five thread projections, so classifying it \
             thread-free would make cairn_event_thread return NULL for every event on every \
             medication thread (#385)"
        );
    }
}
