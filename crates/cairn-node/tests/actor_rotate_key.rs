//! GUI sign-in slice 2, Task 1 — the actor registry (db/004) learns `rotate-key`, and with it
//! `confirm`, `repudiate`, `grant-enroller` and `withdraw-enroller`.
//!
//! What these tests pin, in the order a junior reader meets them:
//!
//! * A **re-key keeps the actor**: `rotate-key` is a new row on the SAME `actor_id`, so
//!   `actor_current`'s latest-row-wins makes the old key non-current, while `actor_key_vouches`
//!   (the "did this key ever legitimately belong to the actor" read) keeps both.
//! * The **one-key-one-actor** floor (#166) covers a rotated-in key, or a re-key could hand an
//!   existing key to a second actor.
//! * A **superseded** actor is not current (#664), and a `supersede` row's shape is fixed by a
//!   per-op CHECK (#666).
//! * A **repudiated** latest key leaves the actor with no current key — the actor is NOT revoked,
//!   so a later re-key makes it current again.
//! * Key confirmation status has exactly ONE definition, `cairn_key_confirmation`.
//! * The 3-argument `enroll_actor` every existing caller uses still resolves.
//!
//! Real Postgres, gated on $CAIRN_TEST_PG, serialized by the shared advisory-lock guard.
use cairn_event::generate_key;
use cairn_node::db;
use tokio_postgres::Client;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// The RAISE / constraint message of a failed statement (Display renders only "db error").
fn db_msg(e: &tokio_postgres::Error) -> String {
    e.as_db_error()
        .map(|d| d.message().to_string())
        .unwrap_or_else(|| e.to_string())
}

/// A fresh registry per test: the whole-history guards would otherwise see earlier rows.
async fn reset(c: &Client) {
    c.batch_execute("TRUNCATE event_log, actor_event, patient_chart CASCADE")
        .await
        .unwrap();
}

/// A fresh, runtime-generated public-key id (hex). Never a literal (CLAUDE.md rule 6).
fn key() -> String {
    generate_key().unwrap().1
}

/// Enroll a human with a person-distinguishing handle through the 3-arg door; returns its actor_id.
async fn enroll(c: &Client, handle: &str, kid: &str) -> Vec<u8> {
    let pinned = format!("{{\"role\":\"clinician\",\"handle\":\"{handle}\"}}");
    c.query_one(
        "SELECT enroll_actor('human', $1::text::jsonb, $2)",
        &[&pinned, &kid],
    )
    .await
    .unwrap()
    .get(0)
}

/// Append one of the non-enrol ops through the single append door.
async fn append(
    c: &Client,
    aid: &[u8],
    op: &str,
    kid: Option<&String>,
    approved_by: Option<&String>,
) -> Result<u64, tokio_postgres::Error> {
    c.execute(
        "SELECT cairn_actor_append($1, $2, $3, $4, NULL)",
        &[&aid, &op, &kid, &approved_by],
    )
    .await
}

async fn current_keys(c: &Client, aid: &[u8]) -> Vec<String> {
    c.query(
        "SELECT signing_key_id FROM actor_current WHERE actor_id = $1",
        &[&aid],
    )
    .await
    .unwrap()
    .iter()
    .map(|r| r.get(0))
    .collect()
}

async fn vouching_keys(c: &Client, aid: &[u8]) -> Vec<String> {
    c.query(
        "SELECT signing_key_id FROM actor_key_vouches WHERE actor_id = $1 ORDER BY 1",
        &[&aid],
    )
    .await
    .unwrap()
    .iter()
    .map(|r| r.get(0))
    .collect()
}

async fn confirmation(c: &Client, kid: &str) -> String {
    c.query_one("SELECT cairn_key_confirmation($1)", &[&kid])
        .await
        .unwrap()
        .get(0)
}

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

#[tokio::test]
async fn a_rotated_key_is_no_longer_current() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    reset(&c).await;
    let (k1, k2) = (key(), key());
    let a = enroll(&c, "A", &k1).await;
    append(&c, &a, "rotate-key", Some(&k2), None).await.unwrap();

    assert_eq!(
        current_keys(&c, &a).await,
        vec![k2.clone()],
        "K2 is current"
    );
    assert_eq!(
        vouching_keys(&c, &a).await,
        sorted(vec![k1.clone(), k2.clone()]),
        "both keys still vouch for the same actor"
    );
    // The rotate-key row carries the actor's kind/pinned, copied from its enroll row.
    let kind: String = c
        .query_one("SELECT kind FROM actor_current WHERE actor_id = $1", &[&a])
        .await
        .unwrap()
        .get(0);
    assert_eq!(kind, "human");
}

#[tokio::test]
async fn a_rotated_in_key_cannot_be_enrolled_to_another_actor() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    reset(&c).await;
    let (k1, k2) = (key(), key());
    let a = enroll(&c, "A", &k1).await;
    append(&c, &a, "rotate-key", Some(&k2), None).await.unwrap();

    let pinned = "{\"role\":\"clinician\",\"handle\":\"B\"}";
    let err = c
        .execute(
            "SELECT enroll_actor('human', $1::text::jsonb, $2)",
            &[&pinned, &k2],
        )
        .await
        .expect_err("a rotated-in key must not bind a second actor (#166)");
    assert!(
        db_msg(&err).contains("different actor_id"),
        "{}",
        db_msg(&err)
    );

    // And the mirror: a key another actor holds cannot be rotated in.
    let k3 = key();
    let b = enroll(&c, "B", &k3).await;
    let err = append(&c, &b, "rotate-key", Some(&k2), None)
        .await
        .expect_err("rotating another actor's key in must be refused");
    assert!(
        db_msg(&err).contains("different actor_id"),
        "{}",
        db_msg(&err)
    );
}

#[tokio::test]
async fn a_superseded_actor_is_not_current() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    reset(&c).await;
    let (k1, k2) = (key(), key());
    let a = enroll(&c, "A", &k1).await;
    let b = enroll(&c, "B", &k2).await;

    // No door writes supersede yet; restore (db/052) replays one raw, so stage it raw.
    let err = c
        .execute(
            "INSERT INTO actor_event (actor_id, op, signing_key_id, superseded_by) \
             VALUES ($1, 'supersede', $2, $3)",
            &[&a, &k1, &b],
        )
        .await
        .expect_err("a supersede row carrying a key breaks the #666 convention");
    assert!(
        db_msg(&err).contains("actor_event_op_shape"),
        "{}",
        db_msg(&err)
    );

    c.execute(
        "INSERT INTO actor_event (actor_id, op, superseded_by) VALUES ($1, 'supersede', $2)",
        &[&a, &b],
    )
    .await
    .unwrap();
    assert!(
        current_keys(&c, &a).await.is_empty(),
        "a superseded actor is not current (#664)"
    );
    assert_eq!(current_keys(&c, &b).await, vec![k2]);
}

#[tokio::test]
async fn a_repudiated_latest_key_leaves_the_actor_without_a_current_key() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    reset(&c).await;
    let (k1, k2, k3, su) = (key(), key(), key(), key());
    let a = enroll(&c, "A", &k1).await;
    append(&c, &a, "rotate-key", Some(&k2), None).await.unwrap();
    append(&c, &a, "repudiate", Some(&k2), Some(&su))
        .await
        .unwrap();

    assert!(
        current_keys(&c, &a).await.is_empty(),
        "no fall-back to K1: the actor has no current key"
    );
    assert_eq!(
        vouching_keys(&c, &a).await,
        vec![k1.clone()],
        "K1 keeps vouching, K2 does not"
    );
    let revokes: i64 = c
        .query_one(
            "SELECT count(*) FROM actor_event WHERE actor_id = $1 AND op = 'revoke'",
            &[&a],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(revokes, 0, "the actor itself is not revoked");

    append(&c, &a, "rotate-key", Some(&k3), None).await.unwrap();
    assert_eq!(
        current_keys(&c, &a).await,
        vec![k3],
        "a proper re-key makes A current again"
    );
}

#[tokio::test]
async fn confirmation_has_one_definition() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    reset(&c).await;
    let (su, enroller, pre, conf, unconf, doubt) = (key(), key(), key(), key(), key(), key());
    let token: Vec<u8> = (0..16u8).map(|i| i.wrapping_mul(7)).collect();
    let approved = |handle: &'static str, kid: String, by: String| {
        let pinned = format!("{{\"role\":\"clinician\",\"handle\":\"{handle}\"}}");
        let c = &c;
        let token = token.clone();
        async move {
            let aid: Vec<u8> = c
                .query_one(
                    "SELECT enroll_actor('human', $1::text::jsonb, $2, $3, $4)",
                    &[&pinned, &kid, &by, &token],
                )
                .await
                .unwrap()
                .get(0);
            aid
        }
    };

    assert_eq!(confirmation(&c, &key()).await, "unknown");

    enroll(&c, "PRE", &pre).await; // 3-arg, approved_by NULL: outside the mechanism
    assert_eq!(confirmation(&c, &pre).await, "outside");

    let e = approved("E", enroller.clone(), su.clone()).await;
    let cf = approved("CF", conf.clone(), enroller.clone()).await;
    approved("UN", unconf.clone(), su.clone()).await;
    approved("DB", doubt.clone(), enroller.clone()).await;
    assert_eq!(confirmation(&c, &unconf).await, "unconfirmed");

    append(&c, &cf, "confirm", Some(&conf), Some(&su))
        .await
        .unwrap();
    assert_eq!(confirmation(&c, &conf).await, "confirmed");

    // Repudiating the enroller's key: the enroller reads repudiated; what it approved and nobody
    // confirmed reads doubtful; what a superuser already confirmed stays confirmed (the cascade).
    append(&c, &e, "repudiate", Some(&enroller), Some(&su))
        .await
        .unwrap();
    assert_eq!(confirmation(&c, &enroller).await, "repudiated");
    assert_eq!(confirmation(&c, &doubt).await, "doubtful");
    assert_eq!(confirmation(&c, &conf).await, "confirmed");
    assert_eq!(confirmation(&c, &unconf).await, "unconfirmed");

    // The approval token is stored as given.
    let stored: Vec<u8> = c
        .query_one(
            "SELECT approval FROM actor_event WHERE signing_key_id = $1 AND op = 'enroll'",
            &[&unconf],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(stored, token);
}

#[tokio::test]
async fn the_append_door_refuses_what_it_does_not_own() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    reset(&c).await;
    let (k1, k2, su) = (key(), key(), key());
    let a = enroll(&c, "A", &k1).await;

    // enroll / supersede / revoke keep their own paths; anything outside the algebra is refused.
    for op in ["enroll", "supersede", "revoke", "merge"] {
        let err = append(&c, &a, op, Some(&k2), Some(&su))
            .await
            .expect_err("cairn_actor_append must refuse this op");
        assert!(
            db_msg(&err).contains("cairn_actor_append"),
            "{op}: {}",
            db_msg(&err)
        );
    }

    // A grant needs an approver (the per-op shape CHECK), and lands keyless.
    let err = append(&c, &a, "grant-enroller", None, None)
        .await
        .expect_err("a grant with no approver breaks the shape");
    assert!(
        db_msg(&err).contains("actor_event_op_shape"),
        "{}",
        db_msg(&err)
    );
    append(&c, &a, "grant-enroller", None, Some(&su))
        .await
        .unwrap();

    // Judging a key the actor never held is refused.
    let err = append(&c, &a, "confirm", Some(&k2), Some(&su))
        .await
        .expect_err("confirming a key this actor never held must be refused");
    assert!(
        db_msg(&err).contains("cairn_actor_append"),
        "{}",
        db_msg(&err)
    );
}

#[tokio::test]
async fn a_rotation_never_resurrects() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    reset(&c).await;
    let (k1, k2, k3) = (key(), key(), key());
    let a = enroll(&c, "A", &k1).await;
    append(&c, &a, "rotate-key", Some(&k2), None).await.unwrap();

    // A rotated-away key never comes back, on this actor either.
    let err = append(&c, &a, "rotate-key", Some(&k1), None)
        .await
        .expect_err("re-rotating a retired key must be refused");
    assert!(
        db_msg(&err).contains("cairn_actor_append"),
        "{}",
        db_msg(&err)
    );

    // A rotate-key after a revoke would outrank the revoke in actor_current: refused.
    c.execute(
        "INSERT INTO actor_event (actor_id, op) VALUES ($1, 'revoke')",
        &[&a],
    )
    .await
    .unwrap();
    let err = append(&c, &a, "rotate-key", Some(&k3), None)
        .await
        .expect_err("a rotation must not resurrect a revoked actor");
    assert!(
        db_msg(&err).contains("cairn_actor_append"),
        "{}",
        db_msg(&err)
    );
    assert!(current_keys(&c, &a).await.is_empty());
    assert!(
        vouching_keys(&c, &a).await.is_empty(),
        "a revoked actor's keys no longer vouch"
    );
}

#[tokio::test]
async fn the_three_arg_enroll_still_works() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    reset(&c).await;
    let kid = key();
    let pinned = format!("{{\"role\":\"registration-desk\",\"node_key\":\"{kid}\"}}");
    let aid: Vec<u8> = c
        .query_one(
            "SELECT enroll_actor('device', $1::text::jsonb, $2)",
            &[&pinned, &kid],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(current_keys(&c, &aid).await, vec![kid.clone()]);
    assert_eq!(confirmation(&c, &kid).await, "outside");
    // One signature only: the 3-arg form is gone, the 5-arg form (with defaults) serves it.
    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM pg_proc WHERE proname = 'enroll_actor'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 1, "exactly one enroll_actor overload");
}
