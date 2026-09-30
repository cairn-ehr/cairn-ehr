//! R2b-2 Task 3: `record_edges` lists the STANDING links inside a chart set — the read behind the
//! window's per-link "Not the same person…" (ADR-0076 decision 4).
//!
//! Real Postgres, gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.
use cairn_event::{generate_key, SigningKey};
use cairn_medication_view::ChartSet;
use cairn_node::chart_link::{canonical_pair, link_charts, unlink_charts, LinkVerb, Reviewer};
use cairn_node::db;
use cairn_node::patient::edges::record_edges;
use cairn_node::patient::person::person_charts;
use tokio_postgres::Client;
use uuid::Uuid;

mod common;
use common::{apply_remote_raw, link_assertion_event};

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

const ORIGIN: &str = "r2b2-record-edges";

/// Clean identity + proposal state; enrol an agent (to register charts and play the peer's
/// machine) and a human reviewer. Copied from `tests/chart_link.rs` so this suite stays
/// self-contained (test helpers are file-local by house convention).
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
        "SELECT enroll_actor('agent', '{\"model\":\"r2b2-stub\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
        &[&kid_a],
    )
    .await
    .unwrap();
    c.execute(
        "SELECT enroll_actor('human', '{\"role\":\"records-officer\",\"actor\":\"R2B2\"}', $1)",
        &[&kid_h],
    )
    .await
    .unwrap();
    (sk_a, kid_a, sk_h, kid_h)
}

/// A peer machine's UN-attested link between `x` and `y`, filed under `filed` (the chart the
/// peer held) and admitted through the sync door at `wall`.
async fn peer_link(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    x: Uuid,
    y: Uuid,
    filed: Uuid,
    wall: i64,
) {
    let mut ev = link_assertion_event(kid, x, y, LinkVerb::Link, wall, 0, "peer-matcher", false);
    ev.patient_id = filed.to_string();
    apply_remote_raw(c, sk, ev)
        .await
        .expect("a peer's link is admitted");
}

/// A date shaped `YYYY-MM-DD`: ten characters, eight of them digits.
fn is_a_day(s: &str) -> bool {
    s.len() == 10 && s.chars().filter(char::is_ascii_digit).count() == 8
}

#[tokio::test]
async fn a_chain_has_two_links_each_with_its_standing() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b, cc) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    for (i, p) in [a, b, cc].into_iter().enumerate() {
        common::submit_registration(&c, &sk_a, &kid_a, p, 1 + i as i64).await;
    }
    let who = Reviewer {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };
    link_charts(&mut c, a, b, &who, ORIGIN)
        .await
        .expect("human link");
    peer_link(&c, &sk_a, &kid_a, b, cc, b, 60).await;

    let set = person_charts(&c, a).await.unwrap();
    let edges = record_edges(&c, &set).await.expect("edges read");
    assert_eq!(edges.len(), 2, "a chain of three charts has two links");

    let (l1, h1) = canonical_pair(a, b);
    let (l2, h2) = canonical_pair(b, cc);
    let mut want = [(l1, h1, true), (l2, h2, false)];
    want.sort_by_key(|(l, h, _)| (*l, *h));
    for (edge, (l, h, att)) in edges.iter().zip(want) {
        assert_eq!((edge.low, edge.high), (l, h), "canonical pair, ordered");
        assert_eq!(edge.attested, att, "the standing's attestation");
        assert!(is_a_day(&edge.recorded_on), "recorded_on is YYYY-MM-DD");
        if !att {
            // The peer's link carries a fixed wall of 60 ms: exactly the epoch's day, in UTC.
            assert_eq!(edge.recorded_on, "1970-01-01", "ms to UTC day, exactly");
        }
    }
}

#[tokio::test]
async fn an_unlinked_pair_is_not_a_link() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    common::submit_registration(&c, &sk_a, &kid_a, a, 1).await;
    common::submit_registration(&c, &sk_a, &kid_a, b, 2).await;
    let who = Reviewer {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };
    link_charts(&mut c, a, b, &who, ORIGIN).await.expect("link");
    unlink_charts(&mut c, a, b, None, &who, ORIGIN)
        .await
        .expect("unlink");

    // The set holding BOTH charts is the one that bites: both ends of the row are in the set, so
    // only `state = 'link'` can exclude it. (The both-ends test is pinned on its own by
    // `a_link_with_one_end_outside_the_set_is_not_listed`.)
    let both = ChartSet::new([a, b]).expect("two charts make a set");
    assert!(
        record_edges(&c, &both).await.unwrap().is_empty(),
        "an unlink row joins nothing even when both ends are in the set"
    );
    let alone = ChartSet::single(a);
    assert!(record_edges(&c, &alone).await.unwrap().is_empty());
    let set_b = person_charts(&c, b).await.unwrap();
    assert!(
        record_edges(&c, &set_b).await.unwrap().is_empty(),
        "an unlink row joins nothing"
    );
}

#[tokio::test]
async fn a_single_chart_has_no_links() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, _sk_h, _kid_h) = setup(&c).await;
    let a = Uuid::now_v7();
    common::submit_registration(&c, &sk_a, &kid_a, a, 1).await;
    let set = person_charts(&c, a).await.unwrap();
    assert!(record_edges(&c, &set).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_link_with_one_end_outside_the_set_is_not_listed() {
    // `record_edges` lists links whose BOTH ends are in the set (`low = ANY … AND high = ANY …`).
    // A standing link with one end outside must not appear: the pane would offer a link to a chart
    // that is not a member of the record shown, and the window's `standing_edge` check would
    // accept a pair half outside the displayed set. Chain A–B–C; ask for {A, B} and for {B, C}.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, _sk_h, _kid_h) = setup(&c).await;
    let (a, b, cc) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    common::submit_registration(&c, &sk_a, &kid_a, a, 1).await;
    peer_link(&c, &sk_a, &kid_a, a, b, a, 50).await;
    peer_link(&c, &sk_a, &kid_a, b, cc, a, 51).await;

    for (x, y) in [(a, b), (b, cc)] {
        let set = ChartSet::new([x, y]).expect("two charts make a set");
        let pairs: Vec<(Uuid, Uuid)> = record_edges(&c, &set)
            .await
            .expect("edges read")
            .iter()
            .map(|e| (e.low, e.high))
            .collect();
        assert_eq!(
            pairs,
            vec![canonical_pair(x, y)],
            "only the link inside the set is listed"
        );
    }
}
