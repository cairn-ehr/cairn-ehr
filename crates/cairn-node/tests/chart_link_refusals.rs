//! Repair path R2a — what a human's judgement on two charts REFUSES, and that a refusal moves
//! nothing: a pair with neither chart held here, a non-human key, a chart this node has never
//! seen, a chart linked to itself, and a floor refusal reaching the caller with its reason and
//! its step (ADR-0076 decisions 4 and 5).
//!
//! Split out of `chart_link.rs` (#810), whose helpers it copies (test helpers are file-local by
//! house convention). Real Postgres, gated on `$CAIRN_TEST_PG`, serialized via
//! `db::test_serial_guard`.
use cairn_event::{generate_key, SigningKey};
use cairn_node::chart_link::{link_charts, unlink_charts, LinkVerb, Reviewer};
use cairn_node::db;
use cairn_node::db_diagnosis::{refusal_scope, RefusalScope};
use tokio_postgres::Client;
use uuid::Uuid;

mod common;
use common::{apply_remote_raw, link_assertion_event, register_pair, seed_proposal};

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

const ORIGIN: &str = "r2a-test-node";

/// Clean identity + proposal state; enroll an agent (to register charts and play the
/// machine) and a human reviewer.
async fn setup(c: &Client) -> (SigningKey, String, SigningKey, String) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, patient_identifier, \
         patient_demographic, patient_link, person_member, identity_projection_flag CASCADE",
    )
    .await
    .unwrap();
    c.batch_execute(
        "DO $$ BEGIN \
           IF to_regclass('public.chart_dispute') IS NOT NULL THEN TRUNCATE chart_dispute; END IF; \
           IF to_regclass('public.chart_identity_state') IS NOT NULL THEN TRUNCATE chart_identity_state; END IF; \
           IF to_regclass('public.link_veto_flag') IS NOT NULL THEN TRUNCATE link_veto_flag; END IF; \
           TRUNCATE match_proposal; \
         END $$;",
    )
    .await
    .unwrap();
    let (sk_a, kid_a) = generate_key().unwrap();
    let (sk_h, kid_h) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('agent', '{\"model\":\"cl-stub\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
        &[&kid_a],
    )
    .await
    .unwrap();
    c.execute(
        "SELECT enroll_actor('human', '{\"role\":\"records-officer\",\"actor\":\"CL\"}', $1)",
        &[&kid_h],
    )
    .await
    .unwrap();
    (sk_a, kid_a, sk_h, kid_h)
}

async fn standing(c: &Client, a: Uuid, b: Uuid) -> Option<(String, bool)> {
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    c.query_opt(
        "SELECT state, attested FROM patient_link WHERE low = $1::text::uuid AND high = $2::text::uuid",
        &[&lo.to_string(), &hi.to_string()],
    )
    .await
    .unwrap()
    .map(|r| (r.get(0), r.get(1)))
}

async fn proposal(c: &Client, a: Uuid, b: Uuid) -> (String, Option<String>) {
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    let r = c
        .query_one(
            "SELECT status, applied_event_id::text FROM match_proposal \
             WHERE patient_low = $1::text::uuid AND patient_high = $2::text::uuid",
            &[&lo.to_string(), &hi.to_string()],
        )
        .await
        .unwrap();
    (r.get(0), r.get(1))
}

#[tokio::test]
async fn an_unlink_needs_at_least_one_of_the_charts_held_here() {
    // The other edge of finding 1's rule: two charts NEITHER of which is held here are not
    // something a clinician on this node has looked at, even if a peer's link joins them.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (x, y) = (Uuid::now_v7(), Uuid::now_v7());
    let peer = link_assertion_event(&kid_a, x, y, LinkVerb::Link, 50, 0, "peer-matcher", false);
    apply_remote_raw(&c, &sk_a, peer).await.unwrap();
    let who = Reviewer {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };
    let err = unlink_charts(&mut c, x, y, None, &who, ORIGIN)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains(&x.to_string()) && err.contains(&y.to_string()),
        "names both charts: {err}"
    );
    assert_eq!(
        standing(&c, x, y).await,
        Some(("link".into(), false)),
        "nothing was written"
    );
}

#[tokio::test]
async fn a_non_human_key_is_refused_and_nothing_moves() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, _sk_h, _kid_h) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk_a, &kid_a, a, b).await;
    seed_proposal(&c, a, b, "pending").await;

    let agent = Reviewer {
        human_sk: &sk_a,
        human_kid: &kid_a,
    };
    let err = link_charts(&mut c, a, b, &agent, ORIGIN).await.unwrap_err();
    // #702's class: a verdict must be MARKED, or the window words it as an outage ("try again").
    // NodeState: an operator's `enroll-human` makes the identical call succeed.
    assert_eq!(
        refusal_scope(&err),
        Some(RefusalScope::NodeState),
        "a non-human reviewer is a node-state verdict"
    );
    let err = err.to_string();
    assert!(err.contains("not an enrolled human"), "names why: {err}");
    assert_eq!(standing(&c, a, b).await, None, "no event landed");
    assert_eq!(
        proposal(&c, a, b).await.0,
        "pending",
        "the proposal did not move"
    );
}

#[tokio::test]
async fn a_chart_this_node_has_never_seen_is_refused_before_signing() {
    // A NEVER-LINKED stranger is refused for BOTH verbs: an unlink may name
    // a chart not held here only when it already reads as part of the other's record (see
    // `a_displayed_member_not_held_here_can_still_be_unlinked`).
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let a = Uuid::now_v7();
    common::submit_registration(&c, &sk_a, &kid_a, a, 1).await;
    let stranger = Uuid::now_v7();

    let who = Reviewer {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };
    for verb_is_link in [true, false] {
        let r = if verb_is_link {
            link_charts(&mut c, a, stranger, &who, ORIGIN).await
        } else {
            unlink_charts(&mut c, stranger, a, None, &who, ORIGIN).await
        };
        let err = r.unwrap_err();
        assert_eq!(
            refusal_scope(&err),
            Some(RefusalScope::NodeState),
            "a chart not held here is a node-state verdict: it succeeds once the chart arrives"
        );
        let err = err.to_string();
        assert!(
            err.contains(&stranger.to_string()),
            "names the unknown chart: {err}"
        );
    }
    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM event_log WHERE event_type LIKE 'identity.%link.asserted'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 0);
}

#[tokio::test]
async fn a_chart_cannot_be_linked_to_itself() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let a = Uuid::now_v7();
    common::submit_registration(&c, &sk_a, &kid_a, a, 1).await;
    let who = Reviewer {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };
    let err = link_charts(&mut c, a, a, &who, ORIGIN).await.unwrap_err();
    assert_eq!(
        refusal_scope(&err),
        Some(RefusalScope::Input),
        "the same chart twice is a verdict about the input, forever"
    );
    let err = err.to_string();
    assert!(err.contains("same chart"), "{err}");
}

#[tokio::test]
async fn a_floor_refusal_reaches_the_caller_with_its_reason_and_its_step() {
    // #702: a refusal from the in-DB floor must reach the operator naming BOTH the step that
    // met it and the server's reason — not a bare `db error`. Drive a real one past every
    // Rust pre-check: `stranger` has a `patient_chart` row (so this node reads it as held)
    // but no event history, and sorts LOW (minted first; v7 is time-ordered), so the event
    // is filed under it and db/005 step 8b refuses a first event that is not a registration.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let stranger = Uuid::now_v7();
    let a = Uuid::now_v7();
    assert!(
        stranger < a,
        "precondition: the historyless chart is the one filed under"
    );
    common::submit_registration(&c, &sk_a, &kid_a, a, 1).await;
    c.execute(
        "INSERT INTO patient_chart (patient_id) VALUES ($1::text::uuid)",
        &[&stranger.to_string()],
    )
    .await
    .unwrap();

    let who = Reviewer {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };
    let refused = link_charts(&mut c, a, stranger, &who, ORIGIN)
        .await
        .expect_err("db/005 step 8b refuses the event");
    let chain = format!("{refused:#}");
    assert!(
        chain.contains("submitting the judgement through the floor"),
        "names the step: {chain}"
    );
    assert!(
        chain.contains("the first event on a chart must be its registration"),
        "and the floor's reason: {chain}"
    );
    assert_eq!(standing(&c, a, stranger).await, None, "nothing was written");
}
