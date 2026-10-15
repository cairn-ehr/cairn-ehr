//! R6 (#716): the per-link doubt read, `cairn_chart_set_doubts` (db/054), and the rule that
//! `cairn_chart_set_has_doubted_link` is EXACTLY "some doubt row exists" — so the sign-off hold
//! (R1b) and the window's per-link marks can never disagree.
//!
//! The three ways a set holds a doubted link (db/054's comment): (a) a `link_veto_flag` row, (b)
//! an un-attested standing link that trips the hard veto now, (c) a clinician's ATTESTED unlink
//! between two charts the set still joins. (a)+(b) are kind `clash`; (c) is `separated`. An
//! UN-attested unlink inside the set is deliberately not a doubt.
//!
//! DB-gated on $CAIRN_TEST_PG, serialized via `db::test_serial_guard`. Keys minted at runtime.
mod common;
use cairn_medication_view::ChartSet;
use cairn_node::chart_link::{link_charts, unlink_charts, LinkVerb, Reviewer};
use cairn_node::db;
use cairn_node::patient::edges::{record_doubts, record_is_doubted, DoubtKind};
use common::{
    apply_remote_raw, assert_clashing_dobs, cs, link_assertion_event, medication_setup as setup,
    register_pair, vetoed_pair,
};
use tokio_postgres::Client;
use uuid::Uuid;

const ORIGIN: &str = "r6-doubts";

async fn clean(c: &Client) {
    c.batch_execute("TRUNCATE patient_link, person_member, link_veto_flag")
        .await
        .unwrap();
}

/// db/054's boolean, asked directly.
async fn has_doubt(c: &Client, charts: &[Uuid]) -> bool {
    let ids: Vec<String> = charts.iter().map(Uuid::to_string).collect();
    c.query_one(
        "SELECT cairn_chart_set_has_doubted_link($1::text[]::uuid[])",
        &[&ids],
    )
    .await
    .unwrap()
    .get(0)
}

/// The per-link rows, as `(low, high, kind)`.
async fn doubts(c: &Client, charts: &[Uuid]) -> Vec<(Uuid, Uuid, String)> {
    let ids: Vec<String> = charts.iter().map(Uuid::to_string).collect();
    c.query(
        "SELECT low::text, high::text, kind FROM cairn_chart_set_doubts($1::text[]::uuid[]) \
         ORDER BY low, high",
        &[&ids],
    )
    .await
    .unwrap()
    .iter()
    .map(|r| {
        (
            r.get::<_, String>(0).parse().unwrap(),
            r.get::<_, String>(1).parse().unwrap(),
            r.get(2),
        )
    })
    .collect()
}

fn pair(a: Uuid, b: Uuid) -> (Uuid, Uuid) {
    (a.min(b), a.max(b))
}

/// (a)+(b): a peer machine's un-attested link of an already-vetoed pair is a `clash` on exactly
/// that pair. db/018 raises a flag as the link arrives, so this reaches BOTH clash arms at once —
/// the next test isolates (b).
#[tokio::test]
async fn an_unattested_link_that_trips_the_veto_is_a_clash_on_that_pair() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    clean(&c).await;
    let (sk, kid, _hsk, _hkid) = setup(&c).await;
    let (a, b) = vetoed_pair(&c, &sk, &kid).await;
    let ev = link_assertion_event(&kid, a, b, LinkVerb::Link, 50, 0, "peer-matcher", false);
    apply_remote_raw(&c, &sk, ev).await.unwrap();
    let (lo, hi) = pair(a, b);
    assert_eq!(
        doubts(&c, &[a, b]).await,
        vec![(lo, hi, "clash".to_string())]
    );
    assert!(has_doubt(&c, &[a, b]).await);
}

/// (b) alone, issue #220's path: the link arrives while nothing clashes, so db/018 raises no
/// flag, and the clashing dates come after. The pair is still a `clash`, because db/054
/// re-evaluates the veto at read time. Deleting the veto-now branch of the clash arm fails this.
#[tokio::test]
async fn a_link_the_veto_trips_only_later_is_still_a_clash() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    clean(&c).await;
    let (sk, kid, _hsk, _hkid) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk, &kid, a, b).await;
    let ev = link_assertion_event(&kid, a, b, LinkVerb::Link, 40, 0, "peer-matcher", false);
    apply_remote_raw(&c, &sk, ev).await.unwrap();
    assert!(
        doubts(&c, &[a, b]).await.is_empty(),
        "positive control: nothing clashes yet"
    );
    assert_clashing_dobs(&c, &sk, &kid, a, b, (50, 51)).await;
    let flags: i64 = c
        .query_one("SELECT count(*) FROM link_veto_flag", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        flags, 0,
        "precondition: #220 — the late clash raised no flag"
    );
    let (lo, hi) = pair(a, b);
    assert_eq!(
        doubts(&c, &[a, b]).await,
        vec![(lo, hi, "clash".to_string())]
    );
}

/// A human's attested link of the same vetoed pair is the decision the veto forces: no doubt.
#[tokio::test]
async fn a_confirmed_link_is_no_doubt() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    clean(&c).await;
    let (sk, kid, hsk, hkid) = setup(&c).await;
    let (a, b) = vetoed_pair(&c, &sk, &kid).await;
    let ev = link_assertion_event(&kid, a, b, LinkVerb::Link, 50, 0, "peer-matcher", false);
    apply_remote_raw(&c, &sk, ev).await.unwrap();
    let who = Reviewer {
        human_sk: &hsk,
        human_kid: &hkid,
    };
    link_charts(&mut c, a, b, &who, ORIGIN).await.unwrap();
    assert!(doubts(&c, &[a, b]).await.is_empty());
    assert!(!has_doubt(&c, &[a, b]).await);
}

/// (c), the A–C–X bridge: a human says A and X are different people while A–C and C–X still
/// join them. One `separated` row on A–X; the bridging links themselves are not clashes.
#[tokio::test]
async fn an_attested_unlink_the_set_still_joins_is_separated() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    clean(&c).await;
    let (sk, kid, hsk, hkid) = setup(&c).await;
    let (a, x) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk, &kid, a, x).await;
    let cc = Uuid::now_v7();
    common::submit_registration(&c, &sk, &kid, cc, 1).await;
    for (p, q, wall) in [(a, cc, 40), (cc, x, 41)] {
        let ev = link_assertion_event(&kid, p, q, LinkVerb::Link, wall, 0, "peer-matcher", false);
        apply_remote_raw(&c, &sk, ev).await.unwrap();
    }
    let who = Reviewer {
        human_sk: &hsk,
        human_kid: &hkid,
    };
    unlink_charts(&mut c, a, x, None, &who, ORIGIN)
        .await
        .unwrap();
    let (lo, hi) = pair(a, x);
    assert_eq!(
        doubts(&c, &[a, cc, x]).await,
        vec![(lo, hi, "separated".to_string())]
    );
    assert!(has_doubt(&c, &[a, cc, x]).await);
    // And the Rust read carries the kind and a recorded day.
    let set = ChartSet::new([a, cc, x]).unwrap();
    let read = record_doubts(&c, &set).await.unwrap();
    assert_eq!(read.len(), 1);
    assert_eq!(
        (read[0].low, read[0].high, read[0].kind),
        (lo, hi, DoubtKind::Separated)
    );
    assert_eq!(
        read[0].recorded_on.len(),
        10,
        "YYYY-MM-DD: {}",
        read[0].recorded_on
    );
}

/// An UN-attested unlink inside the set is deliberately not a doubt (db/054's NOT COVERED note).
#[tokio::test]
async fn an_unattested_unlink_inside_the_set_is_no_doubt() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    clean(&c).await;
    let (sk, kid, _hsk, _hkid) = setup(&c).await;
    let (a, x) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk, &kid, a, x).await;
    let cc = Uuid::now_v7();
    common::submit_registration(&c, &sk, &kid, cc, 1).await;
    for (p, q, wall) in [(a, cc, 40), (cc, x, 41)] {
        let ev = link_assertion_event(&kid, p, q, LinkVerb::Link, wall, 0, "peer-matcher", false);
        apply_remote_raw(&c, &sk, ev).await.unwrap();
    }
    let ev = link_assertion_event(&kid, a, x, LinkVerb::Unlink, 42, 0, "peer-matcher", false);
    apply_remote_raw(&c, &sk, ev).await.unwrap();
    assert!(doubts(&c, &[a, cc, x]).await.is_empty());
    assert!(!has_doubt(&c, &[a, cc, x]).await);
}

/// The equivalence, over the case matrix: the boolean is EXACTLY "a doubt row exists" — the
/// sign-off hold and the window's per-link marks can never disagree — and the Rust boolean
/// (`record_is_doubted`, the front door's and the tray's mark) is that same boolean.
///
/// db/054 spells the boolean as `EXISTS` over the rows, so this is a DEFINITIONAL pin: it fails
/// the day someone re-spells the boolean independently. That is why the matrix holds every kind
/// (none, clash, confirmed, single chart, separated) and an array carrying two doubts — the cases
/// where an independent spelling would drift first.
#[tokio::test]
async fn the_hold_is_exactly_some_doubt_row() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    clean(&c).await;
    let (sk, kid, hsk, hkid) = setup(&c).await;
    let who = Reviewer {
        human_sk: &hsk,
        human_kid: &hkid,
    };
    // 1. a plain un-attested link, no veto: no doubt.
    let (p, q) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk, &kid, p, q).await;
    apply_remote_raw(
        &c,
        &sk,
        link_assertion_event(&kid, p, q, LinkVerb::Link, 30, 0, "m", false),
    )
    .await
    .unwrap();
    // 2. a vetoed un-attested link: clash.
    let (a, b) = vetoed_pair(&c, &sk, &kid).await;
    apply_remote_raw(
        &c,
        &sk,
        link_assertion_event(&kid, a, b, LinkVerb::Link, 31, 0, "m", false),
    )
    .await
    .unwrap();
    // 3. a human-confirmed link: no doubt.
    let (d, e) = vetoed_pair(&c, &sk, &kid).await;
    link_charts(&mut c, d, e, &who, ORIGIN).await.unwrap();
    // 4. a single chart: no doubt.
    // 5. the bridge: a clinician separated f–x while f–g and g–x still join them: separated.
    let (f, x) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk, &kid, f, x).await;
    let g = Uuid::now_v7();
    common::submit_registration(&c, &sk, &kid, g, 1).await;
    for (l, r, wall) in [(f, g, 32), (g, x, 33)] {
        let ev = link_assertion_event(&kid, l, r, LinkVerb::Link, wall, 0, "m", false);
        apply_remote_raw(&c, &sk, ev).await.unwrap();
    }
    unlink_charts(&mut c, f, x, None, &who, ORIGIN)
        .await
        .unwrap();
    // 6. one array carrying two doubts (case 2's clash and case 5's separation).
    let matrix = [
        vec![p, q],
        vec![a, b],
        vec![d, e],
        vec![p],
        vec![f, g, x],
        vec![a, b, f, g, x],
    ];
    for set in matrix {
        let held = has_doubt(&c, &set).await;
        assert_eq!(
            held,
            !doubts(&c, &set).await.is_empty(),
            "the hold and the rows disagree over {set:?}"
        );
        let charts = ChartSet::new(set.iter().copied()).unwrap();
        assert_eq!(
            record_is_doubted(&c, &charts).await.unwrap(),
            held,
            "the Rust boolean and db/054 disagree over {set:?}"
        );
    }
    assert!(
        has_doubt(&c, &[a, b]).await && has_doubt(&c, &[f, g, x]).await,
        "cases 2 and 5 must be doubted, or the matrix proves nothing"
    );
    assert_eq!(
        doubts(&c, &[a, b, f, g, x]).await.len(),
        2,
        "case 6 must carry two doubts"
    );
}
