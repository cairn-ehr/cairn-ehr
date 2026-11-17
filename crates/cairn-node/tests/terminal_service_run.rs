//! #909 slice A, Task 7: one iteration of the terminal service's loop (`run::run_jobs`) does the
//! jobs that are due. The reconnect path is untested (needs a killed backend).
mod common;

use cairn_node::db;
use cairn_node::note::draft::save_part;
use cairn_node::terminal_service::run::{run_jobs, Due, JobState};
use cairn_node::terminal_service::startup::{can_claim, grant_verdict};
use cairn_patient_search::{CandidateList, SearchQuery};
use common::{cs, owner_and_window, service_login};
use uuid::Uuid;

#[tokio::test]
async fn a_requests_iteration_registers_a_waiting_request() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let (owner, window) = owner_and_window(&base).await;
    let (sk, _kid) = common::setup(&owner, &["patient_registration", "patient_name"]).await;
    owner
        .batch_execute("TRUNCATE registration_request")
        .await
        .unwrap();
    let mut service = db::connect(&service_login(&owner, &base).await)
        .await
        .unwrap();
    let id = Uuid::now_v7();
    let q = SearchQuery::new("Zed Runloop", None, &[]);
    window
        .query_one(
            "SELECT cairn_registration_submit($1::text::uuid, $2, $3::text::jsonb, $4::text::jsonb, $5)",
            &[
                &id.to_string(),
                &"Zed Runloop",
                &serde_json::to_string(&q).unwrap(),
                &serde_json::to_string(&CandidateList::empty()).unwrap(),
                &"2026-11-17",
            ],
        )
        .await
        .unwrap();
    let d = Due {
        requests: true,
        ..Due::default()
    };
    run_jobs(&mut service, &sk, "run-test", &d, &mut JobState::default()).await;
    let status: String = window
        .query_one(
            "SELECT status FROM cairn_registration_status($1::text::uuid)",
            &[&id.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(status, "registered");
}

#[tokio::test]
async fn a_commits_iteration_commits_an_overdue_draft() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let (owner, _window) = owner_and_window(&base).await;
    let (sk, _kid, _, _, chart) = common::note_setup(&owner).await;
    let mut service = db::connect(&service_login(&owner, &base).await)
        .await
        .unwrap();
    let draft = Uuid::now_v7();
    save_part(
        &owner,
        draft,
        chart,
        "ward-3-pc",
        Some("notewriter"),
        "obs stable",
        None,
        None,
    )
    .await
    .unwrap();
    // Age it past the quarantine, as terminal_service_commit.rs does.
    owner
        .execute(
            "WITH h AS (UPDATE note_draft SET last_keystroke = last_keystroke - interval '400 minutes', \
                               recording_started = recording_started - interval '400 minutes' \
                         WHERE draft_id = $1::text::uuid) \
             UPDATE note_draft_part SET last_keystroke = last_keystroke - interval '400 minutes', \
                    recording_started = recording_started - interval '400 minutes' \
              WHERE draft_id = $1::text::uuid",
            &[&draft.to_string()],
        )
        .await
        .unwrap();
    let d = Due {
        commits: true,
        ..Due::default()
    };
    run_jobs(&mut service, &sk, "run-test", &d, &mut JobState::default()).await;
    let n: i64 = owner
        .query_one(
            "SELECT count(*) FROM clinical_note WHERE patient_id = $1::text::uuid",
            &[&chart.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 1);
}

/// Final review F4 (b). The service checks at startup that its login can claim requests: a
/// window login (or any login without `cairn_terminal_service`) would otherwise start, say
/// "running", and fail every 250 ms while clerks' registrations time out. The refusal names the
/// owner step that makes the right login.
#[tokio::test]
async fn a_window_login_is_refused_as_the_terminal_service() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let (owner, window) = owner_and_window(&base).await;
    let service = db::connect(&service_login(&owner, &base).await)
        .await
        .unwrap();

    assert!(
        !can_claim(&window).await.unwrap(),
        "the window cannot claim"
    );
    let refusal = grant_verdict(can_claim(&window).await.unwrap()).unwrap_err();
    assert!(
        refusal.contains("provision-terminal-service-role"),
        "{refusal}"
    );
    assert!(
        can_claim(&service).await.unwrap(),
        "the service's login can"
    );
    assert!(grant_verdict(can_claim(&service).await.unwrap()).is_ok());
}
