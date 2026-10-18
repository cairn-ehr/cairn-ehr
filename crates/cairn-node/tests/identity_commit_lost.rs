//! #820: a lost COMMIT on the identity side, driven end to end through a real FATAL answer.
//!
//! #815/#817 mark every identity-side commit with `db_diagnosis::commit_failure`, and a guard
//! (`db_errors_stay_legible.rs`) pins that each `.commit()` goes through it. What no test pinned
//! is what the marker SAYS once a commit really is lost:
//!
//! - `register_john_doe`: a re-run after a lost commit mints a SECOND chart, so the marker must
//!   name the registration event, and the headline must carry the chart id the operator is told
//!   to search for.
//! - `register_patient`: the same hazard; the context names the chart.
//! - `assert_observed_evidence`: the marker names the event that would have landed
//!   (`signed[0].0`).
//!
//! # How the commit is lost
//!
//! The `signoff_interrupted.rs` technique: a test-only DEFERRED constraint trigger on the
//! projection table the write fills, so it fires while the transaction COMMITs, and it makes
//! the backend terminate ITSELF. That is a real FATAL answer to a real COMMIT, which
//! `commit_failure` marks outcome-unknown. It is gated on a SESSION setting
//! (`cairn.test_kill_at_commit`), so it is inert for every other connection even if a panic
//! leaked it. It sits on a projection table, not on `event_log`: `projection_registry.rs` pins
//! the dispatcher as `event_log`'s only AFTER INSERT trigger.
//!
//! # How the test learns which event was killed
//!
//! The transaction rolls back, so nothing it wrote can be read afterwards. The orchestrators
//! mint their event ids internally, so the test cannot know them in advance either. So the
//! trigger says it: just before terminating, it raises a WARNING naming the event whose
//! projection row it fired on, and that event's chart, and the test's connection collects notices
//! (`connect_hearing_warnings`). A notice is sent at once, ahead of the FATAL.
//!
//! DB-gated on $CAIRN_TEST_PG, guard-before-connect.
mod common;

use cairn_node::db;
use cairn_node::db_diagnosis::commit_outcome_unknown_event;
use cairn_node::evidence::{self, AgeObservation, ObservedEvidence};
use cairn_node::john_doe;
use cairn_node::patient::register::register_patient;
use cairn_patient_search::{CandidateList, SearchQuery};
use common::cs;
use tokio::sync::mpsc;
use tokio_postgres::{AsyncMessage, Client, NoTls};
use uuid::Uuid;

/// Every warning starts with this, so it cannot be confused with a server warning.
const KILLED: &str = "cairn-test-killed-event:";

/// Remove the switch from `table` (healing a leaked copy first).
fn drop_kill_switch(table: &str) -> String {
    format!(
        "DROP TRIGGER IF EXISTS cairn_test_kill_at_commit ON {table};
         DROP FUNCTION IF EXISTS cairn_test_kill_at_commit();"
    )
}

/// Install the switch on `table`: at COMMIT, for a session that set `cairn.test_kill_at_commit`,
/// name the event behind the new row and its chart, then end this backend.
fn create_kill_switch(table: &str) -> String {
    format!(
        "CREATE FUNCTION cairn_test_kill_at_commit() RETURNS trigger
        LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
        DECLARE
            killed event_log;
        BEGIN
            IF coalesce(current_setting('cairn.test_kill_at_commit', true), '') <> 'on' THEN
                RETURN NULL;  -- not this session's test: inert
            END IF;
            SELECT * INTO killed FROM event_log WHERE content_address = NEW.content_address;
            RAISE WARNING '{KILLED}% %', killed.event_id, killed.patient_id;
            PERFORM pg_terminate_backend(pg_backend_pid());
            RETURN NULL;
        END $$;
        CREATE CONSTRAINT TRIGGER cairn_test_kill_at_commit
            AFTER INSERT ON {table}
            DEFERRABLE INITIALLY DEFERRED
            FOR EACH ROW EXECUTE FUNCTION cairn_test_kill_at_commit();"
    )
}

/// A connection whose server notices are kept, with the switch armed for its session.
///
/// `db::connect` drops notices (nothing in production reads them, #585), so this one polls the
/// connection itself and forwards each notice's message.
async fn connect_hearing_warnings(base: &str) -> (Client, mpsc::UnboundedReceiver<String>) {
    let (client, mut connection) = tokio_postgres::connect(base, NoTls).await.unwrap();
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Some(Ok(message)) = std::future::poll_fn(|cx| connection.poll_message(cx)).await {
            if let AsyncMessage::Notice(notice) = message {
                let _ = tx.send(notice.message().to_string());
            }
        }
    });
    client
        .batch_execute("SET cairn.test_kill_at_commit = 'on'")
        .await
        .unwrap();
    (client, rx)
}

/// The event the switch killed and its chart, read off the warnings received.
fn killed_event(warnings: &mut mpsc::UnboundedReceiver<String>) -> (Uuid, Uuid) {
    while let Ok(message) = warnings.try_recv() {
        if let Some(ids) = message.strip_prefix(KILLED) {
            let (event, chart) = ids.split_once(' ').expect("the switch names two ids");
            return (
                Uuid::parse_str(event).expect("an event uuid"),
                Uuid::parse_str(chart).expect("a chart uuid"),
            );
        }
    }
    panic!("the kill switch never fired: no '{KILLED}' warning arrived");
}

/// Load the schema, clear the fixtures and arm the switch on `table`. Returns the signer and a
/// healthy connection to look with (and to remove the switch with afterwards).
async fn arm(base: &str, table: &str) -> (cairn_event::SigningKey, String, Client) {
    let look = db::connect_and_load_schema(base).await.unwrap();
    let (sk, kid) = common::setup(&look, &["patient_name", "patient_registration"]).await;
    look.batch_execute(&drop_kill_switch(table)).await.unwrap();
    look.batch_execute(&create_kill_switch(table))
        .await
        .unwrap();
    (sk, kid, look)
}

/// A John Doe registration whose COMMIT is lost names its REGISTRATION event and the chart to
/// search for: a re-run would mint a second chart for the same patient.
#[tokio::test]
async fn a_lost_john_doe_commit_names_the_registration_and_the_chart() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let (sk, kid, look) = arm(&base, "patient_registration").await;
    let (mut victim, mut warnings) = connect_hearing_warnings(&base).await;

    let outcome = john_doe::register_john_doe(
        &mut victim,
        &sk,
        &kid,
        "n",
        "ED",
        "site1",
        "2026-10-18",
        "unconscious ED arrival, no ID",
    )
    .await;
    look.batch_execute(&drop_kill_switch("patient_registration"))
        .await
        .unwrap();

    let e = outcome.expect_err("the commit was answered FATAL");
    let (registration, patient) = killed_event(&mut warnings);
    assert_eq!(
        commit_outcome_unknown_event(&e),
        Some(registration),
        "the marker names the registration event: {e:#}"
    );
    assert_eq!(
        format!("{e}"),
        format!(
            "John Doe chart {patient} may have been created — search for it before registering \
             again"
        ),
        "the headline leads with the safe action and the chart id"
    );
}

/// A standard registration whose COMMIT is lost names its registration event and the chart.
#[tokio::test]
async fn a_lost_registration_commit_names_the_registration_and_the_chart() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let (sk, kid, look) = arm(&base, "patient_registration").await;
    let (mut victim, mut warnings) = connect_hearing_warnings(&base).await;

    let query = SearchQuery::new("Lost Commit", Some("1970-02-03"), &[]);
    let outcome = register_patient(
        &mut victim,
        &sk,
        &kid,
        "n",
        Some("Lost Commit"),
        &query,
        &CandidateList {
            people: vec![],
            incomplete: false,
            incomplete_reason: None,
        },
    )
    .await;
    look.batch_execute(&drop_kill_switch("patient_registration"))
        .await
        .unwrap();

    let e = outcome.expect_err("the commit was answered FATAL");
    let (registration, patient) = killed_event(&mut warnings);
    assert_eq!(
        commit_outcome_unknown_event(&e),
        Some(registration),
        "the marker names the registration event: {e:#}"
    );
    assert_eq!(format!("{e}"), format!("registering chart {patient}"));
}

/// Observed evidence whose COMMIT is lost names the event that would have landed.
#[tokio::test]
async fn a_lost_evidence_commit_names_its_event() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let (sk, kid, look) = arm(&base, "patient_demographic").await;
    let patient = Uuid::now_v7();
    // #345: a chart must be registered before anything is recorded about it.
    common::submit_registration(&look, &sk, &kid, patient, 0).await;
    let (mut victim, mut warnings) = connect_hearing_warnings(&base).await;

    let ev = ObservedEvidence {
        age: Some(AgeObservation {
            age_years: 40,
            tolerance_years: 5,
            basis: "dentition".into(),
        }),
        sex: None,
    };
    let outcome =
        evidence::assert_observed_evidence(&mut victim, &sk, &kid, "n", patient, &ev, 2026).await;
    look.batch_execute(&drop_kill_switch("patient_demographic"))
        .await
        .unwrap();

    let e = outcome.expect_err("the commit was answered FATAL");
    let (event, chart) = killed_event(&mut warnings);
    assert_eq!(chart, patient);
    assert_eq!(
        commit_outcome_unknown_event(&e),
        Some(event),
        "the marker names the evidence event: {e:#}"
    );
}
