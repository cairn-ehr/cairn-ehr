//! Repair path R5b (#680): the worklist's node reads over db/057's `match_proposal_open`.
//! DB-gated on $CAIRN_TEST_PG; serialized via `db::test_serial_guard`; keys minted at runtime.
mod common;
use cairn_node::chart_link::LinkVerb;
use cairn_node::db;
use cairn_node::duplicate_review::worklist::{worklist, worklist_count};
use common::{
    apply_remote_attested, apply_remote_raw, cs, enroll_human, link_assertion_event, register_pair,
    seed_proposal, setup, submit_link_event, vetoed_pair,
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

/// Review Focus 2: two of one record's charts against one other record are ONE entry, counted
/// once — and the count statement agrees with the list.
#[tokio::test]
async fn two_members_against_one_record_count_once() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &TABLES).await;
    let (a, m, x) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk, &kid, a, m).await;
    register_pair(&c, &sk, &kid, x, Uuid::now_v7()).await;
    submit_link_event(&c, &sk, &kid, a, m, 10, true).await; // one record: a + m
    seed_proposal(&c, a, x, "pending").await;
    seed_proposal(&c, m, x, "review").await;
    assert_eq!(worklist_count(&c).await.unwrap(), 1);
    let w = worklist(&c, 20).await.unwrap();
    assert_eq!(w.total, 1);
    assert_eq!(w.items.len(), 1);
    assert_eq!(w.items[0].entry.pairs.len(), 2);
}

/// The newer record is the one holding the latest-minted chart; Review opens it.
#[tokio::test]
async fn review_opens_the_newer_record() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &TABLES).await;
    let old = Uuid::now_v7();
    std::thread::sleep(std::time::Duration::from_millis(3)); // a later UUIDv7 millisecond
    let new = Uuid::now_v7();
    register_pair(&c, &sk, &kid, old, new).await;
    seed_proposal(&c, old, new, "pending").await;
    let w = worklist(&c, 20).await.unwrap();
    assert_eq!(w.items[0].entry.open_chart, new);
    assert!(w.items[0].newer.contains(&new) && w.items[0].older.contains(&old));
}

/// A peer's ATTESTED unlink, arriving by sync, clears the entry — at read time, no status write.
/// An UN-attested one leaves it, flagged `disputed` (ADR-0078).
#[tokio::test]
async fn only_an_attested_unlink_clears_the_entry_and_an_unattested_one_is_a_dispute() {
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
    let w = worklist(&c, 20).await.unwrap();
    assert_eq!(w.total, 1);
    assert!(w.items[0].entry.disputed);
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
    assert_eq!(worklist_count(&c).await.unwrap(), 0);
    assert_eq!(worklist(&c, 20).await.unwrap().total, 0);
}

/// `accepted` and `vetoed` each come from their own fixture; a pair in one record is never counted.
#[tokio::test]
async fn flags_and_the_one_record_case() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &TABLES).await;
    let (v1, v2) = vetoed_pair(&c, &sk, &kid).await;
    seed_proposal(&c, v1, v2, "review").await;
    let (p, q) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk, &kid, p, q).await;
    seed_proposal(&c, p, q, "accepted").await;
    let (s, t) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk, &kid, s, t).await;
    submit_link_event(&c, &sk, &kid, s, t, 10, true).await;
    seed_proposal(&c, s, t, "pending").await; // one record: never counted
    let w = worklist(&c, 20).await.unwrap();
    assert_eq!(worklist_count(&c).await.unwrap(), 2);
    assert_eq!(w.total, 2);
    let of = |chart: Uuid| {
        w.items
            .iter()
            .find(|i| i.entry.pairs.iter().any(|p| p.0 == chart || p.1 == chart))
            .unwrap()
    };
    assert!(of(v1).entry.vetoed && !of(v1).entry.accepted);
    assert!(of(p).entry.accepted && !of(p).entry.vetoed);
}

/// `limit` bounds the entries READ IN FULL, never `total`.
#[tokio::test]
async fn the_limit_bounds_the_items_not_the_total() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &TABLES).await;
    for _ in 0..3 {
        let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
        register_pair(&c, &sk, &kid, a, b).await;
        seed_proposal(&c, a, b, "pending").await;
    }
    let w = worklist(&c, 2).await.unwrap();
    assert_eq!((w.items.len(), w.total), (2, 3));
    assert_eq!(worklist_count(&c).await.unwrap(), 3);
}
