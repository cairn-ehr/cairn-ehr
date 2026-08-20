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
//! reading in `chart_sensitivity` resolves off the chart's registration event whenever one is
//! on file, so on the common path it paid the full five, per chart, every time. (Not
//! *always*: `sensitivity/report.rs` has an explicit no-registration arm — reachable in
//! ordinary federated operation, when a chart's events arrive by sync ahead of its
//! registration — which reads `cairn_sensitivity_standing` directly and never came here.)
//!
//! # What is asserted here, and what is deliberately not
//!
//! **Asserted:** the five indexes exist (leading-column, valid, on the `public` table); the
//! type short-circuit genuinely fires; and the predicate it keys on still answers "might have
//! a thread" for every medication event type — which since #385 is not merely a precondition
//! for the optimisation but the guard standing between a §10b list edit and silent protection
//! loss across every medication thread. See
//! [`no_medication_event_type_is_classified_thread_free`], which carries the measured
//! before/after.
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
//! prints `ok` and proves nothing. CI sets it. That the whole DB-gated suite can go silently
//! green if that one variable is ever unset is a suite-wide hole, tracked in #442.
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
        // THE TABLE IS RESOLVED BY QUALIFIED NAME, NOT BY `relname`, and that is not
        // fussiness — the `relname = $1` form has a demonstrated false-positive path, the
        // one failure mode a guard must never have (reporting coverage that is not there).
        // `relname` is unique only WITHIN a schema, so dropping the real index and creating a
        // same-named table with one in any other schema makes the unqualified query answer
        // `true`. A `pg_temp` table in the very same session is enough, and pg_temp is
        // searched first.
        //
        // `REPO_SCHEMAS` does NOT fix this, which was worth measuring rather than assuming:
        // it excludes Postgres's own schemas, and a decoy schema is not one of those — the
        // decoy still matches. It is the right filter for "which functions is this repo
        // answerable for" and the wrong one for "is THIS table indexed". `to_regclass` is the
        // right tool, and it is the same idiom db/048 §10 uses to probe for these very
        // tables.
        //
        // `indisvalid` closes the narrower second hole: a failed CREATE INDEX (e.g.
        // CONCURRENTLY) leaves a row in pg_index that the planner will never use — present in
        // the catalogue, useless in practice, indistinguishable here without this clause.
        //
        // An index whose FIRST key column is content_address (`indkey[0]`, an int2vector, so
        // 0-based). Leading-column, not "mentions the column anywhere": only a leading key
        // serves an equality lookup on it, so a composite index that merely happens to
        // include content_address later must not read as coverage.
        let indexed: bool = c
            .query_one(
                "SELECT EXISTS (
                     SELECT 1 FROM pg_index i
                       JOIN pg_attribute a
                         ON a.attrelid = i.indrelid AND a.attnum = i.indkey[0]
                      WHERE i.indrelid = to_regclass('public.' || $1)
                        AND a.attname = 'content_address'
                        AND i.indisvalid)",
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

    // BOTH FACTS ARE NOW IN HAND, SO REMOVE THE PROBE BEFORE ASSERTING ON THEM. The ordering
    // is the point: an assertion that fired first would panic past this cleanup and leave the
    // decoy behind on exactly the runs someone is debugging.
    //
    // `connect_and_load_schema` does not truncate and `setup()` only TRUNCATEs the tables it
    // is told about, so without this the decoy survives every later test in the shared
    // database — one more per run. Nothing today is harmed (every other read of
    // medication_statement is scoped by patient_id or medication_id), but it is a projection
    // row that contradicts the event log, and the first person to write the very plausible
    // guard "every projection row's content_address resolves to an event of a type registered
    // for that projection" would find it and have no idea where it came from.
    c.execute(
        "DELETE FROM medication_statement WHERE content_address = $1",
        &[&address],
    )
    .await
    .expect("decoy projection row removed");

    assert_eq!(
        planted, 1,
        "the probe must have been present, or the NULL below proves nothing"
    );
    assert!(
        resolved.is_none(),
        "a note.added is a type db/048 §10b has confirmed thread-free, so cairn_event_thread \
         must return without scanning the medication projections — it found {resolved:?}"
    );
}

/// **The load-bearing guard of #385. Do not delete it as a tidy-up.**
///
/// It pins the precondition the §10 short-circuit rests on: widening §10b's list to cover a
/// type that genuinely has a thread must not be possible without something going red.
///
/// An earlier draft of this docstring described that widening as a safe asymmetry — "both the
/// short-circuit and §11's conservative bound key on the same predicate, so resolution and the
/// bound move together, toward over-protection, never toward exposure". **That is exactly
/// backwards**, and it matters because it reads as licence to delete this test.
///
/// §11's bound arm is gated on the NEGATION (`AND NOT cairn_event_type_has_no_thread(...)`),
/// so a type in the list is EXCLUDED from the bound rather than covered by it. Add a
/// medication type and all three thread arms fall silent at once — resolved (this returns
/// NULL now), bound (predicate TRUE), coarsened (the thread is on this very chart) — and a
/// standing `sequestered` thread grade reads back as `('routine','none')`. Measured: adding
/// `OR p_type LIKE 'clinical.%'` to §10b does precisely that. Before #385 the same edit was
/// harmless, because §11's resolved arm was an independent net that never consulted the
/// predicate. The short-circuit removed that independence, so this test and
/// `sensitivity_ladder.rs::f5a_*` are what remain.
///
/// KNOWN LIMIT: `MEDICATION_EVENT_TYPES` is hand-maintained, so a SEVENTH medication verb is
/// unguarded until someone adds it here — the mirror-pair hazard `common/mod.rs` warns about,
/// tracked in #441.
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
