//! Repair path R5a (#680): the banner's node read over db/057's `match_proposal_open`.
//! DB-gated on $CAIRN_TEST_PG; serialized via `db::test_serial_guard`; keys minted at runtime.
mod common;
use cairn_medication_view::ChartSet;
use cairn_node::chart_link::LinkVerb;
use cairn_node::db;
use cairn_node::duplicate_review::{open_pairs_between, possible_duplicates};
use common::{
    apply_remote_attested, apply_remote_raw, cs, enroll_human, link_assertion_event, register_pair,
    seed_proposal, setup, submit_link_event, submit_registration,
};
use uuid::Uuid;

const TABLES: [&str; 5] = [
    "patient_link",
    "person_member",
    "identity_projection_flag",
    "link_veto_flag",
    "match_proposal",
];

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

#[tokio::test]
async fn the_entry_shows_on_both_charts() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &TABLES).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk, &kid, a, b).await;
    seed_proposal(&c, a, b, "pending").await;
    let on_a = possible_duplicates(&c, &ChartSet::single(a)).await.unwrap();
    let on_b = possible_duplicates(&c, &ChartSet::single(b)).await.unwrap();
    assert_eq!(on_a.len(), 1);
    assert_eq!(on_a[0].review_chart, b);
    assert_eq!(on_b.len(), 1);
    assert_eq!(on_b[0].review_chart, a);
}

/// Review Focus 1 + 2: the proposal's in-record side is a linked MEMBER (m), not the opened
/// chart; and two members proposed against one other record make one entry.
#[tokio::test]
async fn a_members_proposals_show_once_per_other_record() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &TABLES).await;
    let (a, m, x, y) = (
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
    );
    register_pair(&c, &sk, &kid, a, m).await;
    register_pair(&c, &sk, &kid, x, y).await;
    submit_link_event(&c, &sk, &kid, a, m, 10, true).await; // my record: a + m
    submit_link_event(&c, &sk, &kid, x, y, 11, true).await; // the other record: x + y
    seed_proposal(&c, a, x, "pending").await;
    seed_proposal(&c, m, y, "review").await;
    let mine = ChartSet::new([a, m]).unwrap();
    let got = possible_duplicates(&c, &mine).await.unwrap();
    assert_eq!(got.len(), 1, "one other person, one entry");
    assert_eq!(got[0].other_record, ChartSet::new([x, y]).unwrap());
    let mut want = vec![(a.min(x), a.max(x)), (m.min(y), m.max(y))];
    want.sort();
    assert_eq!(got[0].pairs, want);
    assert_eq!(
        open_pairs_between(&c, &mine, &ChartSet::new([x, y]).unwrap())
            .await
            .unwrap(),
        want
    );
}

/// Review Focus 3, at the read: a peer's un-attested unlink leaves the entry; an attested one
/// clears it — with no local status write.
#[tokio::test]
async fn only_a_peers_attested_unlink_clears_the_entry() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &TABLES).await;
    let (sk_h, kid_h) = enroll_human(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk, &kid, a, b).await;
    seed_proposal(&c, a, b, "pending").await;
    let agent = link_assertion_event(&kid, a, b, LinkVerb::Unlink, now_ms(), 0, "peer", false);
    apply_remote_raw(&c, &sk, agent).await.unwrap();
    assert_eq!(
        possible_duplicates(&c, &ChartSet::single(a))
            .await
            .unwrap()
            .len(),
        1
    );
    let human = link_assertion_event(
        &kid_h,
        a,
        b,
        LinkVerb::Unlink,
        now_ms() + 1,
        0,
        "peer",
        true,
    );
    apply_remote_attested(&c, &sk_h, human, &sk_h, &kid_h)
        .await
        .unwrap();
    assert!(possible_duplicates(&c, &ChartSet::single(a))
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn a_chart_with_no_open_proposal_has_no_entry() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &TABLES).await;
    let a = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, a, 1).await;
    assert!(possible_duplicates(&c, &ChartSet::single(a))
        .await
        .unwrap()
        .is_empty());
}
