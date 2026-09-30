//! #699 (a): an unlink where neither chart is held here, judged from an open chart whose record
//! holds both, is filed under that chart. And the audit's pins (R2b-2 plan): the pair comes from
//! the payload wherever the envelope files it.
//!
//! The clinical picture: this node holds chart A. A peer's matcher joined A–B and then B–C, so
//! A's record reads {A, B, C} here although B and C were never registered on this node. The
//! clinician, reading A, sees that C is not the same person as B. Before #699 (a) that unlink was
//! refused ("neither chart is held here"); now it is filed under A — the chart the judgement was
//! made FROM — because db/005 step 8b only admits a local event filed under a chart with history
//! here, and db/018 reads the pair from the payload, never from the envelope.
//!
//! Real Postgres, gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.
use cairn_event::{generate_key, sign, SigningKey};
use cairn_node::chart_link::{
    assert_link_in_tx, canonical_pair, unlink_charts, FiledUnder, LinkEffect, LinkVerb, Reviewer,
};
use cairn_node::db;
use cairn_node::db_diagnosis::{refusal_scope, RefusalScope};
use std::time::Duration;
use tokio::time::timeout;
use tokio_postgres::Client;
use uuid::Uuid;

mod common;
use common::{apply_remote_attested, apply_remote_raw, link_assertion_event};

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
async fn a_chain_split_from_the_opened_chart_took_effect() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b, cc) = chain(&c, &sk_a, &kid_a).await;
    let who = Reviewer {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };

    let out = unlink_charts(&mut c, b, cc, Some(a), &who, ORIGIN)
        .await
        .expect("the far link of A–B–C unlinks when judged from A");
    assert_eq!(out.filed_under, a, "filed under the chart judged from");
    assert_eq!(out.record_of, a, "the record shown is the opened chart's");
    assert_eq!(out.effect, LinkEffect::TookEffect);
    let mut expected = [a, b];
    expected.sort();
    assert_eq!(
        out.charts.members(),
        &expected[..],
        "A's record is now A and B"
    );

    let row = c
        .query_one(
            "SELECT patient_id::text, body->>'subject_a', body->>'subject_b', plaintext_twin \
               FROM event_log WHERE event_id = $1::text::uuid",
            &[&out.event_id.to_string()],
        )
        .await
        .unwrap();
    let (filed, sa, sb, twin): (String, String, String, String) =
        (row.get(0), row.get(1), row.get(2), row.get(3));
    assert_eq!(filed, a.to_string(), "the envelope names the opened chart");
    let (lo, hi) = canonical_pair(b, cc);
    assert_eq!(
        (sa, sb),
        (lo.to_string(), hi.to_string()),
        "the payload names the canonical pair"
    );
    assert!(
        twin.contains(&b.to_string()) && twin.contains(&cc.to_string()),
        "the twin names both subjects"
    );
    assert!(
        !twin.contains(&a.to_string()),
        "the twin says nothing about the chart it is filed under"
    );
    assert_eq!(
        standing(&c, b, cc).await,
        Some(("unlink".into(), true)),
        "the human's unlink stands"
    );
}

#[tokio::test]
async fn an_unlink_on_a_cycle_is_recorded_and_says_still_joined() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b, cc) = chain(&c, &sk_a, &kid_a).await;
    peer_link(&c, &sk_a, &kid_a, a, cc, a, 52).await;
    let who = Reviewer {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };

    let out = unlink_charts(&mut c, b, cc, Some(a), &who, ORIGIN)
        .await
        .expect("recorded, though the cycle keeps the record whole");
    assert_eq!(out.effect, LinkEffect::StillJoined);
    assert!(
        [a, b, cc].iter().all(|x| out.charts.contains(x)),
        "A's record still reads all three"
    );
    assert_eq!(
        standing(&c, b, cc).await,
        Some(("unlink".into(), true)),
        "the judged edge itself is unlinked"
    );
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
    // A held chart whose record lacks the pair is a stale or wrong picture (the INPUT), not a
    // node that has yet to receive something: the same class the in-transaction re-check gives
    // the same fact, so the clinician is told to reload, never to wait (PR #711 review).
    assert_eq!(
        refusal_scope(&err),
        Some(RefusalScope::Input),
        "a verdict about the picture judged from"
    );
    let text = err.to_string();
    assert!(text.contains(&d.to_string()), "names the open chart");
    assert!(
        text.contains("does not read both") && text.contains("reload the chart"),
        "says why, and what to do: {text}"
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
async fn a_receiver_without_the_opened_chart_applies_the_unlink() {
    // The audit's sync pin, in one database: a peer filed an unlink under a chart Z that this
    // node has never seen. The sync door admits it anyway and projects the pair from the
    // PAYLOAD — the envelope's chart is where the event is filed, not what it is about. The
    // event is the one R2b-2 actually makes — a human's ATTESTED unlink — so the pin also shows
    // the receiver keeps its rank (decision 5) without the envelope chart (PR #711 review).
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (_a, b, cc) = chain(&c, &sk_a, &kid_a).await;
    let z = Uuid::now_v7();
    let mut ev = link_assertion_event(&kid_h, b, cc, LinkVerb::Unlink, 60, 0, "peer-z", true);
    ev.patient_id = z.to_string();
    apply_remote_attested(&c, &sk_h, ev, &sk_h, &kid_h)
        .await
        .expect("the sync door admits an attested unlink filed under a chart unseen here");
    assert_eq!(
        standing(&c, b, cc).await,
        Some(("unlink".into(), true)),
        "projected from the payload's pair, still attested"
    );
}

#[tokio::test]
async fn a_reprojection_reproduces_the_third_chart_unlink() {
    // Replay must reach the same record from the stored payloads alone. A full rebuild over
    // 'identity.' is refused by cairn_reproject (patient_chart is also fed by patient.amended
    // and note.added), so the link projection's own tables are emptied by hand and HEAL mode
    // replays into them — for these tables that IS a rebuild, and every row it produces comes
    // from the events' payloads, never the envelope.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b, cc) = chain(&c, &sk_a, &kid_a).await;
    let who = Reviewer {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };
    unlink_charts(&mut c, b, cc, Some(a), &who, ORIGIN)
        .await
        .expect("the far link unlinks");

    c.batch_execute(
        "TRUNCATE patient_link, person_member, link_veto_flag, identity_projection_flag",
    )
    .await
    .unwrap();
    assert_eq!(standing(&c, b, cc).await, None, "the projection is empty");
    c.query_one(
        "SELECT count(*) FROM cairn_reproject('identity.', false, 'test')",
        &[],
    )
    .await
    .unwrap();

    assert_eq!(
        standing(&c, b, cc).await,
        Some(("unlink".into(), true)),
        "the replayed unlink stands, still attested"
    );
    let set: Vec<String> = c
        .query(
            "SELECT x::text FROM cairn_person_charts($1::text::uuid) AS x ORDER BY 1",
            &[&a.to_string()],
        )
        .await
        .unwrap()
        .iter()
        .map(|r| r.get(0))
        .collect();
    let mut expected = vec![a.to_string(), b.to_string()];
    expected.sort();
    assert_eq!(set, expected, "A's record replays as A and B");
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
        (d, RefusalScope::Input, "does not read both"),
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

/// Register `a`, then two more charts in sort order `lo_chart < hi_chart`, and link `a` to the
/// NEAR one and the near one to the FAR one — all three held here, the links a peer machine's.
/// `near_is_low` picks which of the two sorts low, so a test can put the near chart on either
/// side of the canonical pair. Returns `(a, near, far)`.
async fn held_chain(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    near_is_low: bool,
) -> (Uuid, Uuid, Uuid) {
    // `now_v7` is monotonic within a process, so mint order IS sort order.
    let (a, lo_chart, hi_chart) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    let (near, far) = if near_is_low {
        (lo_chart, hi_chart)
    } else {
        (hi_chart, lo_chart)
    };
    common::submit_registration(c, sk, kid, a, 1).await;
    common::submit_registration(c, sk, kid, near, 2).await;
    common::submit_registration(c, sk, kid, far, 3).await;
    peer_link(c, sk, kid, a, near, a, 50).await;
    peer_link(c, sk, kid, near, far, a, 51).await;
    (a, near, far)
}

#[tokio::test]
async fn the_record_shown_is_the_open_charts_even_when_a_subject_carries_the_filing() {
    // All three charts held: the far link near–far is filed under a SUBJECT (low, by C1), yet the
    // record reported back must be A's — the chart on screen — or the window would tell the
    // clinician reading A that A's own members left "this record". Run with the far chart on
    // either side of the pair: when it sorts low it carries the filing, and a report of the
    // FILED-UNDER chart's record ({far}) instead of A's would be caught (PR #711 review).
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    for near_is_low in [true, false] {
        let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
        let (a, near, far) = held_chain(&c, &sk_a, &kid_a, near_is_low).await;
        let who = Reviewer {
            human_sk: &sk_h,
            human_kid: &kid_h,
        };
        let out = unlink_charts(&mut c, near, far, Some(a), &who, ORIGIN)
            .await
            .expect("the far link unlinks when judged from A");
        let (lo, _) = canonical_pair(near, far);
        assert_eq!(out.filed_under, lo, "both held: filed under low (C1)");
        assert_eq!(out.record_of, a, "the record shown is the open chart's");
        assert_eq!(
            out.effect,
            LinkEffect::TookEffect,
            "near_is_low={near_is_low}"
        );
        let mut expected = [a, near];
        expected.sort();
        assert_eq!(
            out.charts.members(),
            &expected[..],
            "A's record is now A and the near chart (near_is_low={near_is_low})"
        );
    }
}

#[tokio::test]
async fn a_split_reads_took_effect_whichever_way_round_the_pair_is_named() {
    // "Still joined?" is asked of the two SUBJECTS. Asked of anything else — the first-named
    // chart, or `high`, in the OPEN chart's record — it answers StillJoined for exactly one
    // orientation of a successful split, because the near chart stays in A's record. So every
    // orientation is run: the near chart sorting low or high, named first or second.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    for near_is_low in [true, false] {
        for near_first in [true, false] {
            let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
            // Only A is held (the #699 (a) picture); B and C arrive through a peer's links.
            let a = Uuid::now_v7();
            let (lo_chart, hi_chart) = (Uuid::now_v7(), Uuid::now_v7());
            let (near, far) = if near_is_low {
                (lo_chart, hi_chart)
            } else {
                (hi_chart, lo_chart)
            };
            common::submit_registration(&c, &sk_a, &kid_a, a, 1).await;
            peer_link(&c, &sk_a, &kid_a, a, near, a, 50).await;
            peer_link(&c, &sk_a, &kid_a, near, far, a, 51).await;
            let who = Reviewer {
                human_sk: &sk_h,
                human_kid: &kid_h,
            };
            let (x, y) = if near_first { (near, far) } else { (far, near) };
            let out = unlink_charts(&mut c, x, y, Some(a), &who, ORIGIN)
                .await
                .expect("the far link unlinks");
            assert_eq!(
                out.effect,
                LinkEffect::TookEffect,
                "near_is_low={near_is_low} near_first={near_first}"
            );
        }
    }
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
        "chart-review unlinked-by:test",
        None,
        &who,
        hlc,
    )
    .await
    .expect_err("D's record does not hold the pair");
    tx.rollback().await.unwrap();
    assert_eq!(refusal_scope(&err), Some(RefusalScope::Input));
    let text = err.to_string();
    assert!(text.contains(&d.to_string()), "names the chart: {text}");
    assert_eq!(
        standing(&c, b, cc).await,
        Some(("link".into(), false)),
        "nothing was written"
    );
    assert_eq!(attested_unlinks(&c).await, 0, "no judgement was signed");
}

#[tokio::test]
async fn a_peer_unlink_landing_mid_judgement_is_seen_before_anything_is_signed() {
    // The third-chart filing rests on A's record holding B and C. A peer's A–B unlink can arrive
    // through the sync door at any moment; a re-read that is merely INSIDE the judgement's
    // READ COMMITTED transaction can still read the record before that peer commits, and then
    // file B–C under A after A's record has stopped holding either (PR #711 review, finding 1).
    // The re-read must therefore come after taking db/018's CARNLK, the one lock every identity
    // apply holds until it commits.
    //
    // Deterministic, not timing-based: T1 (a second connection) holds CARNLK; the judgement is
    // fired and observed PARKED on that advisory lock (`pg_stat_activity`); only then does T1
    // apply the peer's A–B unlink and commit. With the re-read before the lock, the judgement had
    // already passed it and files under A; with the re-read after the lock, it sees the new record
    // and refuses. (T1 applies the unlink only AFTER the judgement parks: the sync door's clock
    // merge row-locks the node clock, which would otherwise block the judgement's own clock tick
    // BEFORE its transaction and hide the race.)
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b, cc) = chain(&c, &sk_a, &kid_a).await;

    let mut t1 = db::connect(&base)
        .await
        .expect("second connection to CAIRN_TEST_PG");
    let t1_tx = t1.transaction().await.unwrap();
    t1_tx
        .execute("SELECT pg_advisory_xact_lock(x'4341524E4C4B'::bigint)", &[])
        .await
        .unwrap();

    let judgement_pid: i32 = c
        .query_one("SELECT pg_backend_pid()", &[])
        .await
        .unwrap()
        .get(0);
    let (sk_h_owned, kid_h_owned) = (sk_h.clone(), kid_h.clone());
    let handle = tokio::spawn(async move {
        let who = Reviewer {
            human_sk: &sk_h_owned,
            human_kid: &kid_h_owned,
        };
        let out = unlink_charts(&mut c, b, cc, Some(a), &who, ORIGIN).await;
        (c, out)
    });

    // Wait (bounded) until the judgement is parked on an ADVISORY lock — CARNLK, which T1 holds.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let row = t1_tx
            .query_one(
                "SELECT wait_event_type, wait_event FROM pg_stat_activity WHERE pid = $1",
                &[&judgement_pid],
            )
            .await
            .unwrap();
        let (kind, event): (Option<String>, Option<String>) = (row.get(0), row.get(1));
        if kind.as_deref() == Some("Lock") && event.as_deref() == Some("advisory") {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the judgement never parked on CARNLK within 5s (last: {kind:?}/{event:?})"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // The peer's unlink of A–B, delivered through the sync door inside T1, then committed.
    let ev = link_assertion_event(&kid_a, a, b, LinkVerb::Unlink, 70, 0, "peer-mid", false);
    let signed = sign(&ev, &sk_a).unwrap();
    t1_tx
        .execute("SELECT apply_remote_event($1)", &[&signed.signed_bytes])
        .await
        .expect("the peer's unlink is admitted");
    t1_tx.commit().await.unwrap();

    let (_c, out) = timeout(Duration::from_secs(5), handle)
        .await
        .expect("the judgement finished within 5s of CARNLK being released")
        .expect("the spawned task did not panic");
    let err = out.expect_err("A's record no longer holds B and C: nothing may be filed under A");
    assert_eq!(refusal_scope(&err), Some(RefusalScope::Input));
    let text = err.to_string();
    assert!(text.contains("reload the chart"), "{text}");
    assert_eq!(
        standing(&t1, b, cc).await,
        Some(("link".into(), false)),
        "B–C still stands: nothing was written"
    );
    assert_eq!(attested_unlinks(&t1).await, 0, "no judgement was signed");
}
