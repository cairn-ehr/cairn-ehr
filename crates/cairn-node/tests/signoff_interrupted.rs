//! #818: a sign-off interrupted after some lines are signed REPORTS what it did, never "failed".
//!
//! `sign_off_medication_list` signs each line in its own transaction (ADR-0060). Before #818 any
//! error after the first committed line left the loop with `?`, so the caller got an `Err` and
//! lost which lines were signed, which failed and why, and which were never tried. Partial
//! completion is reported, never implied (ADR-0060 decision 2), so the outcome now says where the
//! gesture stopped.
//!
//! How the interruption is made real, not simulated: a test-only deferred constraint trigger on
//! the `medication_attestation` projection makes the backend terminate ITSELF while the Nth
//! attestation commits. That is a real FATAL answer to a real COMMIT, which
//! `db_diagnosis::commit_failure` marks outcome-unknown (`db_diagnosis.rs`'s
//! `a_fatal_answer_is_marked_outcome_unknown`). The trigger is gated on a SESSION setting
//! (`cairn.test_kill_on`), so it is inert for every other connection even if a panic leaked it.
//! It sits on the projection table, not on `event_log`, because `projection_registry.rs` pins
//! the dispatcher as `event_log`'s only AFTER INSERT trigger.
//!
//! DB-gated on $CAIRN_TEST_PG, guard-before-connect (see `medication_signoff.rs`'s header).
mod common;

use cairn_event::SigningKey;
use cairn_node::db;
use cairn_node::medication::signoff::sign_off_medication_list;
use cairn_node::medication::{assert_medication, AssertMedicationInput, AttestParams};
use common::{attestation_count, cs, medication_setup as setup};
use tokio_postgres::Client;
use uuid::Uuid;

/// The trigger and its function: create (healing a leaked copy first) or drop.
const DROP_KILL_SWITCH: &str = "
    DROP TRIGGER IF EXISTS cairn_test_kill_on_nth_attestation ON medication_attestation;
    DROP FUNCTION IF EXISTS cairn_test_kill_on_nth_attestation();";

const CREATE_KILL_SWITCH: &str = "
    CREATE FUNCTION cairn_test_kill_on_nth_attestation() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
    DECLARE
        kill_on text := current_setting('cairn.test_kill_on', true);
        seen int := coalesce(nullif(current_setting('cairn.test_kill_seen', true), ''), '0')::int + 1;
    BEGIN
        IF coalesce(kill_on, '') = '' THEN
            RETURN NULL;  -- not this session's test: inert
        END IF;
        -- Session-level (is_local = false): survives the commit of the lines before the Nth.
        PERFORM set_config('cairn.test_kill_seen', seen::text, false);
        IF seen >= kill_on::int THEN
            PERFORM pg_terminate_backend(pg_backend_pid());
        END IF;
        RETURN NULL;
    END $$;
    CREATE CONSTRAINT TRIGGER cairn_test_kill_on_nth_attestation
        AFTER INSERT ON medication_attestation
        DEFERRABLE INITIALLY DEFERRED
        FOR EACH ROW EXECUTE FUNCTION cairn_test_kill_on_nth_attestation();";

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

/// Three lines; the second line's COMMIT is answered FATAL. The first is signed and reported
/// so; the second's outcome is NOT known and is named with its event; the third was never tried.
#[tokio::test]
async fn a_commit_lost_mid_sign_off_reports_the_signed_the_unknown_and_the_untried() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
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
    c.batch_execute(CREATE_KILL_SWITCH).await.unwrap();
    c.batch_execute("SET cairn.test_kill_on = '2'")
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

    // The killed connection is gone: look and clean up on a new one.
    let look = db::connect(&base).await.unwrap();
    look.batch_execute(DROP_KILL_SWITCH).await.unwrap();

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
        stopped.unknown_event.is_some(),
        "a FATAL answer to COMMIT is outcome-unknown: {}",
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
