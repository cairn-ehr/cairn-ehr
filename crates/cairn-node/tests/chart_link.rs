//! Repair path R2a — `link_charts` / `unlink_charts`: a human's judgement on two charts,
//! authored as an ATTESTED identity event, resolving any open match_proposal for the pair
//! in the same transaction (ADR-0076 decisions 4 and 5; #681's orchestration).
//!
//! How a judgement meets the machine's proposals, vetoes and doubted links is in
//! `chart_link_machine.rs`; what a judgement refuses is in `chart_link_refusals.rs` (#810).
//!
//! Real Postgres, gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.
use cairn_event::{generate_key, SigningKey};
use cairn_node::chart_link::{link_charts, unlink_charts, LinkEffect, LinkVerb, Reviewer};
use cairn_node::db;
use tokio_postgres::Client;
use uuid::Uuid;

mod common;
use common::{apply_remote_raw, link_assertion_event, register_pair};

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

const ORIGIN: &str = "r2a-test-node";

/// Clean identity + proposal state; enroll an agent (to register charts and play the
/// machine) and a human reviewer.
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
        "SELECT enroll_actor('agent', '{\"model\":\"cl-stub\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
        &[&kid_a],
    )
    .await
    .unwrap();
    c.execute(
        "SELECT enroll_actor('human', '{\"role\":\"records-officer\",\"actor\":\"CL\"}', $1)",
        &[&kid_h],
    )
    .await
    .unwrap();
    (sk_a, kid_a, sk_h, kid_h)
}

async fn standing(c: &Client, a: Uuid, b: Uuid) -> Option<(String, bool)> {
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    c.query_opt(
        "SELECT state, attested FROM patient_link WHERE low = $1::text::uuid AND high = $2::text::uuid",
        &[&lo.to_string(), &hi.to_string()],
    )
    .await
    .unwrap()
    .map(|r| (r.get(0), r.get(1)))
}

#[tokio::test]
async fn a_link_is_attested_and_the_two_charts_read_as_one_set() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk_a, &kid_a, a, b).await;

    let who = Reviewer {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };
    let out = link_charts(&mut c, b, a, &who, ORIGIN).await.expect("link");

    assert_eq!(standing(&c, a, b).await, Some(("link".into(), true)));
    assert_eq!(
        out.charts.members().len(),
        2,
        "the returned set is the combined chart"
    );
    assert!(!out.proposal_resolved, "no proposal existed");
    let attester: Option<Vec<u8>> = c
        .query_one(
            "SELECT attester_key FROM event_log WHERE event_id = $1::text::uuid",
            &[&out.event_id.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        attester,
        Some(sk_h.verifying_key().to_bytes().to_vec()),
        "the human vouched"
    );
}

#[tokio::test]
async fn different_people_is_an_unlink_on_a_pair_never_linked() {
    // ADR-0076 decision 4.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk_a, &kid_a, a, b).await;

    let who = Reviewer {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };
    let out = unlink_charts(&mut c, a, b, None, &who, ORIGIN)
        .await
        .expect("unlink");
    assert_eq!(standing(&c, a, b).await, Some(("unlink".into(), true)));
    assert_eq!(out.charts.members(), &[a], "a stays a chart of its own");
    assert_eq!(out.effect, LinkEffect::TookEffect);
}

#[tokio::test]
async fn an_unlink_after_a_link_splits_the_set_again() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk_a, &kid_a, a, b).await;

    let who = Reviewer {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };
    link_charts(&mut c, a, b, &who, ORIGIN).await.unwrap();
    let out = unlink_charts(&mut c, a, b, None, &who, ORIGIN)
        .await
        .unwrap();
    assert_eq!(out.charts.members().len(), 1);
    assert_eq!(
        out.effect,
        LinkEffect::TookEffect,
        "a DIRECT link, unlinked, really splits the pair — nothing else holds them together"
    );
}

#[tokio::test]
async fn an_unlink_through_a_third_chart_is_recorded_and_says_it_did_not_split() {
    // A–C and C–B are linked, so A and B read as one record THROUGH
    // C although no A–B edge was ever asserted. The human says "A and B are different
    // people": the attested unlink on the (never-linked) A–B edge IS recorded — it is the
    // human's judgement and it replicates (ADR-0076 decision 4) — but it cannot split the
    // record, because the A–C–B path still stands. Which of A–C or C–B is wrong is a
    // judgement only a human may make (principle 2), so nothing is auto-resolved: the
    // outcome must SAY that the charts are still joined, never claim "unlinked".
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b, mid) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk_a, &kid_a, a, b).await;
    common::submit_registration(&c, &sk_a, &kid_a, mid, 1).await;
    let who = Reviewer {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };
    link_charts(&mut c, a, mid, &who, ORIGIN).await.unwrap();
    link_charts(&mut c, mid, b, &who, ORIGIN).await.unwrap();

    let out = unlink_charts(&mut c, a, b, None, &who, ORIGIN)
        .await
        .expect("the judgement is recorded even though it cannot split the record");
    assert_eq!(
        out.effect,
        LinkEffect::StillJoined,
        "b still reads as part of a's record through the third chart"
    );
    assert_eq!(out.charts.members().len(), 3, "the record is still one");
    assert_eq!(
        standing(&c, a, b).await,
        Some(("unlink".into(), true)),
        "the human's A–B judgement stands on its own edge"
    );
}

#[tokio::test]
async fn a_displayed_member_not_held_here_can_still_be_unlinked() {
    // R1's combined read shows a member line for every chart in the
    // person component — including one whose REGISTRATION has not reached this node (a
    // peer's link named it; it synced ahead). R2 puts "Not the same person" on that line.
    // Refusing it because the chart is "not held" would leave the clinician looking at a
    // wrong merge they cannot undo. An unlink attaches nothing, so the typo risk that makes
    // LINK demand both charts held does not apply.
    //
    // Run twice: once with the unheld chart sorting HIGH, once LOW. The second order is
    // the one that matters for the write door — the event's envelope must be filed under
    // the chart this node HOLDS, because db/005 step 8b refuses a local event about a chart
    // with no history here (and the peer's link below is filed under the held chart, so the
    // unheld one has none). The second run also names the UNHELD chart first: the outcome
    // must still describe the held chart, never one this node cannot open.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    for unheld_sorts_low in [false, true] {
        let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
        let (x, y) = (Uuid::now_v7(), Uuid::now_v7()); // x < y (v7 is time-ordered)
        let (held, unheld) = if unheld_sorts_low { (y, x) } else { (x, y) };
        common::submit_registration(&c, &sk_a, &kid_a, held, 1).await;
        // A peer's machine link joins them; `unheld` is never registered here.
        let peer = link_assertion_event(
            &kid_a,
            held,
            unheld,
            LinkVerb::Link,
            50,
            0,
            "peer-matcher",
            false,
        );
        apply_remote_raw(&c, &sk_a, peer)
            .await
            .expect("set-union admits a link naming a chart that has not synced yet");

        let who = Reviewer {
            human_sk: &sk_h,
            human_kid: &kid_h,
        };
        let (first, second) = if unheld_sorts_low {
            (unheld, held)
        } else {
            (held, unheld)
        };
        let out = unlink_charts(&mut c, first, second, None, &who, ORIGIN)
            .await
            .unwrap_or_else(|e| {
                panic!("unheld_sorts_low={unheld_sorts_low}: a displayed member unlinks: {e}")
            });
        assert_eq!(
            standing(&c, held, unheld).await,
            Some(("unlink".into(), true)),
            "unheld_sorts_low={unheld_sorts_low}"
        );
        assert_eq!(out.filed_under, held, "unheld_sorts_low={unheld_sorts_low}");
        assert_eq!(
            out.charts.members(),
            &[held],
            "unheld_sorts_low={unheld_sorts_low}: the held chart reads alone again"
        );
        assert_eq!(out.effect, LinkEffect::TookEffect);
    }
}

#[tokio::test]
async fn a_judgement_a_later_one_outranks_is_recorded_but_says_it_did_not_take_effect() {
    // A peer's clinician judged the same pair "same person" with a clock ahead of this
    // node's (db/020 admits a future wall; it only caps how far it moves the local clock).
    // Between two human judgements the later wins, so this node's unlink is recorded — it is
    // real and replicates — but the record still reads as one. It must say so: "unlinked"
    // would be false, and so would "still joined through another link" (there is none).
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk_a, &kid_a, a, b).await;

    let peer = link_assertion_event(
        &kid_h,
        a,
        b,
        LinkVerb::Link,
        now_ms() + THIRTY_DAYS_MS,
        0,
        "peer-ahead",
        true,
    );
    common::apply_remote_attested(&c, &sk_h, peer, &sk_h, &kid_h)
        .await
        .expect("the peer's attested link lands");

    let who = Reviewer {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };
    let out = unlink_charts(&mut c, a, b, None, &who, ORIGIN)
        .await
        .expect("recorded, though outranked");
    assert_eq!(out.effect, LinkEffect::Outranked);
    assert_eq!(
        standing(&c, a, b).await,
        Some(("link".into(), true)),
        "the later human link still stands"
    );
    assert!(out.charts.contains(&b), "the record still reads as one");
    let recorded: i64 = c
        .query_one(
            "SELECT count(*) FROM event_log WHERE event_id = $1::text::uuid",
            &[&out.event_id.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(recorded, 1, "the judgement is recorded all the same");
    reset_clock(&c).await;
}

#[tokio::test]
async fn a_judgement_a_later_agreeing_one_outranks_still_took_effect() {
    // The mirror: a peer's clinician already said "different people", with a clock ahead of
    // this node's. This node's unlink loses the overlay to it — but the record reads exactly
    // as this clinician said, so there is no disagreement to settle. Reporting one would send
    // a clinician looking for a conflict that does not exist.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk_a, &kid_a, a, b).await;
    let peer = link_assertion_event(
        &kid_h,
        a,
        b,
        LinkVerb::Unlink,
        now_ms() + THIRTY_DAYS_MS,
        0,
        "peer-ahead",
        true,
    );
    common::apply_remote_attested(&c, &sk_h, peer, &sk_h, &kid_h)
        .await
        .expect("the peer's attested unlink lands");

    let who = Reviewer {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };
    let out = unlink_charts(&mut c, a, b, None, &who, ORIGIN)
        .await
        .unwrap();
    assert_eq!(out.effect, LinkEffect::TookEffect);
    assert_eq!(out.charts.members(), &[a]);
    reset_clock(&c).await;
}

/// Wall-clock now in ms, the HLC's unit.
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

/// Far enough ahead that db/020's drift clamp (24h) keeps this node's clock BEHIND the
/// peer's event, so a local judgement made now is deterministically the earlier one.
const THIRTY_DAYS_MS: i64 = 30 * 24 * 3_600_000;

/// Put the shared clock back (PR #285's rule, as `status.rs` does): `hlc_state` survives
/// TRUNCATE and the suites share one serialized database, and a far-future remote event
/// leaves it ~24h ahead — a latent trap for whichever test runs next.
async fn reset_clock(c: &Client) {
    c.execute(
        "UPDATE hlc_state SET hlc_wall = 0, hlc_counter = 0 WHERE id",
        &[],
    )
    .await
    .unwrap();
}
