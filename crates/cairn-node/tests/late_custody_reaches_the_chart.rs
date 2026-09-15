//! #584 / ADR-0070 — a key that arrives after its event brings the record to the chart.
//!
//! # The defect
//!
//! Projections are dispatched by ONE `AFTER INSERT` trigger on `event_log`. Both doors write
//! custody (`event_dek`, `event_clear`) BEFORE their INSERT so the trigger can read the clear view.
//! When the key comes later, the second apply writes `event_clear`, its INSERT is a no-op, and the
//! trigger never fires again: the body opens and the medication list stays empty. `pull --full`,
//! `requeue` and `restore` all reached that state, and `restore` said nothing about it.
//!
//! # What these tests pin
//!
//! 1. The headline: a keyed re-apply projects BOTH appliers of the type.
//! 2. It happens once: a further keyed apply runs nothing, and a `heal_safe = false` applier never
//!    runs again (the counting probe).
//! 3. The lenient posture holds: a contradiction the late key reveals is FLAGGED, not refused —
//!    otherwise the key could never land.
//! 4. A deferred event gains its key but not its chart, until re-adjudication promotes it.
//! 5. A rival body under an existing id, carrying its own key, is refused as a substitution and
//!    projects nothing.
//! 6. The strict door has the same entrance and the same fix.
//!
//! Tests 4 and 5 pass against the pre-#584 door too — they pin placement rules the fix must not
//! break, and each is proven by a named mutation in the plan (Task 7). Test 3 fails against the
//! pre-#584 door (nothing projects, so nothing flags) and also pins the marker-clear placement via
//! its mutation.
//!
//! Real Postgres, gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.

mod common;
#[path = "common/late_custody_kit.rs"]
mod late_custody_kit;

use cairn_node::db;
use common::db_msg; // the RAISE text — an error's Display is only "db error"
use late_custody_kit::*;
use uuid::Uuid;

/// THE HEADLINE. Admitted without its key, the record is invisible; its key arriving makes it
/// readable AND puts it on the chart, through both of the type's appliers.
#[tokio::test]
async fn a_key_arriving_after_its_event_brings_the_record_to_the_chart() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let keys = fresh_node(&c).await;
    remove_probe(&c).await;

    let e = sealed_assert(
        &keys,
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        "amoxicillin",
        WALL,
    );
    apply_without_key(&c, &e)
        .await
        .expect("admitted without custody");
    assert_eq!(
        clear_twin(&c, e.event_id).await,
        None,
        "premise: the body is unreadable"
    );
    assert_eq!(
        statement_rows(&c, e.medication_id).await,
        0,
        "premise: nothing projected"
    );
    assert_eq!(
        dose_seed_rows(&c, e.medication_id).await,
        0,
        "premise: the dose seed is custody-gated too, so 1 below proves the landing ran it"
    );

    apply_with_key(&c, &e).await.expect("the key is admitted");
    assert_eq!(
        clear_twin(&c, e.event_id).await.as_deref(),
        Some(e.twin.as_str()),
        "the body opens"
    );
    assert_eq!(
        statement_rows(&c, e.medication_id).await,
        1,
        "THE ASSERTION THAT MATTERS: the record is on the medication list, with no reproject"
    );
    assert_eq!(
        dose_seed_rows(&c, e.medication_id).await,
        1,
        "and the type's second applier ran too — a fix that ran only one would pass the line above"
    );
}

/// The landing is the ONE moment: a third apply, keyed again, runs nothing — and the counter-shaped
/// applier is never re-run, even at the landing.
#[tokio::test]
async fn a_late_key_runs_the_heal_safe_appliers_once_and_never_again() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let keys = fresh_node(&c).await;
    // Build the fixture BEFORE installing the probe: a panic between install and remove would
    // otherwise leave two extra rows in cairn_projection_apply, which projection_registry.rs
    // pins at an exact count (27).
    let e = sealed_assert(
        &keys,
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        "amoxicillin",
        WALL,
    );
    install_probe(&c, "clinical.medication.asserted").await;

    let first = apply_without_key(&c, &e).await;
    let after_first = (probe_runs(&c, "safe").await, probe_runs(&c, "unsafe").await);
    let landing = apply_with_key(&c, &e).await;
    let after_landing = (probe_runs(&c, "safe").await, probe_runs(&c, "unsafe").await);
    let again = apply_with_key(&c, &e).await;
    let after_again = (probe_runs(&c, "safe").await, probe_runs(&c, "unsafe").await);
    remove_probe(&c).await; // BEFORE asserting

    first.expect("admitted without custody");
    landing.expect("the key is admitted");
    again.expect("an idempotent re-apply is a silent no-op");
    assert_eq!(
        after_first,
        (1, 1),
        "premise: a fresh insert runs every applier once"
    );
    assert_eq!(
        after_landing,
        (2, 1),
        "the landing re-runs the heal-safe applier and NOT the counter-shaped one"
    );
    assert_eq!(
        after_again,
        (2, 1),
        "custody already held: nothing new landed, so nothing runs — the trigger for the heal is \
         'this call wrote event_clear', never 'the INSERT was a no-op'"
    );
}

/// The late key reveals a #192 contradiction (the same thread asserted for a second patient). In
/// the lenient posture that is a FLAG. Were the heal to run after the door clears
/// `cairn.remote_apply`, the guard would RAISE and the key could never land.
#[tokio::test]
async fn a_contradiction_revealed_by_a_late_key_is_flagged_not_refused() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let keys = fresh_node(&c).await;
    remove_probe(&c).await;

    let thread = Uuid::now_v7();
    let standing = sealed_assert(
        &keys,
        Uuid::now_v7(),
        thread,
        Uuid::now_v7(),
        "amoxicillin",
        WALL,
    );
    apply_with_key(&c, &standing)
        .await
        .expect("the thread's first chart");
    let rival_patient = sealed_assert(
        &keys,
        Uuid::now_v7(),
        thread,
        Uuid::now_v7(),
        "amoxicillin",
        WALL + 1,
    );
    apply_without_key(&c, &rival_patient)
        .await
        .expect("admitted without custody");
    assert_eq!(
        conflict_flags(&c, thread).await,
        0,
        "premise: an unreadable body cannot contradict"
    );

    let landed = apply_with_key(&c, &rival_patient).await;
    assert!(
        landed.is_ok(),
        "the key must land — a refusal here strands it forever: {:?}",
        landed.as_ref().err().map(db_msg)
    );
    assert_eq!(
        conflict_flags(&c, thread).await,
        1,
        "the contradiction is on the worklist, as it would be had the key come with the event"
    );
}

/// A deferred event's key lands, its body opens, and its chart waits for re-adjudication — which
/// then projects it through the same shared dispatch.
#[tokio::test]
async fn a_deferred_event_gains_its_key_but_not_its_chart_until_promoted() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let keys = fresh_node(&c).await;
    remove_probe(&c).await;

    let e = sealed_assert(
        &keys,
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        "amoxicillin",
        WALL,
    );
    apply_without_key(&c, &e)
        .await
        .expect("admitted without custody");
    c.execute(
        "INSERT INTO event_deferred (event_id, event_type) \
         VALUES ($1::text::uuid, 'clinical.medication.asserted')",
        &[&e.event_id.to_string()],
    )
    .await
    .expect("mark it deferred, as an event awaiting re-adjudication is");

    apply_with_key(&c, &e).await.expect("the key is admitted");
    assert_eq!(
        clear_twin(&c, e.event_id).await.as_deref(),
        Some(e.twin.as_str())
    );
    assert_eq!(
        statement_rows(&c, e.medication_id).await,
        0,
        "a deferred event has not passed its gates: the late key must not project it"
    );

    c.batch_execute("SELECT * FROM cairn_readjudicate_deferred()")
        .await
        .expect("re-adjudication runs");
    assert_eq!(
        statement_rows(&c, e.medication_id).await,
        1,
        "promotion projects it — gate 4, through the shared dispatch"
    );
}

/// A DIFFERENT body filed under an event id this node already holds without custody, carrying its
/// own key. The door refuses it as a substitution with the door's own reason, and no projection
/// of the rival survives.
#[tokio::test]
async fn a_rival_body_carrying_its_own_key_is_refused_and_projects_nothing() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let keys = fresh_node(&c).await;
    remove_probe(&c).await;

    let (patient, event_id) = (Uuid::now_v7(), Uuid::now_v7());
    let original = sealed_assert(
        &keys,
        patient,
        Uuid::now_v7(),
        event_id,
        "amoxicillin",
        WALL,
    );
    apply_without_key(&c, &original)
        .await
        .expect("admitted without custody");
    let rival = sealed_assert(&keys, patient, Uuid::now_v7(), event_id, "warfarin", WALL);

    let err = apply_with_key(&c, &rival)
        .await
        .expect_err("a second body under one event id is a substitution");
    assert!(
        db_msg(&err).contains("substitution refused"),
        "the refusal names the substitution, not whatever a projection raised: {}",
        db_msg(&err)
    );
    assert_eq!(
        clear_twin(&c, event_id).await,
        None,
        "the rival's body did not stay"
    );
    assert_eq!(
        statement_rows(&c, rival.medication_id).await,
        0,
        "the rival is on no chart"
    );
    assert_eq!(statement_rows(&c, original.medication_id).await, 0);
}
