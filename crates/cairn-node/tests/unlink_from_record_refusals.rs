//! #699 (a)'s refusals: an unlink judged from an open chart is filed under that chart only when
//! the open chart's record holds BOTH charts of the pair. Without an open chart, with an open
//! chart unrelated to the pair, or with a third-chart filing whose record lacks the pair, the
//! judgement is refused and nothing is signed (the clinical picture: `unlink_from_record.rs`).
//!
//! Split out of `unlink_from_record.rs` (#810), whose helpers it copies (test helpers are
//! file-local by house convention). Real Postgres, gated on `$CAIRN_TEST_PG`, serialized via
//! `db::test_serial_guard`.
use cairn_event::{generate_key, SigningKey};
use cairn_node::chart_link::{
    assert_link_in_tx, canonical_pair, unlink_charts, FiledUnder, LinkVerb, Reviewer,
};
use cairn_node::db;
use cairn_node::db_diagnosis::{refusal_scope, RefusalScope};
use tokio_postgres::Client;
use uuid::Uuid;

mod common;
use common::{apply_remote_raw, link_assertion_event};

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

const ORIGIN: &str = "r2b2-unlink-from-record";

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

/// The pair's standing `patient_link` row as `(state, attested)`, or `None` if no assertion
/// about the pair exists. Either argument order.
async fn standing(c: &Client, x: Uuid, y: Uuid) -> Option<(String, bool)> {
    let (lo, hi) = canonical_pair(x, y);
    c.query_opt(
        "SELECT state, attested FROM patient_link WHERE low = $1::text::uuid AND high = $2::text::uuid",
        &[&lo.to_string(), &hi.to_string()],
    )
    .await
    .unwrap()
    .map(|r| (r.get(0), r.get(1)))
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

/// A held; B and C never registered here; a peer's machine links A–B and B–C (both filed under
/// A, the only chart with history here — as a peer holding A would file them).
async fn chain(c: &Client, sk: &SigningKey, kid: &str) -> (Uuid, Uuid, Uuid) {
    let (a, b, cc) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    common::submit_registration(c, sk, kid, a, 1).await;
    peer_link(c, sk, kid, a, b, a, 50).await;
    peer_link(c, sk, kid, b, cc, a, 51).await;
    (a, b, cc)
}

/// Every judgement-count check below asks one thing: how many local attested unlinks exist.
async fn attested_unlinks(c: &Client) -> i64 {
    c.query_one(
        "SELECT count(*) FROM event_log \
          WHERE event_type = 'identity.unlink.asserted' AND attestation IS NOT NULL",
        &[],
    )
    .await
    .unwrap()
    .get(0)
}

#[tokio::test]
async fn the_open_chart_must_hold_both_in_its_record() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (_a, b, cc) = chain(&c, &sk_a, &kid_a).await;
    let d = Uuid::now_v7();
    common::submit_registration(&c, &sk_a, &kid_a, d, 2).await;
    let who = Reviewer {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };

    let err = unlink_charts(&mut c, b, cc, Some(d), &who, ORIGIN)
        .await
        .expect_err("D is held, but its record does not hold B and C");
    // A held chart whose record lacks the pair is a stale picture (`RefusalScope::Stale`), not a
    // node that has yet to receive something: the same class the in-transaction re-check gives
    // the same fact, so the window re-reads, never waits (PR #711 review, #797).
    assert_eq!(
        refusal_scope(&err),
        Some(RefusalScope::Stale),
        "a verdict about the picture judged from"
    );
    let text = err.to_string();
    assert!(text.contains(&d.to_string()), "names the open chart");
    assert!(
        text.contains("does not read both") && text.ends_with(". Nothing was done"),
        "says why, and that nothing was done: {text}"
    );
    assert_eq!(
        standing(&c, b, cc).await,
        Some(("link".into(), false)),
        "nothing was written"
    );
    assert_eq!(attested_unlinks(&c).await, 0, "no judgement was signed");
}

#[tokio::test]
async fn without_an_open_chart_a_neither_held_unlink_is_still_refused() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (_a, b, cc) = chain(&c, &sk_a, &kid_a).await;
    let who = Reviewer {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };

    let err = unlink_charts(&mut c, b, cc, None, &who, ORIGIN)
        .await
        .expect_err("neither chart held, and no chart judged from");
    assert_eq!(refusal_scope(&err), Some(RefusalScope::NodeState));
    assert_eq!(
        standing(&c, b, cc).await,
        Some(("link".into(), false)),
        "nothing was written"
    );
    assert_eq!(attested_unlinks(&c).await, 0, "no judgement was signed");
}

#[tokio::test]
async fn an_open_chart_unrelated_to_the_pair_is_refused_even_when_a_subject_is_held() {
    // Ruling R5: `--from` is checked whenever it names a THIRD chart, not only when it decides
    // the filing. Here both subjects are held (so the event could be filed under one of them),
    // but the chart named as "judged from" is either a typo (no chart here at all) or a held
    // chart whose record does not contain the pair. Either way nothing is signed — otherwise
    // the result would report "chart <typo> now reads as: <typo>", a record that does not exist.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b, d) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    common::submit_registration(&c, &sk_a, &kid_a, a, 1).await;
    common::submit_registration(&c, &sk_a, &kid_a, b, 2).await;
    common::submit_registration(&c, &sk_a, &kid_a, d, 3).await;
    peer_link(&c, &sk_a, &kid_a, a, b, a, 50).await;
    let who = Reviewer {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };

    // A chart this node does not hold is a verdict about the node (sync may yet deliver it, as
    // for R2a's "not held here"); a held chart whose record lacks the pair is a verdict about the
    // picture judged from (PR #711 review). Each names the chart and says which it is.
    let typo = Uuid::now_v7();
    for (opened, scope, why) in [
        (typo, RefusalScope::NodeState, "is not held here"),
        (d, RefusalScope::Stale, "does not read both"),
    ] {
        let err = unlink_charts(&mut c, a, b, Some(opened), &who, ORIGIN)
            .await
            .expect_err("a judgement cannot be made from a chart unrelated to the pair");
        assert_eq!(refusal_scope(&err), Some(scope), "{opened}");
        let text = err.to_string();
        assert!(text.contains(&opened.to_string()), "names the open chart");
        assert!(text.contains(why), "says why: {text}");
    }
    assert_eq!(
        standing(&c, a, b).await,
        Some(("link".into(), false)),
        "nothing was written"
    );
    assert_eq!(attested_unlinks(&c).await, 0, "no judgement was signed");
}

#[tokio::test]
async fn a_third_chart_filing_is_refused_at_the_signing_core_when_its_record_lacks_the_pair() {
    // `assert_link_in_tx` is public (apply_proposal uses it; a future importer or façade may),
    // so the one rule a `RecordOf` filing rests on — the chart's record holds both subjects — is
    // checked THERE, not only by `judge`. Here the core is called directly with a held chart D
    // whose record is just {D}: without that check D would carry an identity event about two
    // strangers, at D's sensitivity grade (PR #711 review).
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (_a, b, cc) = chain(&c, &sk_a, &kid_a).await;
    let d = Uuid::now_v7();
    common::submit_registration(&c, &sk_a, &kid_a, d, 2).await;
    let who = Reviewer {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };
    let (lo, hi) = canonical_pair(b, cc);
    let hlc = db::next_hlc(&c, ORIGIN).await.unwrap();
    let tx = c.transaction().await.unwrap();
    let err = assert_link_in_tx(
        &tx,
        LinkVerb::Unlink,
        lo,
        hi,
        FiledUnder::RecordOf(d),
        None,
        "chart-review unlinked-by:test",
        None,
        &who,
        hlc,
    )
    .await
    .expect_err("D's record does not hold the pair");
    tx.rollback().await.unwrap();
    assert_eq!(refusal_scope(&err), Some(RefusalScope::Stale));
    let text = err.to_string();
    assert!(text.contains(&d.to_string()), "names the chart: {text}");
    assert_eq!(
        standing(&c, b, cc).await,
        Some(("link".into(), false)),
        "nothing was written"
    );
    assert_eq!(attested_unlinks(&c).await, 0, "no judgement was signed");
}
