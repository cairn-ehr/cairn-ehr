//! R5b (#680, #736, ADR-0078): the banner's `accepted` and `disputed` flags, and "Different
//! people" refusing to overrule an earlier human's "same person". DB-gated on $CAIRN_TEST_PG.
mod common;
use cairn_medication_view::ChartSet;
use cairn_node::chart_link::{LinkVerb, Reviewer};
use cairn_node::db;
use cairn_node::duplicate_review::{possible_duplicates, record_different_people, DifferentPeople};
use cairn_node::patient::person::person_charts;
use common::{
    apply_remote_raw, cs, enroll_human, link_assertion_event, register_pair, seed_proposal, setup,
    submit_link_event,
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

/// Even if the webview sent it, "Different people" never overrules an accepted
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

/// ONE accepted pair among several between the same two records is enough to refuse: record
/// {a, m} against {x}, with (a, x) accepted and (m, x) still pending. Signing an unlink for the
/// pending pair alone would split a record a clinician already said is the same person as x.
#[tokio::test]
async fn different_people_refuses_when_any_pair_between_the_records_is_accepted() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &TABLES).await;
    let (sk_h, kid_h) = enroll_human(&c).await;
    let (a, m, x) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk, &kid, a, m).await;
    register_pair(&c, &sk, &kid, x, Uuid::now_v7()).await;
    submit_link_event(&c, &sk, &kid, a, m, 10, true).await; // one record: a + m
    seed_proposal(&c, a, x, "accepted").await;
    seed_proposal(&c, m, x, "pending").await;
    let left = person_charts(&c, a).await.unwrap();
    assert!(left.contains(&m), "a and m read as one record");
    let before: i64 = c
        .query_one("SELECT count(*) FROM event_log", &[])
        .await
        .unwrap()
        .get(0);
    let reviewer = Reviewer {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };
    let out = record_different_people(&mut c, &left, &ChartSet::single(x), &reviewer, "testnode")
        .await
        .unwrap();
    assert!(matches!(out, DifferentPeople::AcceptedAsSame));
    let after: i64 = c
        .query_one("SELECT count(*) FROM event_log", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        after, before,
        "nothing is signed, not even for the pending pair"
    );
}
