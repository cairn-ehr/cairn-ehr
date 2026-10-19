//! #820: a lost COMMIT on the identity side, driven end to end through a real FATAL answer.
//!
//! #815/#817 mark the identity-side commits that write an event with
//! `db_diagnosis::commit_failure`, and a guard (`db_errors_stay_legible.rs`) counts one
//! `commit_failure(` per `.commit()` in each marked file — it counts, it does not check which id
//! is passed. What no test pinned is what the marker SAYS once a commit really is lost:
//!
//! - `register_john_doe`: a re-run after a lost commit mints a SECOND chart, so the marker must
//!   name the registration event, and the headline must carry the chart id the operator is told
//!   to search for.
//! - `register_patient`: the same hazard; the context names the chart.
//! - `assert_observed_evidence`: the marker names the event that would have landed
//!   (`signed[0].0`).
//! - `apply_auto_candidates` (#823): a lost link commit stops the batch with its summary, that
//!   pair named outcome-unknown. Two siblings are NOT commit losses: the same switch made
//!   `NOT DEFERRABLE` kills the backend inside the submit statement, and a second switch makes
//!   the per-pair HLC tick fail on a live connection — each must stop the run the same way.
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

use cairn_node::auto_apply;
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
         DROP FUNCTION IF EXISTS cairn_test_kill_at_commit() CASCADE;"
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
        while let Some(message) = std::future::poll_fn(|cx| connection.poll_message(cx)).await {
            // The error that ends the connection is forwarded too, so a failure reads as what
            // happened rather than as "the switch never fired". The receiver may be gone.
            match message {
                Ok(AsyncMessage::Notice(notice)) => {
                    let _ = tx.send(notice.message().to_string());
                }
                Ok(_) => {}
                Err(e) => {
                    let _ = tx.send(format!("(the connection ended: {e})"));
                    break;
                }
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
    let mut seen = Vec::new();
    while let Ok(message) = warnings.try_recv() {
        if let Some(ids) = message.strip_prefix(KILLED) {
            let (event, chart) = ids.split_once(' ').expect("the switch names two ids");
            return (
                Uuid::parse_str(event).expect("an event uuid"),
                Uuid::parse_str(chart).expect("a chart uuid"),
            );
        }
        seen.push(message);
    }
    panic!("no '{KILLED}' warning arrived; the connection reported: {seen:?}");
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

/// Two registered pairs, each with a pending auto-candidate proposal, as the matcher worker leaves
/// them: scored after the charts' re-check notices were drained (#763).
async fn seed_two_auto_candidates(look: &Client, sk: &cairn_event::SigningKey, kid: &str) {
    look.batch_execute("TRUNCATE match_proposal, patient_link, person_member")
        .await
        .unwrap();
    for _ in 0..2 {
        let (low, high) = {
            let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
            (a.min(b), a.max(b))
        };
        common::register_pair(look, sk, kid, low, high).await;
        look.execute(
            "DELETE FROM match_pending WHERE patient_id IN ($1::text::uuid, $2::text::uuid)",
            &[&low.to_string(), &high.to_string()],
        )
        .await
        .unwrap();
        look.execute(
            "INSERT INTO match_proposal (patient_low, patient_high, score_total, band, \
               veto_findings, evidence, matcher_version, status) \
             VALUES ($1::text::uuid, $2::text::uuid, 9.10, 'auto_candidate', '[]'::jsonb, \
               '[]'::jsonb, '0.3.0+lost', 'pending')",
            &[&low.to_string(), &high.to_string()],
        )
        .await
        .unwrap();
    }
}

/// #823: an auto-applied link whose COMMIT is lost stops the ceremony with its summary — that
/// pair named as outcome-unknown (not errored), the rest counted as not attempted — instead of
/// the next pair's first call aborting the run as a plain "connection closed". And the summary's
/// claim holds: a re-run is safe, and links every pair exactly once.
#[tokio::test]
async fn a_lost_auto_apply_commit_stops_the_run_with_its_summary() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let (sk, kid, look) = arm(&base, "patient_link").await;
    seed_two_auto_candidates(&look, &sk, &kid).await;
    let keys = tempfile::tempdir().unwrap();
    let (mut victim, mut warnings) = connect_hearing_warnings(&base).await;

    let outcome = auto_apply::apply_auto_candidates(&mut victim, keys.path(), None, "n").await;
    look.batch_execute(&drop_kill_switch("patient_link"))
        .await
        .unwrap();

    let summary = outcome.expect("a lost commit ends the run with its summary, not an outage");
    let (killed, _chart) = killed_event(&mut warnings);
    let unknown = summary
        .outcome_unknown
        .as_ref()
        .expect("the pair is named as outcome-unknown");
    assert_eq!(
        unknown.event_id, killed,
        "names the link event that was lost"
    );
    assert_eq!(
        summary.not_attempted, 1,
        "the second pair was not attempted"
    );
    assert_eq!((summary.applied, summary.errored), (0, 0));

    // The self-terminated commit rolled back, so a re-run links both pairs — once each.
    assert_reruns_link_each_pair_once(&base, keys.path(), &look).await;
}

/// #823 review: the connection can die somewhere other than the link's commit — here inside the
/// link's submit statement, which is not a commit and so not marked. The pair is counted errored,
/// and the run still stops with its summary instead of the next pair's first call ending it as a
/// plain "connection closed".
#[tokio::test]
async fn a_connection_lost_mid_pair_stops_the_run_with_its_summary() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let (sk, kid, look) = arm(&base, "patient_link").await;
    // The same switch, fired at the end of the INSERT statement rather than at COMMIT.
    look.batch_execute(&drop_kill_switch("patient_link"))
        .await
        .unwrap();
    look.batch_execute(
        &create_kill_switch("patient_link")
            .replace("DEFERRABLE INITIALLY DEFERRED", "NOT DEFERRABLE"),
    )
    .await
    .unwrap();
    seed_two_auto_candidates(&look, &sk, &kid).await;
    let keys = tempfile::tempdir().unwrap();
    let (mut victim, _warnings) = connect_hearing_warnings(&base).await;

    let outcome = auto_apply::apply_auto_candidates(&mut victim, keys.path(), None, "n").await;
    look.batch_execute(&drop_kill_switch("patient_link"))
        .await
        .unwrap();

    let summary = outcome.expect("a lost connection ends the run with its summary");
    assert!(summary.connection_lost);
    assert!(summary.outcome_unknown.is_none(), "no commit was reached");
    assert_eq!((summary.errored, summary.not_attempted), (1, 1));
    assert!(auto_apply::run_verdict(&summary).is_some());
    assert_reruns_link_each_pair_once(&base, keys.path(), &look).await;
}

/// The verdict's claim, checked: a healthy re-run links both pairs, and each exactly once.
async fn assert_reruns_link_each_pair_once(base: &str, keys: &std::path::Path, look: &Client) {
    let mut healthy = db::connect(base).await.unwrap();
    let again = auto_apply::apply_auto_candidates(&mut healthy, keys, None, "n")
        .await
        .unwrap();
    assert_eq!(again.applied, 2);
    let links: i64 = look
        .query_one(
            "SELECT count(*) FROM event_log WHERE event_type = 'identity.link.asserted'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(links, 2, "each pair linked exactly once");
}

/// Remove the HLC-tick switch (healing a leaked copy first).
const DROP_FAIL_HLC_TICK: &str = "DROP TRIGGER IF EXISTS cairn_test_fail_hlc_tick ON hlc_state;
     DROP FUNCTION IF EXISTS cairn_test_fail_hlc_tick() CASCADE;";

/// A switch that makes `node_hlc_tick()` FAIL, on a connection that stays open, for a session
/// that set `cairn.test_kill_at_commit`. It reads the client's statement (`current_query()`), so
/// the HLC merges an enrolment runs are untouched: only the tick fails.
const CREATE_FAIL_HLC_TICK: &str = "CREATE FUNCTION cairn_test_fail_hlc_tick() RETURNS trigger
    LANGUAGE plpgsql AS $$
    BEGIN
        IF coalesce(current_setting('cairn.test_kill_at_commit', true), '') = 'on'
           AND current_query() LIKE '%node_hlc_tick%' THEN
            RAISE EXCEPTION 'cairn-test: the HLC tick failed';
        END IF;
        RETURN NEW;
    END $$;
    CREATE TRIGGER cairn_test_fail_hlc_tick BEFORE UPDATE ON hlc_state
        FOR EACH ROW EXECUTE FUNCTION cairn_test_fail_hlc_tick();";

/// #827 review: the per-pair HLC tick can fail before the client has seen its connection close
/// (a FATAL routed to the tick's own request). Before the fix only a tick failure on an
/// ALREADY-closed client stopped the run; any other returned a bare error, the summary lost. Now
/// every tick failure stops the run with its summary: the tick raises nothing of its own (db/007),
/// so a failed tick is the session failing, and it would fail for every pair left.
#[tokio::test]
async fn a_failed_hlc_tick_stops_the_run_with_its_summary() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let (sk, kid, look) = arm(&base, "patient_link").await;
    look.batch_execute(&drop_kill_switch("patient_link"))
        .await
        .unwrap();
    look.batch_execute(DROP_FAIL_HLC_TICK).await.unwrap();
    look.batch_execute(CREATE_FAIL_HLC_TICK).await.unwrap();
    seed_two_auto_candidates(&look, &sk, &kid).await;
    let keys = tempfile::tempdir().unwrap();
    let (mut victim, _warnings) = connect_hearing_warnings(&base).await;

    let outcome = auto_apply::apply_auto_candidates(&mut victim, keys.path(), None, "n").await;
    look.batch_execute(DROP_FAIL_HLC_TICK).await.unwrap();

    let summary = outcome.expect("a failed tick ends the run with its summary");
    assert!(summary.connection_lost);
    assert_eq!(
        (summary.applied, summary.errored, summary.not_attempted),
        (0, 0, 2)
    );
    assert!(auto_apply::run_verdict(&summary).is_some());
    assert_reruns_link_each_pair_once(&base, keys.path(), &look).await;
}
