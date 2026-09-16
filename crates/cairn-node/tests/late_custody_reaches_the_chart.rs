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
//! 2. It happens once: a further keyed apply runs nothing, a `heal_safe = false` applier never
//!    runs again, and a FRESH keyed apply still runs each applier exactly once (the counting
//!    probe) — the landing is keyed on "the INSERT was a no-op", not on "custody was written".
//! 3. The lenient posture holds: a contradiction the late key reveals is FLAGGED, not refused —
//!    otherwise the key could never land.
//! 4. A deferred event gains its key but not its chart, until re-adjudication promotes it.
//! 5. A rival body under an existing id, carrying its own key, is refused as a substitution and
//!    projects nothing.
//! 6. A rival body never reaches an applier at all: a raising probe proves the substitution guard
//!    refuses it BEFORE the late-custody call could run one (placement rule 1).
//! 7. The strict door has the same entrance and the same fix.
//! 8. The STRICT door's placement rule 1, the twin of test 6: `submit_event`'s late-custody call
//!    also sits after ITS substitution guard.
//! 9. The STRICT door's posture: a contradiction a late key reveals there is REFUSED, and its
//!    custody does not land — the strict counterpart of test 3, and the rule that would break if
//!    someone wrapped db/005's call in `cairn.remote_apply = 'on'` "to match db/020".
//!
//! Tests 4, 5, 6 and 8 pass against the pre-#584 door too — they pin rules the fix must not break,
//! proven by the named mutations in the plan's review ledger: test 4 by M3, test 6 by M6 (killed in
//! the final fix wave). Test 5 on its own cannot see where the late-custody call sits relative to
//! the substitution guard — M6 survived it, because the refusal's rollback erases whatever the
//! rival's appliers wrote — which is why tests 6 and 8 exist. Test 3 fails against the pre-#584
//! door (nothing projects, so nothing flags) and also pins the marker-clear placement via its
//! mutation.
//!
//! Tests 6 and 8 each carry their OWN positive control: after proving the refusal came first, they
//! apply a row the probe SHOULD reach and assert it raises. Without that, a probe that silently
//! stopped being registered (renamed, wrong event type, `heal_safe = FALSE`) would leave the only
//! placement pins passing while testing nothing.
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
    // Build BOTH fixtures BEFORE installing the probe: a panic between install and remove would
    // otherwise leave two extra rows in cairn_projection_apply, which projection_registry.rs
    // pins at an exact count.
    let e = sealed_assert(
        &keys,
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        "amoxicillin",
        WALL,
    );
    // A second event that never arrives keyless: its ONE apply carries its key, so it exercises
    // the other half of the door's condition (see the `after_fresh` assertion).
    let fresh = sealed_assert(
        &keys,
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        "ibuprofen",
        WALL,
    );
    install_probe(&c, "clinical.medication.asserted").await;

    let first = apply_without_key(&c, &e).await;
    let after_first = (probe_runs(&c, "safe").await, probe_runs(&c, "unsafe").await);
    let landing = apply_with_key(&c, &e).await;
    let after_landing = (probe_runs(&c, "safe").await, probe_runs(&c, "unsafe").await);
    let again = apply_with_key(&c, &e).await;
    let after_again = (probe_runs(&c, "safe").await, probe_runs(&c, "unsafe").await);
    let fresh_keyed = apply_with_key(&c, &fresh).await;
    let after_fresh = (probe_runs(&c, "safe").await, probe_runs(&c, "unsafe").await);
    remove_probe(&c).await; // BEFORE asserting

    first.expect("admitted without custody");
    landing.expect("the key is admitted");
    again.expect("an idempotent re-apply is a silent no-op");
    fresh_keyed.expect("a first arrival carrying its key is admitted");
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
    assert_eq!(
        after_fresh,
        (3, 2),
        "A FIRST ARRIVAL CARRYING ITS KEY RUNS EACH APPLIER ONCE: its INSERT is real, so the \
         AFTER INSERT trigger did the work and the late-custody call must NOT run as well. A door \
         that tested only 'this call wrote event_clear' — dropping the `v_log_rows = 0` half of \
         the condition — would read (4, 2) here: every keyed write on the sync hot path paying \
         for its heal-safe appliers twice, with no test to say so"
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

/// A rival body never reaches an applier AT ALL — not even one whose work the refusal would later
/// roll back. Pins placement rule 1: the late-custody call comes AFTER the substitution guard.
///
/// The test above cannot see that rule. With the call moved before the guard, the rival's appliers
/// run, the guard then raises, and the transaction's rollback erases every projection they wrote —
/// so "refused, and on no chart" still holds and the refusal still reads "substitution refused".
/// Here a RAISING probe applier is registered instead: if any applier runs over the row, the probe
/// raises first and its message replaces the door's. With the call placed before the guard, this
/// test fails on exactly that (mutation M6 in the plan's review ledger).
#[tokio::test]
async fn a_rival_body_never_reaches_an_applier() {
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

    // Installed only NOW, after the keyless admission: the AFTER INSERT trigger runs every
    // registered applier on a fresh insert, so the probe would have raised there instead.
    install_raising_probe(&c, "clinical.medication.asserted").await;
    let outcome = apply_with_key(&c, &rival).await;
    // THE POSITIVE CONTROL, under the same probe: the ORIGINAL's key is a late landing the probe
    // SHOULD reach. Without it this test's real assertion is a negative one, and a probe that had
    // quietly stopped being registered — renamed, wrong event type, heal_safe = FALSE — would
    // satisfy it while proving nothing. The probe raises, so this apply rolls back and the
    // original stays keyless, leaving the database as the assertions below expect.
    let control = apply_with_key(&c, &original).await;
    remove_probe(&c).await; // BEFORE asserting: no residue in a pinned-count registry

    let err = outcome.expect_err("a second body under one event id is a substitution");
    assert!(
        db_msg(&err).contains("substitution refused"),
        "the door refuses the rival as a substitution: {}",
        db_msg(&err)
    );
    assert!(
        !db_msg(&err).contains("cairn_test probe"),
        "an applier ran over the rival before the substitution guard refused it — the \
         late-custody call must come AFTER the guard (ADR-0070 placement rule 1): {}",
        db_msg(&err)
    );
    let control_err =
        control.expect_err("the probe raises on a landing that DOES reach an applier");
    assert!(
        db_msg(&control_err).contains("cairn_test probe"),
        "CONTROL: a genuine late landing must reach the probe, or the assertion above passes \
         for the wrong reason — the probe is not registered on this row at all: {}",
        db_msg(&control_err)
    );
}

/// The STRICT door has the same step 9 and the same no-op INSERT, so a local re-submit of an event
/// this node holds without its key — with the key — is the same late landing, and gets the same fix.
/// Judged in the strict posture (no `cairn.remote_apply` marker), as a first arrival there would be.
#[tokio::test]
async fn the_strict_door_brings_a_late_key_to_the_chart_too() {
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
    let submitted = submit_with_key(&c, &e).await;
    assert!(
        submitted.is_ok(),
        "the strict door admits the same bytes with their key: {:?}",
        submitted.as_ref().err().map(db_msg)
    );
    assert_eq!(
        clear_twin(&c, e.event_id).await.as_deref(),
        Some(e.twin.as_str())
    );
    assert_eq!(
        statement_rows(&c, e.medication_id).await,
        1,
        "the strict door projects a late key exactly as the lenient one does"
    );
}

/// Test 6's twin at the STRICT door: `submit_event`'s late-custody call also sits AFTER its
/// substitution guard, so a rival body never reaches an applier there either.
///
/// The two doors carry the same rule in two places, and until this test only `db/020`'s copy was
/// pinned: moving db/005's call above its guard passed every test in the tree. What that costs is
/// not corruption — the RAISE rolls the rival's projections back either way — but the refusal a
/// caller reads. `restore` pens on the door's reason, so a substitution would start arriving as
/// whatever a projection raised first.
#[tokio::test]
async fn the_strict_door_refuses_a_rival_body_before_any_applier_runs() {
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

    // Installed only after the keyless admission, as in test 6: on a fresh insert the AFTER INSERT
    // trigger runs every registered applier, so the probe would raise there instead.
    install_raising_probe(&c, "clinical.medication.asserted").await;
    let outcome = submit_with_key(&c, &rival).await;
    let control = submit_with_key(&c, &original).await; // the positive control — see test 6
    remove_probe(&c).await; // BEFORE asserting: no residue in a pinned-count registry

    let err = outcome.expect_err("a second body under one event id is a substitution");
    assert!(
        db_msg(&err).contains("substitution refused"),
        "the strict door refuses the rival as a substitution: {}",
        db_msg(&err)
    );
    assert!(
        !db_msg(&err).contains("cairn_test probe"),
        "an applier ran over the rival before submit_event's substitution guard refused it — \
         db/005's late-custody call must come AFTER the guard, exactly as db/020's does \
         (ADR-0070 decision 1): {}",
        db_msg(&err)
    );
    let control_err =
        control.expect_err("the probe raises on a landing that DOES reach an applier");
    assert!(
        db_msg(&control_err).contains("cairn_test probe"),
        "CONTROL: a genuine late landing at the strict door must reach the probe, or the \
         assertion above passes for the wrong reason: {}",
        db_msg(&control_err)
    );
}

/// The STRICT counterpart of test 3, and the rule that keeps the two doors HONESTLY different: a
/// contradiction a late key reveals at `submit_event` is REFUSED, and its custody does not land.
///
/// `db/020` keeps `cairn.remote_apply` on across its call precisely so the key can land; db/005
/// does not, and ADR-0070 decision 1 says so ("judged in the strict posture, exactly as a first
/// arrival there would be"). Nothing pinned it, so copying db/020's `set_config` around db/005's
/// call — a plausible "make the doors match" edit — would silently turn a strict refusal into a
/// flag. The asymmetry is deliberate: at the remote door a refusal would strand a key this node
/// can never get again, while at the strict door the author is present and holds the bytes.
#[tokio::test]
async fn a_contradiction_revealed_by_a_late_key_is_refused_at_the_strict_door() {
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
    // The same thread asserted for a SECOND patient (#192), admitted here without its key: the
    // contradiction is invisible until the body opens.
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

    let landed = submit_with_key(&c, &rival_patient).await;

    let err = landed.expect_err("the strict door refuses a contradiction instead of flagging it");
    assert!(
        db_msg(&err).contains("patient cannot change"),
        "the refusal is the #192 guard's, raised through the late-custody dispatch: {}",
        db_msg(&err)
    );
    assert_eq!(
        clear_twin(&c, rival_patient.event_id).await,
        None,
        "THE ASSERTION THAT MATTERS: the RAISE rolled back this call's custody write too, so the \
         strict door lands no key it would not have accepted with the event"
    );
    assert_eq!(
        conflict_flags(&c, thread).await,
        0,
        "and nothing was flagged: flagging is the REMOTE door's posture (test 3). A db/005 call \
         wrapped in `cairn.remote_apply = 'on'` would read 1 here"
    );
}
