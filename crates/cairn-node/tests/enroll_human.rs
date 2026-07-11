//! §5.4 — the human-actor enrollment ceremony library path. DB-gated on $CAIRN_TEST_PG,
//! serialized cluster-wide via db::test_serial_guard (shared-DB + TRUNCATE pattern, like
//! identify.rs / attestation.rs). Proves the dual-mapping guard, the ADR-0044 collision
//! refusal, and idempotency — the guarantees that keep the actor trust-anchor sound.
use cairn_event::generate_key;
use cairn_node::db;
use cairn_node::enroll::{build_human_pinned, enroll_human_actor, EnrollHumanOutcome};
use cairn_node::identify::attester_is_enrolled_human;
use tokio_postgres::Client;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// Fresh registry for each test: truncate the append-only actor + event tables.
async fn reset(c: &Client) {
    c.batch_execute("TRUNCATE event_log, actor_event, patient_chart CASCADE")
        .await
        .unwrap();
}

/// Read the actor_id (BYTEA) bound to a signing key in actor_current, if any.
async fn actor_id_of(c: &Client, kid: &str) -> Option<Vec<u8>> {
    c.query_opt(
        "SELECT actor_id FROM actor_current WHERE signing_key_id = $1",
        &[&kid],
    )
    .await
    .unwrap()
    .map(|r| r.get(0))
}

#[tokio::test]
async fn enrolls_a_resolvable_human_actor() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    reset(&c).await;

    let (_sk, kid) = generate_key().unwrap();
    let pinned = build_human_pinned("clinician", Some("MED-001"), None).unwrap();
    let out = enroll_human_actor(&c, &kid, &pinned).await.unwrap();
    assert!(matches!(out, EnrollHumanOutcome::Enrolled));
    assert!(
        attester_is_enrolled_human(&c, &kid).await.unwrap(),
        "the enrolled key resolves as a kind='human' actor (the identify --link pre-check)"
    );
}

#[tokio::test]
async fn distinct_registration_ids_get_distinct_actor_ids() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    reset(&c).await;

    let (_a, kid_a) = generate_key().unwrap();
    let (_b, kid_b) = generate_key().unwrap();
    enroll_human_actor(
        &c,
        &kid_a,
        &build_human_pinned("clinician", Some("MED-001"), None).unwrap(),
    )
    .await
    .unwrap();
    enroll_human_actor(
        &c,
        &kid_b,
        &build_human_pinned("clinician", Some("MED-002"), None).unwrap(),
    )
    .await
    .unwrap();
    assert_ne!(
        actor_id_of(&c, &kid_a).await.unwrap(),
        actor_id_of(&c, &kid_b).await.unwrap(),
        "two clinicians with distinct registration ids are distinct actors"
    );
}

#[tokio::test]
async fn identical_determinant_distinct_keys_is_refused() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    reset(&c).await;

    let pinned = build_human_pinned("clinician", None, Some("dr-a")).unwrap();
    let (_a, kid_a) = generate_key().unwrap();
    let (_b, kid_b) = generate_key().unwrap();
    enroll_human_actor(&c, &kid_a, &pinned).await.unwrap();
    let err = enroll_human_actor(&c, &kid_b, &pinned).await.unwrap_err();
    assert!(
        err.to_string().contains("ADR-0044") || err.to_string().contains("registration-id"),
        "the second key with an identical determinant is refused with a legible hint: {err}"
    );
}

#[tokio::test]
async fn same_key_reenroll_is_idempotent() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    reset(&c).await;

    let (_sk, kid) = generate_key().unwrap();
    let pinned = build_human_pinned("clinician", Some("MED-001"), None).unwrap();
    let first = enroll_human_actor(&c, &kid, &pinned).await.unwrap();
    assert!(matches!(first, EnrollHumanOutcome::Enrolled));
    let again = enroll_human_actor(&c, &kid, &pinned).await.unwrap();
    assert!(
        matches!(again, EnrollHumanOutcome::AlreadyEnrolled),
        "re-running the same enrollment is a no-op, not a second actor_event row"
    );
    // The comment above claims "not a second actor_event row" — assert it directly. The
    // idempotent branch (guard 1) must return BEFORE calling the enroll_actor floor, so
    // exactly one actor_event row exists for this key after both calls.
    let count: i64 = c
        .query_one(
            "SELECT count(*) FROM actor_event WHERE signing_key_id = $1",
            &[&kid],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        count, 1,
        "the idempotent re-enroll must not append a second actor_event row"
    );
}

#[tokio::test]
async fn same_key_different_determinant_is_refused() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    reset(&c).await;

    // Enroll a key as a human under one determinant, then try to re-enroll the SAME key
    // under a DIFFERENT determinant. This computes a different actor_id (guard 1's
    // non-idempotent branch: same key, existing actor_id != new actor_id) — a genuinely
    // different person needs a fresh key; changing this human's OWN determinant is not a
    // re-enroll (it would be a future supersede/rotate operation, ADR-0011 §5).
    let (_sk, kid) = generate_key().unwrap();
    let first = build_human_pinned("clinician", None, Some("dr-a")).unwrap();
    enroll_human_actor(&c, &kid, &first).await.unwrap();

    let second = build_human_pinned("clinician", Some("MED-777"), None).unwrap();
    let err = enroll_human_actor(&c, &kid, &second).await.unwrap_err();
    assert!(
        err.to_string().contains("already enrolled"),
        "re-enrolling the same key under a different determinant is refused: {err}"
    );
}

#[tokio::test]
async fn key_already_enrolled_under_another_kind_is_refused() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    reset(&c).await;

    // Enroll the key as a `device` first (the registration-desk precedent), then try to add a
    // human actor to the SAME key — the dual-mapping guard must refuse (db/005 would otherwise
    // NULL that key's authorship node-wide).
    let (_sk, kid) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('device', '{\"role\":\"registration-desk\"}', $1)",
        &[&kid],
    )
    .await
    .unwrap();
    let pinned = build_human_pinned("clinician", Some("MED-001"), None).unwrap();
    let err = enroll_human_actor(&c, &kid, &pinned).await.unwrap_err();
    assert!(
        err.to_string().contains("already enrolled"),
        "a key already mapped to an actor cannot be re-mapped to a human: {err}"
    );
}
