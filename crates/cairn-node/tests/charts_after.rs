//! #797 — a judgement reports the record AS IT LEFT IT (`LinkOutcome::charts`): the set the
//! window then holds as its baseline, so the clinician's own link or unlink is never announced
//! back to them. Split out of `chart_link.rs` (already over 500 lines) as its own small binary.
//!
//! Real Postgres, gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`. That the
//! set is read INSIDE the judgement's transaction, after the apply and before the commit, is a
//! property of the source, pinned in `src/chart_link/judge_tests.rs`; these tests pin what the
//! read returns.
use cairn_event::{generate_key, SigningKey};
use cairn_node::chart_link::{link_charts, unlink_charts, Reviewer};
use cairn_node::db;
use tokio_postgres::Client;
use uuid::Uuid;

mod common;
use common::register_pair;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

const ORIGIN: &str = "charts-after-test-node";

/// Clean identity state; enroll an agent (to register charts) and a human reviewer. A trimmed
/// copy of `chart_link.rs`'s `setup` (no proposals are read here); `setup` is a repo-wide test
/// idiom, file-local by design (`identity_scaffolding_shared.rs`'s `REPO_WIDE`).
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
        "SELECT enroll_actor('agent', '{\"model\":\"ca-stub\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
        &[&kid_a],
    )
    .await
    .unwrap();
    c.execute(
        "SELECT enroll_actor('human', '{\"role\":\"records-officer\",\"actor\":\"CA\"}', $1)",
        &[&kid_h],
    )
    .await
    .unwrap();
    (sk_a, kid_a, sk_h, kid_h)
}

/// #797: the outcome's `charts` is the record AS THIS LINK LEFT IT — read inside the
/// judgement's own transaction, after its event applied (under db/018's link lock, CARNLK) and
/// before the commit. The window hands it back as the set the clinician "holds", so a re-read
/// never announces their own link to them. A set read BEFORE the event would be `{a}` alone, and
/// the re-read would name `b` as a change they did not make. A third, unrelated chart stays out.
#[tokio::test]
async fn a_link_reports_the_set_it_left() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk_a, &kid_a, a, b).await;
    register_pair(&c, &sk_a, &kid_a, Uuid::now_v7(), Uuid::now_v7()).await;

    let who = Reviewer {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };
    let out = link_charts(&mut c, a, b, &who, ORIGIN).await.expect("link");
    let mut left = vec![a, b];
    left.sort();
    assert_eq!(
        out.charts.members(),
        left.as_slice(),
        "the set the link left"
    );
}

/// #797, the unlink twin of `a_link_reports_the_set_it_left`: judged from `a`, the outcome is
/// `a`'s record as the unlink left it — `{a}` — not the `{a, b}` it read before the event.
#[tokio::test]
async fn an_unlink_reports_the_set_it_left() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk_a, &kid_a, a, b).await;

    let who = Reviewer {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };
    link_charts(&mut c, a, b, &who, ORIGIN).await.expect("link");
    let out = unlink_charts(&mut c, b, a, Some(a), &who, ORIGIN)
        .await
        .expect("unlink");
    assert_eq!(out.record_of, a, "judged from the open chart");
    assert_eq!(out.charts.members(), &[a], "the set the unlink left");
}
