//! ADR-0076 decision 1: `cairn_person_charts` is the ONE answer to "which charts are this
//! person", read by every combined read. A chart never linked is a set of one; a link
//! component is read whole from any member; an unlink splits it again.
mod common;
use cairn_medication_view::ChartSet;
use cairn_node::db;
use cairn_node::patient::person::person_charts;
use common::{cs, medication_setup as setup, submit_link_event, submit_registration};
use uuid::Uuid;

async fn fresh(c: &tokio_postgres::Client, sk: &cairn_event::SigningKey, kid: &str) -> Uuid {
    let p = Uuid::now_v7();
    submit_registration(c, sk, kid, p, 0).await;
    p
}

#[tokio::test]
async fn a_never_linked_chart_is_a_set_of_one() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE patient_link, person_member")
        .await
        .unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let a = fresh(&c, &sk, &kid).await;
    assert_eq!(person_charts(&c, a).await.unwrap(), ChartSet::single(a));
}

#[tokio::test]
async fn a_linked_pair_is_one_set_from_either_side() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE patient_link, person_member")
        .await
        .unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let a = fresh(&c, &sk, &kid).await;
    let b = fresh(&c, &sk, &kid).await;
    submit_link_event(&c, &sk, &kid, a, b, 10, true).await;
    let both = ChartSet::new([a, b]).unwrap();
    assert_eq!(person_charts(&c, a).await.unwrap(), both);
    assert_eq!(
        person_charts(&c, b).await.unwrap(),
        both,
        "the same set from the other side"
    );
}

#[tokio::test]
async fn a_transitive_cluster_is_one_set_from_every_member() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE patient_link, person_member")
        .await
        .unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let a = fresh(&c, &sk, &kid).await;
    let b = fresh(&c, &sk, &kid).await;
    let x = fresh(&c, &sk, &kid).await;
    // a–b and b–x: a and x were never linked to each other, and are still one person.
    submit_link_event(&c, &sk, &kid, a, b, 10, true).await;
    submit_link_event(&c, &sk, &kid, b, x, 11, true).await;
    let all = ChartSet::new([a, b, x]).unwrap();
    for member in [a, b, x] {
        assert_eq!(
            person_charts(&c, member).await.unwrap(),
            all,
            "from {member}"
        );
    }
}

#[tokio::test]
async fn an_unlinked_chart_reads_alone_again() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE patient_link, person_member")
        .await
        .unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let a = fresh(&c, &sk, &kid).await;
    let b = fresh(&c, &sk, &kid).await;
    submit_link_event(&c, &sk, &kid, a, b, 10, true).await;
    submit_link_event(&c, &sk, &kid, a, b, 11, false).await;
    assert_eq!(person_charts(&c, a).await.unwrap(), ChartSet::single(a));
    assert_eq!(person_charts(&c, b).await.unwrap(), ChartSet::single(b));
    // Relinked: the set follows the standing edge, not the first one ever written.
    submit_link_event(&c, &sk, &kid, a, b, 12, true).await;
    assert_eq!(
        person_charts(&c, a).await.unwrap(),
        ChartSet::new([a, b]).unwrap()
    );
}
