//! §5.4 finisher 3 — `identify` → optional link. DB-gated on $CAIRN_TEST_PG, serialized
//! cluster-wide via db::test_serial_guard (shared-DB + TRUNCATE pattern, like
//! attestation.rs / identity_linkage.rs). The human attester is enrolled via raw SQL here
//! (there is no enroll-human CLI yet — a separate future slice).
use cairn_event::{generate_key, SigningKey};
use cairn_node::db;
use cairn_node::identify::{
    attester_is_enrolled_human, identify_patient, IdentifyOutcome, LinkParams,
};
use tokio_postgres::Client;
use uuid::Uuid;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// Truncate the advisory-write tables and enroll the NODE key as a `device` registration
/// actor (so it may author the additive identify). Returns (node sk, node kid).
async fn setup_node(c: &Client) -> (SigningKey, String) {
    c.batch_execute("TRUNCATE event_log, actor_event, patient_chart CASCADE")
        .await
        .unwrap();
    c.batch_execute(
        "DO $$ BEGIN \
           IF to_regclass('public.patient_link')  IS NOT NULL THEN TRUNCATE patient_link;  END IF; \
           IF to_regclass('public.person_member') IS NOT NULL THEN TRUNCATE person_member; END IF; \
           IF to_regclass('public.chart_identity_state') IS NOT NULL THEN TRUNCATE chart_identity_state; END IF; \
         END $$;",
    )
    .await
    .unwrap();
    let (sk, kid) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('device', '{\"role\":\"registration-desk\"}', $1)",
        &[&kid],
    )
    .await
    .unwrap();
    (sk, kid)
}

/// Read the standing identity state for a subject ('pending' | 'identified' | None).
async fn identity_state(c: &Client, p: Uuid) -> Option<String> {
    c.query_opt(
        "SELECT state FROM chart_identity_state WHERE subject = $1::text::uuid",
        &[&p.to_string()],
    )
    .await
    .unwrap()
    .map(|r| r.get::<_, String>(0))
}

/// Read the effective trust_state for a chart, coalescing an absent row to 'confirmed'
/// (the chart_trust VIEW's default — a chart in the default state has no row).
async fn trust_state(c: &Client, p: Uuid) -> String {
    c.query_one(
        "SELECT COALESCE((SELECT trust_state FROM chart_trust WHERE patient_id = $1::text::uuid), 'confirmed')",
        &[&p.to_string()],
    )
    .await
    .unwrap()
    .get(0)
}

/// Author the pending marker for `patient` so the chart starts *unconfirmed* (reusing the
/// real register path would also mint a callsign; here we only need the pending state, so
/// author identify's counterpart directly through identify_patient's sibling is overkill —
/// instead drive the full flow: register a John Doe, then identify it).
async fn register_pending(c: &mut Client, sk: &SigningKey, kid: &str, node_origin: &str) -> Uuid {
    let (pid, _call, _ord) = cairn_node::john_doe::register_john_doe(
        c,
        sk,
        kid,
        node_origin,
        "ED",
        "site",
        "2026-07-11",
        "no ID",
    )
    .await
    .unwrap();
    pid
}

#[tokio::test]
async fn identify_alone_flips_chart_to_confirmed() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let node_origin = "test-node";

    let pid = register_pending(&mut c, &sk, &kid, node_origin).await;
    assert_eq!(identity_state(&c, pid).await.as_deref(), Some("pending"));
    assert_eq!(trust_state(&c, pid).await, "unconfirmed");

    let out: IdentifyOutcome = identify_patient(
        &mut c,
        &sk,
        &kid,
        node_origin,
        pid,
        "driver's licence",
        None,
    )
    .await
    .unwrap();
    assert!(out.link_event_id.is_none());
    assert_eq!(identity_state(&c, pid).await.as_deref(), Some("identified"));
    assert_eq!(trust_state(&c, pid).await, "confirmed");
}

/// Enroll a second key as a `human` actor (the attester), via raw SQL — there is no
/// enroll-human CLI yet. Returns (human sk, human kid).
async fn enroll_human(c: &Client) -> (SigningKey, String) {
    let (sk, kid) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('human', '{\"role\":\"clinician\",\"handle\":\"dr-a\"}', $1)",
        &[&kid],
    )
    .await
    .unwrap();
    (sk, kid)
}

/// Enroll a second key as a NON-human `agent` actor (for the atomicity/rejection test).
async fn enroll_agent(c: &Client) -> (SigningKey, String) {
    let (sk, kid) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('agent', '{\"model\":\"m\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
        &[&kid],
    )
    .await
    .unwrap();
    (sk, kid)
}

/// The person (connected-component) id for a chart, or None if it is in no link.
async fn person_of(c: &Client, p: Uuid) -> Option<Uuid> {
    c.query_opt(
        "SELECT person_id::text FROM person_member WHERE patient_id = $1::text::uuid",
        &[&p.to_string()],
    )
    .await
    .unwrap()
    .map(|r| r.get::<_, String>(0).parse().unwrap())
}

/// Count identify events on a subject (to prove atomicity rollback).
async fn identify_count(c: &Client, p: Uuid) -> i64 {
    c.query_one(
        "SELECT count(*) FROM event_log WHERE event_type = 'identity.identify.asserted' \
         AND body ->> 'subject' = $1",
        &[&p.to_string()],
    )
    .await
    .unwrap()
    .get(0)
}

#[tokio::test]
async fn identify_with_link_joins_prior_chart_and_confirms() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (h_sk, h_kid) = enroll_human(&c).await;
    let node_origin = "test-node";

    let doe = register_pending(&mut c, &sk, &kid, node_origin).await;
    let prior = Uuid::now_v7(); // a prior chart (need not pre-exist; offline-first)

    let out = identify_patient(
        &mut c,
        &sk,
        &kid,
        node_origin,
        doe,
        "family confirmation",
        Some(LinkParams {
            prior,
            human_sk: &h_sk,
            human_kid: &h_kid,
        }),
    )
    .await
    .unwrap();

    assert!(out.link_event_id.is_some(), "a link was requested");
    assert_eq!(identity_state(&c, doe).await.as_deref(), Some("identified"));
    assert_eq!(trust_state(&c, doe).await, "confirmed");
    // Both charts now sit in ONE person component (min-uuid canonical person).
    let expected = doe.min(prior);
    assert_eq!(person_of(&c, doe).await, Some(expected));
    assert_eq!(person_of(&c, prior).await, Some(expected));
}

#[tokio::test]
async fn link_with_non_human_attester_rolls_back_the_whole_op() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (a_sk, a_kid) = enroll_agent(&c).await; // NOT a human → attestation gate refuses
    let node_origin = "test-node";

    let doe = register_pending(&mut c, &sk, &kid, node_origin).await;
    let prior = Uuid::now_v7();

    let r = identify_patient(
        &mut c,
        &sk,
        &kid,
        node_origin,
        doe,
        "family confirmation",
        Some(LinkParams {
            prior,
            human_sk: &a_sk,
            human_kid: &a_kid,
        }),
    )
    .await;

    assert!(
        r.is_err(),
        "a non-human attester must be refused by the floor"
    );
    // Atomicity: the identify must NOT have committed — the chart stays *pending*.
    assert_eq!(identity_state(&c, doe).await.as_deref(), Some("pending"));
    assert_eq!(
        identify_count(&c, doe).await,
        0,
        "no identify event may survive the rollback"
    );
    assert_eq!(trust_state(&c, doe).await, "unconfirmed");
}

#[tokio::test]
async fn human_precheck_distinguishes_human_from_device_and_unenrolled() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (_sk, device_kid) = setup_node(&c).await; // enrolled as `device`
    let (_h_sk, human_kid) = enroll_human(&c).await;
    let (_u_sk, unenrolled_kid) = generate_key().unwrap();

    assert!(attester_is_enrolled_human(&c, &human_kid).await.unwrap());
    assert!(!attester_is_enrolled_human(&c, &device_kid).await.unwrap());
    assert!(!attester_is_enrolled_human(&c, &unenrolled_kid)
        .await
        .unwrap());
}
