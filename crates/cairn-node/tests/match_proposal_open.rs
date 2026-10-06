//! db/057 (repair path R5a, #680): `match_proposal_open` is the ONE answer to "which proposals
//! still need a human". A pair leaves it when (1) both charts read as one record, or (2) an
//! ATTESTED unlink stands for it — never for an un-attested one: unlinks are not veto-gated and
//! the ADR-0030 agent writer can author one, so counting it would let any unreviewed writer
//! silently clear a duplicate banner (design page "R5a", the correction to the R5 bullets).
//!
//! DB-gated on $CAIRN_TEST_PG; serialized through `db::test_serial_guard`. Key material is
//! minted at runtime (house rule 6).
mod common;
use cairn_node::chart_link::{LinkVerb, OPEN_PROPOSAL_STATUSES};
use cairn_node::db;
use common::{
    apply_remote_attested, apply_remote_raw, cs, enroll_human, link_assertion_event, register_pair,
    seed_proposal, setup, submit_link_event, submit_registration,
};
use tokio_postgres::Client;
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

/// The open pairs, canonical, sorted.
async fn open_pairs(c: &Client) -> Vec<(Uuid, Uuid)> {
    c.query(
        "SELECT patient_low::text, patient_high::text FROM match_proposal_open ORDER BY 1, 2",
        &[],
    )
    .await
    .unwrap()
    .iter()
    .map(|r| {
        (
            r.get::<_, String>(0).parse().unwrap(),
            r.get::<_, String>(1).parse().unwrap(),
        )
    })
    .collect()
}

fn canon(a: Uuid, b: Uuid) -> (Uuid, Uuid) {
    (a.min(b), a.max(b))
}

#[test]
fn the_view_lists_exactly_chart_links_open_statuses() {
    // A guard and the thing it guards must be asked about the same string (HANDOVER's trap
    // list): pin the COMPOSED expression, not its pieces. If chart_link.rs gains or loses an
    // open status, the banner and the judgement writer would disagree on what "open" means.
    let sql = include_str!("../../../db/057_match_proposal_open.sql");
    let quoted: Vec<String> = OPEN_PROPOSAL_STATUSES
        .iter()
        .map(|s| format!("'{s}'"))
        .collect();
    let expected = format!("mp.status IN ({})", quoted.join(", "));
    assert!(sql.contains(&expected), "db/057 must contain `{expected}`");
}

#[tokio::test]
async fn open_statuses_are_listed_and_closed_ones_are_not() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &TABLES).await;
    let mut expected = vec![];
    for status in [
        "pending",
        "accepted",
        "review",
        "rejected",
        "applied",
        "auto_applied",
        "retracted",
    ] {
        let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
        register_pair(&c, &sk, &kid, a, b).await;
        seed_proposal(&c, a, b, status).await;
        if OPEN_PROPOSAL_STATUSES.contains(&status) {
            expected.push(canon(a, b));
        }
    }
    expected.sort();
    assert_eq!(open_pairs(&c).await, expected);
}

#[tokio::test]
async fn a_pair_already_in_one_record_is_not_open_directly_or_through_a_third_chart() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &TABLES).await;
    let (a, b, m, x, y) = (
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
    );
    register_pair(&c, &sk, &kid, a, b).await;
    submit_registration(&c, &sk, &kid, m, 1).await;
    register_pair(&c, &sk, &kid, x, y).await;
    seed_proposal(&c, a, b, "pending").await;
    seed_proposal(&c, x, y, "pending").await;
    // a–m–b: one record through m (an un-attested link moves no proposal status, so only the
    // component filter can hide a–b). x–y: directly linked.
    submit_link_event(&c, &sk, &kid, a, m, 10, true).await;
    submit_link_event(&c, &sk, &kid, m, b, 11, true).await;
    submit_link_event(&c, &sk, &kid, x, y, 12, true).await;
    assert_eq!(open_pairs(&c).await, vec![]);
}

#[tokio::test]
async fn only_an_attested_unlink_closes_a_pair() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &TABLES).await;
    let (sk_h, kid_h) = enroll_human(&c).await;
    let (a, b, x, y) = (
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
    );
    register_pair(&c, &sk, &kid, a, b).await;
    register_pair(&c, &sk, &kid, x, y).await;
    seed_proposal(&c, a, b, "pending").await;
    seed_proposal(&c, x, y, "pending").await;
    // Both arrive through the SYNC door, which never moves a local proposal's status — so the
    // view's patient_link filter is the only thing that can hide either pair.
    let human = link_assertion_event(&kid_h, a, b, LinkVerb::Unlink, now_ms(), 0, "peer", true);
    apply_remote_attested(&c, &sk_h, human, &sk_h, &kid_h)
        .await
        .expect("the peer's attested unlink lands");
    let agent = link_assertion_event(&kid, x, y, LinkVerb::Unlink, now_ms(), 0, "peer", false);
    apply_remote_raw(&c, &sk, agent)
        .await
        .expect("the peer's un-attested unlink lands");
    assert_eq!(
        open_pairs(&c).await,
        vec![canon(x, y)],
        "the attested unlink closes a–b; the agent's unlink must NOT close x–y"
    );
}

#[tokio::test]
async fn the_view_survives_a_schema_replay() {
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
    drop(c);
    let c = db::connect_and_load_schema(&base).await.unwrap();
    assert_eq!(open_pairs(&c).await, vec![canon(a, b)]);
}

/// The view is per PAIR, on purpose: an attested unlink between two of the records' charts does
/// not answer a DIFFERENT open pair between the same two records. Hiding the whole entry on the
/// first would bury a proposal no human answered — the banner's "absent means checked" rule.
#[tokio::test]
async fn an_attested_unlink_closes_only_its_own_pair() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &TABLES).await;
    let (sk_h, kid_h) = enroll_human(&c).await;
    let (a, m, x, y) = (
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
    );
    register_pair(&c, &sk, &kid, a, m).await;
    register_pair(&c, &sk, &kid, x, y).await;
    submit_link_event(&c, &sk, &kid, a, m, 10, true).await; // one record: a + m
    submit_link_event(&c, &sk, &kid, x, y, 11, true).await; // the other: x + y
    seed_proposal(&c, a, x, "pending").await;
    seed_proposal(&c, m, y, "pending").await;
    let human = link_assertion_event(&kid_h, a, x, LinkVerb::Unlink, now_ms(), 0, "peer", true);
    apply_remote_attested(&c, &sk_h, human, &sk_h, &kid_h)
        .await
        .expect("the peer's attested unlink of a–x lands");
    assert_eq!(
        open_pairs(&c).await,
        vec![canon(m, y)],
        "a–x is answered; m–y is not, and stays open"
    );
}

/// Hiding a pair because its charts read as one record is evaluated at READ time, never latched:
/// a–b hidden only through a third chart m comes back the moment m–b is unlinked. The db/057
/// header's "convergence without syncing match_proposal", in the direction that matters for
/// safety — a duplicate no human answered must never stay buried.
#[tokio::test]
async fn a_pair_hidden_through_a_third_chart_reappears_when_that_link_is_unlinked() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &TABLES).await;
    let (sk_h, kid_h) = enroll_human(&c).await;
    let (a, b, m) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk, &kid, a, b).await;
    submit_registration(&c, &sk, &kid, m, 1).await;
    seed_proposal(&c, a, b, "pending").await;
    submit_link_event(&c, &sk, &kid, a, m, 10, true).await;
    submit_link_event(&c, &sk, &kid, m, b, 11, true).await;
    assert_eq!(open_pairs(&c).await, vec![], "a–m–b: one record");
    let human = link_assertion_event(&kid_h, m, b, LinkVerb::Unlink, now_ms(), 0, "peer", true);
    apply_remote_attested(&c, &sk_h, human, &sk_h, &kid_h)
        .await
        .expect("the attested unlink of m–b lands");
    assert_eq!(
        open_pairs(&c).await,
        vec![canon(a, b)],
        "a and b are two records again, and nobody judged a–b"
    );
}

/// The view is not `security_invoker`: it reads `person_member` and `patient_link` with its
/// owner's rights. That adds no reach for `cairn_agent` only while db/017 and db/018 grant it
/// SELECT on every table the view reads — pinned here, so a later REVOKE on one of them cannot
/// turn the view into a quiet way round it.
#[tokio::test]
async fn cairn_agent_already_reads_every_table_the_view_reads() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    for table in [
        "match_proposal",
        "person_member",
        "patient_link",
        "match_proposal_open",
    ] {
        let ok: bool = c
            .query_one(
                "SELECT has_table_privilege('cairn_agent', $1, 'SELECT')",
                &[&table],
            )
            .await
            .unwrap()
            .get(0);
        assert!(ok, "cairn_agent must be able to SELECT {table}");
    }
}
