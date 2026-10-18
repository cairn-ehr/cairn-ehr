//! #818: a sign-off interrupted after some lines are signed REPORTS what it did, never "failed".
//!
//! `sign_off_medication_list` signs each line in its own transaction (ADR-0060). Before #818 any
//! error after the first committed line left the loop with `?`, so the caller got an `Err` and
//! lost which lines were signed, which failed and why, and which were never tried. Partial
//! completion is reported, never implied (ADR-0060 decision 2), so the outcome now says where the
//! gesture stopped.
//!
//! How the interruption is made real, not simulated: a test-only constraint trigger on the
//! `medication_attestation` projection fires when ONE named thread's attestation lands, and
//! either makes the backend terminate ITSELF or raises an ordinary ERROR. Deferred, it fires
//! while that line COMMITs: a FATAL there is a real FATAL answer to a real COMMIT, which
//! `db_diagnosis::commit_failure` marks outcome-unknown (`tests/db_diagnosis.rs`'s
//! `a_fatal_answer_is_marked_outcome_unknown`). Immediate, it fires inside the line's own
//! transaction, before any commit. The trigger is gated on SESSION settings
//! (`cairn.test_kill_thread`, `cairn.test_kill_how`), so it is inert for every other connection
//! even if a panic leaked it. It names a thread rather than counting lines: a raised ERROR rolls
//! back a session counter set in the same transaction, so a count would fire again on the next
//! line.
//! It sits on the projection table, not on `event_log`, because `projection_registry.rs` pins
//! the dispatcher as `event_log`'s only AFTER INSERT trigger.
//!
//! DB-gated on $CAIRN_TEST_PG, guard-before-connect (see `medication_signoff.rs`'s header).
mod common;

use cairn_event::SigningKey;
use cairn_node::db;
use cairn_node::medication::signoff::{sign_off_medication_list, SignOffOutcome, StoppedLine};
use cairn_node::medication::{assert_medication, AssertMedicationInput, AttestParams};
use common::{attestation_count, cs, medication_setup as setup};
use tokio_postgres::Client;
use uuid::Uuid;

/// The trigger and its function: create (healing a leaked copy first) or drop.
const DROP_KILL_SWITCH: &str = "
    DROP TRIGGER IF EXISTS cairn_test_kill_on_attestation ON medication_attestation;
    DROP FUNCTION IF EXISTS cairn_test_kill_on_attestation();";

/// When the switch fires for the named thread: at its COMMIT, or at the insert itself.
#[derive(Clone, Copy)]
enum When {
    AtCommit,
    InsideTheLine,
}

/// What it does then: end this backend (a FATAL answer), or refuse with an ordinary ERROR.
#[derive(Clone, Copy)]
enum How {
    Terminate,
    Raise,
}

fn create_kill_switch(when: When) -> String {
    let timing = match when {
        When::AtCommit => "DEFERRABLE INITIALLY DEFERRED",
        When::InsideTheLine => "NOT DEFERRABLE",
    };
    format!(
        "CREATE FUNCTION cairn_test_kill_on_attestation() RETURNS trigger
        LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
        BEGIN
            IF coalesce(current_setting('cairn.test_kill_thread', true), '') <>
               NEW.medication_id::text THEN
                RETURN NULL;  -- not this session's test, or not the named line: inert
            END IF;
            IF current_setting('cairn.test_kill_how', true) = 'raise' THEN
                RAISE EXCEPTION 'test refusal of thread %', NEW.medication_id;
            END IF;
            PERFORM pg_terminate_backend(pg_backend_pid());
            RETURN NULL;
        END $$;
        CREATE CONSTRAINT TRIGGER cairn_test_kill_on_attestation
            AFTER INSERT ON medication_attestation
            {timing}
            FOR EACH ROW EXECUTE FUNCTION cairn_test_kill_on_attestation();"
    )
}

async fn assert_one(c: &mut Client, sk: &SigningKey, kid: &str, patient: Uuid, term: &str) -> Uuid {
    assert_medication(
        c,
        sk,
        kid,
        "origin-a",
        patient,
        &AssertMedicationInput {
            term,
            coding: None,
            formulation: None,
            dose_amount: Some("500"),
            dose_unit: Some("mg"),
            sig: None,
            info_source: "patient",
            started: None,
            started_precision: None,
        },
        None,
        None,
    )
    .await
    .unwrap()
}

/// Three lines in sign order; the switch fires on the SECOND. Returns the outcome, the lines,
/// and a fresh connection to look with (the gesture's own may be dead), the switch removed.
async fn sign_off_with_switch_on_line_2(
    base: &str,
    when: When,
    how: How,
) -> (SignOffOutcome, [Uuid; 3], Client) {
    let mut c = db::connect_and_load_schema(base).await.unwrap();
    let (sk, kid, hsk, hkid) = setup(&c).await;
    let patient = Uuid::now_v7();
    // #345: a chart must be registered before anything is recorded about it.
    common::submit_registration(&c, &sk, &kid, patient, 0).await;
    let mut lines = [
        assert_one(&mut c, &sk, &kid, patient, "metformin").await,
        assert_one(&mut c, &sk, &kid, patient, "amlodipine").await,
        assert_one(&mut c, &sk, &kid, patient, "atorvastatin").await,
    ];
    // `sign_off_targets` signs in sorted thread order.
    lines.sort();

    c.batch_execute(DROP_KILL_SWITCH).await.unwrap();
    c.batch_execute(&create_kill_switch(when)).await.unwrap();
    let how = match how {
        How::Terminate => "terminate",
        How::Raise => "raise",
    };
    c.batch_execute(&format!(
        "SET cairn.test_kill_thread = '{}'; SET cairn.test_kill_how = '{how}'",
        lines[1]
    ))
    .await
    .unwrap();

    let params = AttestParams {
        human_sk: &hsk,
        human_kid: &hkid,
        basis: None,
        note: None,
    };
    let out = sign_off_medication_list(&mut c, &sk, "origin-a", &params, patient, None)
        .await
        .expect("a sign-off that signed a line reports it, never an Err");

    let look = db::connect(base).await.unwrap();
    look.batch_execute(DROP_KILL_SWITCH).await.unwrap();
    (out, lines, look)
}

/// Line 2's COMMIT is answered FATAL. Line 1 is signed and reported so; line 2's outcome is NOT
/// known and is named with its event; line 3 was never tried.
#[tokio::test]
async fn a_commit_lost_mid_sign_off_reports_the_signed_the_unknown_and_the_untried() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let (out, lines, look) =
        sign_off_with_switch_on_line_2(&base, When::AtCommit, How::Terminate).await;

    assert_eq!(
        out.attested,
        vec![lines[0]],
        "line 1 is signed and reported"
    );
    assert_eq!(out.event_ids.len(), 1);
    assert!(out.failed.is_empty(), "{:?}", out.failed);
    let stopped = out.interrupted.expect("the gesture says where it stopped");
    assert_eq!(stopped.medication_id, lines[1], "it stopped at line 2");
    assert!(
        matches!(stopped.line, StoppedLine::MayBeSigned { .. }),
        "a FATAL answer to COMMIT is outcome-unknown: {}",
        stopped.cause
    );
    assert!(
        !stopped.cause.contains("commit outcome unknown"),
        "the cause is the root cause, not the marker's own text: {}",
        stopped.cause
    );
    assert_eq!(
        stopped.not_attempted,
        vec![lines[2]],
        "line 3 was never tried"
    );

    assert_eq!(attestation_count(&look, lines[0]).await, 1);
    assert_eq!(
        attestation_count(&look, lines[2]).await,
        0,
        "an untried line is still unsigned"
    );
}

/// #818 item 3: the backend dies INSIDE line 2, before its commit. Line 2 failed for its own
/// reason, and that reason is what is reported, never the ROLLBACK that then found the
/// connection dead. Line 3's transaction will not open, so the gesture stops there, and line 3
/// is NOT signed. Were the rollback's error propagated with `?`, this would be an `Err` that
/// discards line 1's signature — #818 itself.
#[tokio::test]
async fn a_connection_lost_inside_a_line_fails_that_line_and_stops_at_the_next() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let (out, lines, look) =
        sign_off_with_switch_on_line_2(&base, When::InsideTheLine, How::Terminate).await;

    assert_eq!(out.attested, vec![lines[0]], "line 1 stays signed");
    assert_eq!(out.failed.len(), 1, "{:?}", out.failed);
    assert_eq!(out.failed[0].medication_id, lines[1]);
    assert!(
        out.failed[0].error.contains("terminating connection"),
        "line 2 carries its own reason: {}",
        out.failed[0].error
    );
    let stopped = out
        .interrupted
        .expect("line 3's transaction would not open");
    assert_eq!(stopped.medication_id, lines[2]);
    assert_eq!(stopped.line, StoppedLine::NotSigned);
    assert!(stopped.not_attempted.is_empty(), "line 3 was the last");

    assert_eq!(attestation_count(&look, lines[0]).await, 1);
    assert_eq!(attestation_count(&look, lines[1]).await, 0);
    assert_eq!(attestation_count(&look, lines[2]).await, 0);
}

/// Line 2's COMMIT is answered with an ordinary ERROR, which proves it rolled back: line 2 alone
/// failed, the connection is still good, and line 3 is signed. Nothing was interrupted.
#[tokio::test]
async fn a_commit_refused_with_an_error_fails_that_line_and_signs_the_rest() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let (out, lines, look) =
        sign_off_with_switch_on_line_2(&base, When::AtCommit, How::Raise).await;

    assert_eq!(
        out.attested,
        vec![lines[0], lines[2]],
        "lines 1 and 3 are signed"
    );
    assert_eq!(out.failed.len(), 1, "{:?}", out.failed);
    assert_eq!(out.failed[0].medication_id, lines[1]);
    assert!(
        out.failed[0].error.contains("test refusal"),
        "{}",
        out.failed[0].error
    );
    assert_eq!(
        out.interrupted, None,
        "an answered commit does not stop the gesture"
    );

    assert_eq!(attestation_count(&look, lines[1]).await, 0);
    assert_eq!(attestation_count(&look, lines[2]).await, 1);
}
