//! ADR-0044 / issue #152 — enroll fail-closed on actor_id collision. actor_id is the
//! content-address of the PINNED set only (not the signing key, which must stay mutable
//! across rotate-key); two DISTINCT keys enrolled with an IDENTICAL pinned set collide
//! into one actor_id; actor_current's `DISTINCT ON (actor_id)` then silently drops the
//! earlier key (a silent identity merge — principle 2). enroll_actor now refuses it.
//! Real Postgres, gated on $CAIRN_TEST_PG, serialized cluster-wide.
use cairn_event::generate_key;
use cairn_node::db;
use tokio_postgres::Client;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// The RAISE message for a failed statement (Display renders only "db error"; the text
/// lives in the DbError payload — project convention, see suppression_owner_gate.rs).
fn db_msg(e: &tokio_postgres::Error) -> String {
    e.as_db_error()
        .map(|d| d.message().to_string())
        .unwrap_or_else(|| e.to_string())
}

/// Fresh actor registry for each test (isolation; the whole-history check would
/// otherwise see prior tests' committed rows).
async fn reset(c: &Client) {
    c.batch_execute("TRUNCATE event_log, actor_event, patient_chart CASCADE")
        .await
        .unwrap();
}

const ENROLL_HUMAN: &str = "SELECT enroll_actor('human', '{\"role\":\"clinician\"}', $1)";

#[tokio::test]
async fn distinct_key_same_pinned_is_refused() {
    let Some(cs) = cs() else { return };
    let _g = db::test_serial_guard(&cs).await.unwrap();
    let c = db::connect_and_load_schema(&cs).await.unwrap();
    reset(&c).await;
    let (_sk1, kid1) = generate_key().unwrap();
    let (_sk2, kid2) = generate_key().unwrap();
    // First human enrolls fine.
    c.execute(ENROLL_HUMAN, &[&kid1]).await.unwrap();
    // Second human, IDENTICAL pinned set, DIFFERENT key → same actor_id → refused.
    let err = c
        .execute(ENROLL_HUMAN, &[&kid2])
        .await
        .expect_err("colliding enroll must be refused");
    assert!(
        db_msg(&err).contains("different signing key"),
        "expected the collision RAISE, got: {}",
        db_msg(&err)
    );
    // The first key remains the sole current identity for that actor_id.
    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM actor_current WHERE signing_key_id = $1",
            &[&kid1],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 1, "the first-enrolled key must survive");
}

#[tokio::test]
async fn idempotent_same_key_re_enroll_is_allowed() {
    let Some(cs) = cs() else { return };
    let _g = db::test_serial_guard(&cs).await.unwrap();
    let c = db::connect_and_load_schema(&cs).await.unwrap();
    reset(&c).await;
    let (_sk, kid) = generate_key().unwrap();
    c.execute(ENROLL_HUMAN, &[&kid]).await.unwrap();
    // Same (pinned, key) again → allowed (re-runnable provisioning, matcher per-epoch re-enroll).
    c.execute(ENROLL_HUMAN, &[&kid])
        .await
        .expect("idempotent same-key re-enroll must be allowed");
}

#[tokio::test]
async fn distinct_pinned_sets_do_not_collide() {
    let Some(cs) = cs() else { return };
    let _g = db::test_serial_guard(&cs).await.unwrap();
    let c = db::connect_and_load_schema(&cs).await.unwrap();
    reset(&c).await;
    let (_sk1, kid1) = generate_key().unwrap();
    let (_sk2, kid2) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('human', '{\"role\":\"clinician\",\"actor\":\"A\"}', $1)",
        &[&kid1],
    )
    .await
    .unwrap();
    // Different pinned set → different actor_id → no false positive.
    c.execute(
        "SELECT enroll_actor('human', '{\"role\":\"clinician\",\"actor\":\"B\"}', $1)",
        &[&kid2],
    )
    .await
    .expect("distinct pinned sets must both enroll");
    let n: i64 = c
        .query_one("SELECT count(*) FROM actor_current", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 2, "two distinct actors must be current");
}

#[tokio::test]
async fn actor_id_is_immortal_after_revoke() {
    let Some(cs) = cs() else { return };
    let _g = db::test_serial_guard(&cs).await.unwrap();
    let c = db::connect_and_load_schema(&cs).await.unwrap();
    reset(&c).await;
    let (_sk1, kid1) = generate_key().unwrap();
    let (_sk2, kid2) = generate_key().unwrap();
    // Enroll, capture the actor_id, then revoke it.
    let aid: Vec<u8> = c.query_one(ENROLL_HUMAN, &[&kid1]).await.unwrap().get(0);
    c.execute(
        "INSERT INTO actor_event (actor_id, op) VALUES ($1, 'revoke')",
        &[&aid],
    )
    .await
    .unwrap();
    // A DIFFERENT key re-using that (now-revoked) actor_id is STILL refused — the
    // whole-history check enforces principle-2 immortality (no post-revoke reuse).
    let err = c
        .execute(ENROLL_HUMAN, &[&kid2])
        .await
        .expect_err("post-revoke reuse by a different key must be refused");
    assert!(
        db_msg(&err).contains("different signing key"),
        "expected the collision RAISE, got: {}",
        db_msg(&err)
    );
}

#[tokio::test]
async fn same_key_re_enroll_after_revoke_is_refused() {
    // Principle-2 immortality has a second edge the whole-history check must cover: a
    // fresh enroll of the ORIGINAL key onto a REVOKED actor_id must also be refused. A
    // post-revoke enroll would outrank the revoke in actor_current's (recorded_at, seq)
    // order and silently RESURRECT a retired actor — so the DB floor refuses it directly
    // (the same hazard matcher_actor.rs guards in Rust). The keyless revoke row makes the
    // conflict predicate fire even for the same key; this test pins that behaviour so a
    // later "IS NOT NULL tidy-up" of the predicate can't silently reopen resurrection.
    let Some(cs) = cs() else { return };
    let _g = db::test_serial_guard(&cs).await.unwrap();
    let c = db::connect_and_load_schema(&cs).await.unwrap();
    reset(&c).await;
    let (_sk, kid) = generate_key().unwrap();
    let aid: Vec<u8> = c.query_one(ENROLL_HUMAN, &[&kid]).await.unwrap().get(0);
    c.execute(
        "INSERT INTO actor_event (actor_id, op) VALUES ($1, 'revoke')",
        &[&aid],
    )
    .await
    .unwrap();
    // SAME key, now-revoked actor_id → refused (no resurrection).
    let err = c
        .execute(ENROLL_HUMAN, &[&kid])
        .await
        .expect_err("same-key re-enroll after revoke must be refused");
    assert!(
        db_msg(&err).contains("issue #152"),
        "expected the collision RAISE, got: {}",
        db_msg(&err)
    );
}
