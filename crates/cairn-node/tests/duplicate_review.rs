//! Repair path R5a (#680): the banner's node read over db/057's `match_proposal_open`
//! (`possible_duplicates`, `open_proposals_touching`, `open_pairs_between`, the fresh veto
//! note) and the "Different people" judgement (`record_different_people`).
//! DB-gated on $CAIRN_TEST_PG; serialized via `db::test_serial_guard`; keys minted at runtime.
mod common;
use cairn_medication_view::ChartSet;
use cairn_node::chart_link::{LinkEffect, LinkVerb, Reviewer};
use cairn_node::db;
use cairn_node::duplicate_review::{
    open_pairs_between, open_proposals_touching, possible_duplicates, record_different_people,
    DifferentPeople,
};
use common::{
    apply_remote_attested, apply_remote_raw, cs, enroll_human, link_assertion_event, register_pair,
    seed_proposal, setup, submit_link_event, submit_registration, vetoed_pair,
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
    // The view alone cleared it: no local status write happened.
    assert_eq!(status_of(&c, a, b).await, "pending");
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

async fn status_of(c: &tokio_postgres::Client, a: Uuid, b: Uuid) -> String {
    let (lo, hi) = (a.min(b), a.max(b));
    c.query_one(
        "SELECT status FROM match_proposal WHERE patient_low = $1::text::uuid AND patient_high = $2::text::uuid",
        &[&lo.to_string(), &hi.to_string()],
    )
    .await
    .unwrap()
    .get(0)
}

/// Review Focus 1 + 2: every open pair between the two records gets an ATTESTED unlink — the
/// member's pair included, judged with `opened = None` — each proposal moves to `rejected`, and
/// the banner empties.
#[tokio::test]
async fn different_people_records_an_attested_unlink_on_every_open_pair() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
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
    submit_link_event(&c, &sk, &kid, a, m, 10, true).await;
    submit_link_event(&c, &sk, &kid, x, y, 11, true).await;
    seed_proposal(&c, a, x, "pending").await;
    seed_proposal(&c, m, y, "pending").await;
    let (mine, theirs) = (
        ChartSet::new([a, m]).unwrap(),
        ChartSet::new([x, y]).unwrap(),
    );
    let who = Reviewer {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };

    let out = record_different_people(&mut c, &mine, &theirs, &who, "r5a-test")
        .await
        .unwrap();
    let DifferentPeople::Judged(judged) = out else {
        panic!("two open pairs were judged")
    };
    assert_eq!(judged.len(), 2);
    for j in &judged {
        let effect = j.outcome.as_ref().expect("each unlink is recorded").effect;
        assert_eq!(effect, LinkEffect::TookEffect);
    }
    for (p, q) in [(a, x), (m, y)] {
        assert_eq!(status_of(&c, p, q).await, "rejected");
        let attested: bool = c
            .query_one(
                "SELECT attested FROM patient_link WHERE low = $1::text::uuid AND high = $2::text::uuid AND state = 'unlink'",
                &[&p.min(q).to_string(), &p.max(q).to_string()],
            )
            .await
            .unwrap()
            .get(0);
        assert!(attested, "a human's judgement, attested");
    }
    assert!(possible_duplicates(&c, &mine).await.unwrap().is_empty());
}

/// Review Focus 4: a colleague already resolved it — nothing is signed.
#[tokio::test]
async fn different_people_on_a_resolved_pair_records_nothing() {
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
    seed_proposal(&c, a, b, "pending").await;
    let peer = link_assertion_event(&kid_h, a, b, LinkVerb::Unlink, now_ms(), 0, "peer", true);
    apply_remote_attested(&c, &sk_h, peer, &sk_h, &kid_h)
        .await
        .unwrap();
    let before: i64 = c
        .query_one("SELECT count(*) FROM event_log", &[])
        .await
        .unwrap()
        .get(0);
    let who = Reviewer {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };
    let out = record_different_people(
        &mut c,
        &ChartSet::single(a),
        &ChartSet::single(b),
        &who,
        "r5a-test",
    )
    .await
    .unwrap();
    assert!(matches!(out, DifferentPeople::NothingOpen));
    let after: i64 = c
        .query_one("SELECT count(*) FROM event_log", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(before, after, "nothing signed");
}

/// `open_pairs_between` is symmetric in its arguments, and empty when no open proposal joins
/// the two records.
#[tokio::test]
async fn open_pairs_between_is_symmetric_and_empty_when_unjoined() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &TABLES).await;
    let (a, b, z) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk, &kid, a, b).await;
    submit_registration(&c, &sk, &kid, z, 2).await;
    seed_proposal(&c, a, b, "pending").await;
    let (sa, sb, sz) = (
        ChartSet::single(a),
        ChartSet::single(b),
        ChartSet::single(z),
    );
    let want = vec![(a.min(b), a.max(b))];
    assert_eq!(open_pairs_between(&c, &sa, &sb).await.unwrap(), want);
    assert_eq!(open_pairs_between(&c, &sb, &sa).await.unwrap(), want);
    assert!(open_pairs_between(&c, &sa, &sz).await.unwrap().is_empty());
}

/// Final review I3: the banner's veto note is the db/016 floor read NOW, never the proposal's
/// stored, propose-time `veto_findings`. `seed_proposal` stores `'[]'` — exactly the row a pair
/// that became vetoed AFTER the matcher proposed it carries (`auto_apply.rs` moves such a pair to
/// `review` and leaves the findings untouched). Read from the stored JSON, that pair would show
/// no note at all.
#[tokio::test]
async fn the_veto_note_is_the_floor_read_now_not_the_stored_findings() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &TABLES).await;
    let (v1, v2) = vetoed_pair(&c, &sk, &kid).await;
    seed_proposal(&c, v1, v2, "review").await; // stored findings: '[]'
    let (p1, p2) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk, &kid, p1, p2).await;
    seed_proposal(&c, p1, p2, "pending").await;

    let vetoed = open_proposals_touching(&c, &ChartSet::single(v1))
        .await
        .unwrap();
    assert_eq!(vetoed.len(), 1);
    assert!(vetoed[0].vetoed, "the DOBs clash now, whatever was stored");
    let plain = open_proposals_touching(&c, &ChartSet::single(p1))
        .await
        .unwrap();
    assert_eq!(plain.len(), 1);
    assert!(!plain[0].vetoed, "no recorded fact disagrees");
}

/// Final review I4: one pair's failure is CARRIED in its `PairJudgement` and the other pairs
/// still stand — `different_people_impl` words any `Err` from `record_different_people` as
/// "nothing was done", so a `?` in the loop would make that sentence false. The failing pair is
/// real, not mocked: `u` is never registered here, so its unlink is refused by admission (a
/// chart this node does not hold, sharing no record with `a`) before anything is signed.
#[tokio::test]
async fn one_pairs_failure_is_carried_and_the_others_stand() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &TABLES).await;
    let (sk_h, kid_h) = enroll_human(&c).await;
    let (a, x, u) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk, &kid, a, x).await;
    seed_proposal(&c, a, x, "pending").await;
    seed_proposal(&c, a, u, "pending").await; // u: not held on this node
    let who = Reviewer {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };

    let out = record_different_people(
        &mut c,
        &ChartSet::single(a),
        &ChartSet::new([x, u]).unwrap(),
        &who,
        "r5a-test",
    )
    .await
    .expect("the open pairs were read, so this is Ok whatever each pair did");
    let DifferentPeople::Judged(judged) = out else {
        panic!("two open pairs were judged")
    };
    assert_eq!(judged.len(), 2);
    let failed: Vec<_> = judged.iter().filter(|j| j.outcome.is_err()).collect();
    assert_eq!(failed.len(), 1, "exactly one pair failed");
    assert_eq!(
        (failed[0].low, failed[0].high),
        (a.min(u), a.max(u)),
        "the pair over the unheld chart"
    );
    let ok: Vec<_> = judged.iter().filter(|j| j.outcome.is_ok()).collect();
    assert_eq!(ok.len(), 1);
    assert_eq!((ok[0].low, ok[0].high), (a.min(x), a.max(x)));
    assert_eq!(
        ok[0].outcome.as_ref().unwrap().effect,
        LinkEffect::TookEffect
    );
    assert_eq!(status_of(&c, a, x).await, "rejected");
    assert_eq!(status_of(&c, a, u).await, "pending", "nothing moved it");

    // A second press (the button is kept for a "not confirmed" pair): only the pair still open
    // is judged — a–x is closed and is never signed twice — and the refused pair is refused
    // again before anything is signed.
    let before = event_count(&c).await;
    let again = record_different_people(
        &mut c,
        &ChartSet::single(a),
        &ChartSet::new([x, u]).unwrap(),
        &who,
        "r5a-test",
    )
    .await
    .unwrap();
    let DifferentPeople::Judged(again) = again else {
        panic!("a–u is still open")
    };
    assert_eq!(again.len(), 1, "only the pair still open");
    assert_eq!((again[0].low, again[0].high), (a.min(u), a.max(u)));
    assert!(again[0].outcome.is_err());
    assert_eq!(
        event_count(&c).await,
        before,
        "nothing signed on the re-press"
    );
}

async fn event_count(c: &tokio_postgres::Client) -> i64 {
    c.query_one("SELECT count(*) FROM event_log", &[])
        .await
        .unwrap()
        .get(0)
}

/// Different people judges the pairs between the TWO records compared — never another record's
/// proposal on the same banner. (A "simplification" to `open_proposals_touching(left)` would have
/// the clinician sign against every other person listed.)
#[tokio::test]
async fn different_people_leaves_a_third_records_proposal_open() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &TABLES).await;
    let (sk_h, kid_h) = enroll_human(&c).await;
    let (a, x, z) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk, &kid, a, x).await;
    submit_registration(&c, &sk, &kid, z, 2).await;
    seed_proposal(&c, a, x, "pending").await;
    seed_proposal(&c, a, z, "pending").await;
    let who = Reviewer {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };
    let out = record_different_people(
        &mut c,
        &ChartSet::single(a),
        &ChartSet::single(x),
        &who,
        "r5a-test",
    )
    .await
    .unwrap();
    let DifferentPeople::Judged(judged) = out else {
        panic!("a–x was open")
    };
    assert_eq!(judged.len(), 1, "a–x only");
    assert_eq!((judged[0].low, judged[0].high), (a.min(x), a.max(x)));
    assert_eq!(status_of(&c, a, z).await, "pending", "a–z untouched");
    let left = possible_duplicates(&c, &ChartSet::single(a)).await.unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].review_chart, z, "z's entry still stands");
}

/// Every other signing test has the displayed record on the LOW side (`now_v7` order). Here it
/// is the HIGH side: `x` is minted first, so the pair is `(x, a)` while `a` is on screen.
#[tokio::test]
async fn different_people_judges_a_pair_with_the_displayed_record_on_the_high_side() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &TABLES).await;
    let (sk_h, kid_h) = enroll_human(&c).await;
    let x = Uuid::now_v7();
    let a = Uuid::now_v7();
    assert!(x < a, "the displayed chart is the pair's high side");
    register_pair(&c, &sk, &kid, x, a).await;
    seed_proposal(&c, a, x, "pending").await;
    let who = Reviewer {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };
    let out = record_different_people(
        &mut c,
        &ChartSet::single(a),
        &ChartSet::single(x),
        &who,
        "r5a-test",
    )
    .await
    .unwrap();
    let DifferentPeople::Judged(judged) = out else {
        panic!("x–a was open")
    };
    assert_eq!(judged.len(), 1);
    assert_eq!((judged[0].low, judged[0].high), (x, a));
    assert_eq!(
        judged[0].outcome.as_ref().expect("recorded").effect,
        LinkEffect::TookEffect
    );
    assert_eq!(status_of(&c, a, x).await, "rejected");
}
