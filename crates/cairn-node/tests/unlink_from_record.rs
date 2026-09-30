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
use cairn_event::{generate_key, SigningKey};
use cairn_node::chart_link::{canonical_pair, unlink_charts, LinkEffect, LinkVerb, Reviewer};
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
async fn a_link_on_a_cycle_is_recorded_and_says_still_joined() {
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
    assert_eq!(
        refusal_scope(&err),
        Some(RefusalScope::NodeState),
        "a verdict about this node's state"
    );
    assert!(
        err.to_string().contains(&d.to_string()),
        "names the open chart"
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
    // PAYLOAD — the envelope's chart is where the event is filed, not what it is about.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, _sk_h, _kid_h) = setup(&c).await;
    let (_a, b, cc) = chain(&c, &sk_a, &kid_a).await;
    let z = Uuid::now_v7();
    let mut ev = link_assertion_event(&kid_a, b, cc, LinkVerb::Unlink, 60, 0, "peer-z", false);
    ev.patient_id = z.to_string();
    apply_remote_raw(&c, &sk_a, ev)
        .await
        .expect("the sync door admits an unlink filed under a chart unseen here");
    assert_eq!(
        standing(&c, b, cc).await.map(|(state, _)| state),
        Some("unlink".into()),
        "projected from the payload's pair"
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
