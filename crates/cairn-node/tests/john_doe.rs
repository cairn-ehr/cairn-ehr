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

    let (pid, _call, _ord) = john_doe::register_john_doe(
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

    let (pid, call, _ord) = john_doe::register_john_doe(
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
    let (p1, c1, _ord) = john_doe::register_john_doe(
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
    let (p2, c2, _ord) = john_doe::register_john_doe(
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

// --- finisher 1: node-local friendly ordinal ---

/// Registration returns a per-node_origin ordinal (1, 2, …) and a foreign node_origin's
/// registrations form their OWN partition, never shifting this node's numbers. Proves the
/// VIEW is node-scoped without any `local_node` dependency, and that only callsign
/// registrations count (the count equals the number of John Does, not their events).
#[tokio::test]
async fn ordinal_numbers_registrations_per_node_origin() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;

    // Two John Does first-recorded on node "n" → ordinals 1 then 2, in registration order.
    let (_p1, _c1, o1) =
        john_doe::register_john_doe(&mut c, &sk, &kid, "n", "ED", "s", "2026-07-11", "b")
            .await
            .unwrap();
    let (p2, _c2, o2) =
        john_doe::register_john_doe(&mut c, &sk, &kid, "n", "ED", "s", "2026-07-11", "b")
            .await
            .unwrap();
    assert_eq!(o1, 1);
    assert_eq!(o2, 2);

    // A registration first-recorded on a DIFFERENT node_origin starts its own sequence at
    // 1 and does not shift node "n"'s ordinals.
    let (_p3, _c3, o3) =
        john_doe::register_john_doe(&mut c, &sk, &kid, "m", "ED", "s", "2026-07-11", "b")
            .await
            .unwrap();
    assert_eq!(o3, 1, "a different node_origin is a separate partition");

    // node "n"'s second John Doe still reads ordinal 2 via the VIEW.
    let n2: i64 = c
        .query_one(
            "SELECT ordinal FROM john_doe_local_ordinal WHERE patient_id = $1::text::uuid",
            &[&p2.to_string()],
        )
        .await
        .unwrap()
        .get("ordinal");
    assert_eq!(n2, 2);

    // Only callsign registrations are counted (each register authors ONE callsign name +
    // one pending marker; the pending marker is not a name → excluded). Three John Does
    // total across both partitions.
    let total: i64 = c
        .query_one("SELECT count(*) FROM john_doe_local_ordinal", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        total, 3,
        "only callsign name registrations appear in the VIEW"
    );
}
