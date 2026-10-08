//! R5b (#680, #736, ADR-0078): the banner's `accepted` and `disputed` flags, and "Different
//! people" refusing to overrule an earlier human's "same person". DB-gated on $CAIRN_TEST_PG.
mod common;
use cairn_medication_view::ChartSet;
use cairn_node::chart_link::{LinkVerb, Reviewer};
use cairn_node::db;
use cairn_node::duplicate_review::{possible_duplicates, record_different_people, DifferentPeople};
use common::{
    apply_remote_raw, cs, enroll_human, link_assertion_event, register_pair, seed_proposal, setup,
};
use uuid::Uuid;

const TABLES: [&str; 5] = [
    "patient_link",
    "person_member",
    "identity_projection_flag",
    "link_veto_flag",
    "match_proposal",
];

#[tokio::test]
async fn the_banner_carries_accepted_and_disputed() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &TABLES).await;
    let (a, b, d) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk, &kid, a, b).await;
    register_pair(&c, &sk, &kid, d, Uuid::now_v7()).await;
    seed_proposal(&c, a, b, "accepted").await;
    seed_proposal(&c, a, d, "pending").await;
    let unlink = link_assertion_event(&kid, a, d, LinkVerb::Unlink, 50, 0, "peer", false);
    apply_remote_raw(&c, &sk, unlink).await.unwrap();
    let got = possible_duplicates(&c, &ChartSet::single(a)).await.unwrap();
    let to = |chart: Uuid| {
        got.iter()
            .find(|e| e.other_record.contains(&chart))
            .unwrap()
    };
    assert!(to(b).accepted && !to(b).disputed);
    assert!(to(d).disputed && !to(d).accepted);
}

/// Review Focus 3: even if the webview sent it, "Different people" never overrules an accepted
/// "same person" — nothing is signed.
#[tokio::test]
async fn different_people_refuses_an_accepted_pair_and_signs_nothing() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &TABLES).await;
    let (sk_h, kid_h) = enroll_human(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk, &kid, a, b).await;
    seed_proposal(&c, a, b, "accepted").await;
    let before: i64 = c
        .query_one("SELECT count(*) FROM event_log", &[])
        .await
        .unwrap()
        .get(0);
    let reviewer = Reviewer {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };
    let out = record_different_people(
        &mut c,
        &ChartSet::single(a),
        &ChartSet::single(b),
        &reviewer,
        "testnode",
    )
    .await
    .unwrap();
    assert!(matches!(out, DifferentPeople::AcceptedAsSame));
    let after: i64 = c
        .query_one("SELECT count(*) FROM event_log", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(after, before);
}
