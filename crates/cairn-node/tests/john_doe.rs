//! Integration coverage for §5.4 unidentified ("John Doe") registration (slice A):
//! `john_doe::register_john_doe` composes a callsign name assertion + the C4
//! identity-pending marker through the real `submit_event` door, so a chart is created
//! that (a) renders *unconfirmed* on `chart_trust`, and (b) carries the system-generated
//! callsign as a placeholder-use name in `patient_name` / `patient_name_current`. Real
//! Postgres, gated on `$CAIRN_TEST_PG`, serialized cluster-wide via `db::test_serial_guard`.
//! Mirrors `identity_identify.rs` (the C4 slice this composes onto).

use cairn_event::generate_key;
use cairn_node::{db, john_doe};
use tokio_postgres::Client;
use uuid::Uuid;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// Truncate the clinical + identity tables and enroll one agent signer. Returns (sk, kid).
/// `chart_identity_state` is created by db/024, so it is truncated behind a `to_regclass`
/// guard — keeping `setup()` correct even on a DB migrated only to an earlier stage.
async fn setup(c: &Client) -> (cairn_event::SigningKey, String) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, patient_identifier, \
         patient_demographic, patient_name CASCADE",
    )
    .await
    .unwrap();
    c.batch_execute(
        "DO $$ BEGIN \
           IF to_regclass('public.chart_identity_state') IS NOT NULL THEN TRUNCATE chart_identity_state; END IF; \
         END $$;")
        .await.unwrap();
    let (sk, kid) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('agent', '{\"model\":\"reg-stub\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
        &[&kid],
    ).await.unwrap();
    (sk, kid)
}

/// The effective trust state chart_trust reports for a subject, or None (== confirmed).
async fn trust_of(c: &Client, subject: Uuid) -> Option<String> {
    let s_s = subject.to_string();
    c.query_opt(
        "SELECT trust_state FROM chart_trust WHERE patient_id = $1::text::uuid",
        &[&s_s],
    )
    .await
    .unwrap()
    .map(|r| r.get::<_, String>(0))
}

/// The standing identity state of a chart (db/024 overlay), or None if no row exists.
async fn identity_state(c: &Client, subject: Uuid) -> Option<String> {
    let s_s = subject.to_string();
    c.query_opt(
        "SELECT state FROM chart_identity_state WHERE subject = $1::text::uuid",
        &[&s_s],
    )
    .await
    .unwrap()
    .map(|r| r.get::<_, String>(0))
}

/// The (use_key, value) of every retained name on a chart.
async fn names_of(c: &Client, subject: Uuid) -> Vec<(String, String)> {
    let s_s = subject.to_string();
    c.query(
        "SELECT use_key, value FROM patient_name WHERE patient_id = $1::text::uuid ORDER BY value",
        &[&s_s],
    )
    .await
    .unwrap()
    .iter()
    .map(|r| (r.get(0), r.get(1)))
    .collect()
}

/// The display-winner name for a chart (patient_name_current), or None.
async fn current_name(c: &Client, subject: Uuid) -> Option<String> {
    let s_s = subject.to_string();
    c.query_opt(
        "SELECT value FROM patient_name_current WHERE patient_id = $1::text::uuid",
        &[&s_s],
    )
    .await
    .unwrap()
    .map(|r| r.get::<_, String>(0))
}

// --- registration behaviour ---

#[tokio::test]
async fn register_john_doe_creates_an_unconfirmed_chart() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;

    let (pid, _call) = john_doe::register_john_doe(
        &mut c,
        &sk,
        &kid,
        "n",
        "ED",
        "site1",
        "2026-07-03",
        "unconscious ED arrival, no ID",
    )
    .await
    .expect("registration accepted by the floor");

    // §5.4: identity-pending is an active workflow state → chart renders *unconfirmed*.
    assert_eq!(identity_state(&c, pid).await.as_deref(), Some("pending"));
    assert_eq!(trust_of(&c, pid).await.as_deref(), Some("unconfirmed"));
}

#[tokio::test]
async fn callsign_is_stored_as_a_placeholder_use_name_and_is_the_display_winner() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;

    let (pid, call) = john_doe::register_john_doe(
        &mut c,
        &sk,
        &kid,
        "n",
        "ED",
        "site1",
        "2026-07-03",
        "unconscious ED arrival, no ID",
    )
    .await
    .unwrap();

    // The callsign lives in patient_name under the reserved 'callsign' use — that use is
    // what the advisory matcher excludes on, and it is what marks the name a placeholder.
    let names = names_of(&c, pid).await;
    assert_eq!(names, vec![("callsign".to_string(), call.clone())]);
    // With no legal name, db/012's unidentified-patient fallback makes the callsign the
    // display winner — the chart header shows the obvious placeholder, never a fake name.
    assert_eq!(current_name(&c, pid).await.as_deref(), Some(call.as_str()));
    assert!(
        call.starts_with("Unknown-"),
        "the header is an obvious placeholder: {call}"
    );
}

#[tokio::test]
async fn two_john_does_coexist_as_distinct_pending_charts_with_distinct_callsigns() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;

    // Same site, same day — the partition-safe suffix must still keep them apart.
    let (p1, c1) = john_doe::register_john_doe(
        &mut c,
        &sk,
        &kid,
        "n",
        "ED",
        "site1",
        "2026-07-03",
        "unconscious, no ID",
    )
    .await
    .unwrap();
    let (p2, c2) = john_doe::register_john_doe(
        &mut c,
        &sk,
        &kid,
        "n",
        "ED",
        "site1",
        "2026-07-03",
        "unresponsive trauma, no ID",
    )
    .await
    .unwrap();

    assert_ne!(p1, p2, "distinct UUIDs");
    assert_ne!(
        c1, c2,
        "distinct callsigns even at same site/day (suffix disambiguates)"
    );
    assert_eq!(trust_of(&c, p1).await.as_deref(), Some("unconfirmed"));
    assert_eq!(trust_of(&c, p2).await.as_deref(), Some("unconfirmed"));
}
