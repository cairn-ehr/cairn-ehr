//! R6 (#716) — "Unlink — not the same person" from the review panel is signed only while the
//! pair's standing assertion is still the one the panel showed.
//!
//! # Why the unlink needs the same check as the confirm
//!
//! The review panel offers two verdicts on a link INSIDE one record. The confirm already carries
//! the pair's standing content address (`confirm_link_as_shown`, tests/confirm_link.rs), because
//! a third chart C can keep A and B joined: a colleague's judgement of A–B landing meanwhile
//! leaves the record's chart SET unchanged, so a set check alone passes. The Unlink is the mirror
//! image. The panel shows A–B "linked without a clinician's confirmation"; a colleague's ATTESTED
//! link of A–B lands while our clinician reads; our clinician clicks Unlink. Our unlink is
//! attested too and stamped later by this node's clock (db/020 merges the peer's HLC), so it wins
//! the attested-vs-attested tie-break and silently overrules the colleague — reported as an
//! ordinary `StillJoined`. `unlink_charts_as_shown_from` closes that: the address the panel read
//! is re-checked under the link lock (CARNLK) before anything is signed.
//!
//! # The harness
//!
//! Deterministic, not timing-based (the `confirm_link.rs` harness, copied here because test
//! helpers are file-local by house convention): T1 holds CARNLK, the unlink is observed PARKED on
//! it, and only then does T1 apply the colleague's event and commit.
//!
//! Real Postgres, gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.
use cairn_event::{event_address, generate_key, sign, sign_attestation, SigningKey};
use cairn_medication_view::ChartSet;
use cairn_node::chart_link::{
    canonical_pair, standing_link, unlink_charts_as_shown_from, LinkEffect, LinkOutcome, LinkVerb,
    Reviewer,
};
use cairn_node::db;
use cairn_node::db_diagnosis::{refusal_scope, RefusalScope};
use std::time::Duration;
use tokio::time::timeout;
use tokio_postgres::Client;
use uuid::Uuid;

mod common;
use common::{cs, link_assertion_event};

const ORIGIN: &str = "r6-unlink";

/// Clean identity + proposal state; enrol an agent (registers charts, plays the peer's machine)
/// and a human reviewer. Copied from `tests/confirm_link.rs` (test helpers are file-local by
/// house convention).
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
        "SELECT enroll_actor('agent', '{\"model\":\"unlink-as-shown-stub\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
        &[&kid_a],
    )
    .await
    .unwrap();
    c.execute(
        "SELECT enroll_actor('human', '{\"role\":\"records-officer\",\"actor\":\"UNLINK-AS-SHOWN\"}', $1)",
        &[&kid_h],
    )
    .await
    .unwrap();
    (sk_a, kid_a, sk_h, kid_h)
}

/// Enrol a second human — the colleague whose judgement may land meanwhile.
async fn colleague(c: &Client) -> (SigningKey, String) {
    let (sk, kid) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('human', '{\"role\":\"records-officer\",\"actor\":\"COLLEAGUE\"}', $1)",
        &[&kid],
    )
    .await
    .unwrap();
    (sk, kid)
}

/// The pair's standing `patient_link` row as `(state, attested)`, or `None`. Either order.
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

/// How many ATTESTED unlinks the log holds — every attester's, a colleague's included (the query
/// does not filter by key). "Nothing signed by us" is therefore "the count did not move".
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

/// The content address of the judgement standing on `x`–`y`, as the review panel reads it.
async fn panel_read(c: &Client, x: Uuid, y: Uuid) -> Vec<u8> {
    let (lo, hi) = canonical_pair(x, y);
    standing_link(c, lo, hi)
        .await
        .unwrap()
        .expect("the panel was opened on a standing link")
        .content_address
}

/// Machine (un-attested) links of each pair, applied through the sync door as a peer's.
async fn machine_links(c: &Client, sk_a: &SigningKey, kid_a: &str, pairs: &[(Uuid, Uuid, i64)]) {
    for &(p, q, wall) in pairs {
        let ev = link_assertion_event(kid_a, p, q, LinkVerb::Link, wall, 0, "peer", false);
        common::apply_remote_raw(c, sk_a, ev).await.unwrap();
    }
}

/// Three charts registered here, joined A–C–B by machine links with A–B among them. Returns
/// `(a, b, c, record)`.
async fn bridged_record(
    c: &Client,
    sk_a: &SigningKey,
    kid_a: &str,
) -> (Uuid, Uuid, Uuid, ChartSet) {
    let (a, b, cc) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    common::submit_registration(c, sk_a, kid_a, a, 1).await;
    common::submit_registration(c, sk_a, kid_a, b, 2).await;
    common::submit_registration(c, sk_a, kid_a, cc, 3).await;
    machine_links(c, sk_a, kid_a, &[(a, cc, 40), (cc, b, 41), (a, b, 42)]).await;
    (a, b, cc, ChartSet::new([a, b, cc]).unwrap())
}

/// A colleague's ATTESTED link of `x`–`y`, signed in advance — a human's judgement on a peer node,
/// which the colleague both signs and vouches for. Returns `(signed, token, attester_key)`.
fn colleague_link(sk: &SigningKey, kid: &str, x: Uuid, y: Uuid, wall: i64) -> [Vec<u8>; 3] {
    let ev = link_assertion_event(kid, x, y, LinkVerb::Link, wall, 0, "colleague", true);
    let signed = sign(&ev, sk).unwrap().signed_bytes;
    let token = sign_attestation(&event_address(&signed), kid, "attested", sk).unwrap();
    let vk = sk.verifying_key().to_bytes().to_vec();
    [signed, token, vk]
}

/// Run `judge` on a connection of its own while T1 holds CARNLK; wait until the judgement is
/// PARKED on that advisory lock; then apply `meanwhile` (a colleague's attested event) inside T1
/// and commit it. Returns what the judgement did once released.
async fn judged_while_the_pair_changes<J>(
    base: &str,
    judge: J,
    meanwhile: [Vec<u8>; 3],
) -> anyhow::Result<LinkOutcome>
where
    J: FnOnce(tokio_postgres::Client) -> tokio::task::JoinHandle<anyhow::Result<LinkOutcome>>,
{
    let mut t1 = db::connect(base).await.unwrap();
    let t1_tx = t1.transaction().await.unwrap();
    t1_tx
        .execute("SELECT pg_advisory_xact_lock(x'4341524E4C4B'::bigint)", &[])
        .await
        .unwrap();

    let jc = db::connect(base).await.unwrap();
    let pid: i32 = jc
        .query_one("SELECT pg_backend_pid()", &[])
        .await
        .unwrap()
        .get(0);
    let handle = judge(jc);

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let row = t1_tx
            .query_one(
                "SELECT wait_event_type, wait_event FROM pg_stat_activity WHERE pid = $1",
                &[&pid],
            )
            .await
            .unwrap();
        let (kind, event): (Option<String>, Option<String>) = (row.get(0), row.get(1));
        if kind.as_deref() == Some("Lock") && event.as_deref() == Some("advisory") {
            break;
        }
        assert!(
            !handle.is_finished() && tokio::time::Instant::now() < deadline,
            "the judgement never parked on CARNLK within 5s (last: {kind:?}/{event:?})"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let [signed, token, vk] = meanwhile;
    t1_tx
        .execute(
            "SELECT apply_remote_event($1, $2, $3)",
            &[&signed, &Some(token), &Some(vk)],
        )
        .await
        .expect("the colleague's event is admitted");
    t1_tx.commit().await.unwrap();
    timeout(Duration::from_secs(5), handle)
        .await
        .expect("the judgement finished within 5s of CARNLK being released")
        .expect("the spawned task did not panic")
}

/// Assert `err` is the pair-changed refusal: a verdict about stale input, worded to review again.
fn assert_review_again(err: &anyhow::Error) {
    assert_eq!(
        refusal_scope(err),
        Some(RefusalScope::Input),
        "stale input, not an outage: {err:#}"
    );
    let text = format!("{err:#}");
    assert!(
        text.contains("review it again"),
        "the refusal must say what to do: {text}"
    );
}

/// THE RACE, deterministic. The unlink parks on CARNLK; meanwhile a colleague's ATTESTED LINK of
/// A–B lands. C still joins A and B, so the record's SET is unchanged and the set check alone
/// would pass. The colleague's wall is LOW (43): without the pair check our unlink, stamped later
/// by this node's clock, would win the attested-vs-attested tie-break and silently overrule the
/// colleague's confirm. With the check it is refused, and the colleague's link stands.
#[tokio::test]
async fn an_unlink_never_overrules_a_colleagues_confirm_that_landed_mid_judgement() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b, _cc, set) = bridged_record(&c, &sk_a, &kid_a).await;
    let shown = panel_read(&c, a, b).await;
    let (sk_col, kid_col) = colleague(&c).await;
    let before = attested_unlinks(&c).await;
    let set_j = set.clone();
    let out = judged_while_the_pair_changes(
        &base,
        move |mut jc| {
            tokio::spawn(async move {
                let who = Reviewer {
                    human_sk: &sk_h,
                    human_kid: &kid_h,
                };
                unlink_charts_as_shown_from(&mut jc, a, b, a, &set_j, &shown, &who, ORIGIN).await
            })
        },
        colleague_link(&sk_col, &kid_col, a, b, 43),
    )
    .await;
    let err = out.expect_err("the colleague's confirm landed after the panel read the pair");
    assert_review_again(&err);
    assert_eq!(
        standing(&c, a, b).await,
        Some(("link".into(), true)),
        "the colleague's confirm stands"
    );
    assert_eq!(attested_unlinks(&c).await, before, "nothing signed by us");
    // Positive control: the set really was unchanged, so only the pair check could refuse.
    let now = cairn_node::patient::person::person_charts(&c, a)
        .await
        .unwrap();
    assert_eq!(
        now, set,
        "C still joins A and B — the set check alone would have passed"
    );
}

/// The same refusal without a race: the colleague's confirm landed before the click.
#[tokio::test]
async fn an_unlink_is_refused_when_the_pair_changed_since_the_panel_read_it() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b, _cc, set) = bridged_record(&c, &sk_a, &kid_a).await;
    let shown = panel_read(&c, a, b).await;
    let (sk_col, kid_col) = colleague(&c).await;
    let [signed, token, vk] = colleague_link(&sk_col, &kid_col, a, b, 43);
    c.execute(
        "SELECT apply_remote_event($1, $2, $3)",
        &[&signed, &Some(token), &Some(vk)],
    )
    .await
    .expect("the colleague's event is admitted");
    let who = Reviewer {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };
    let err = unlink_charts_as_shown_from(&mut c, a, b, a, &set, &shown, &who, ORIGIN)
        .await
        .unwrap_err();
    assert_review_again(&err);
    assert_eq!(standing(&c, a, b).await, Some(("link".into(), true)));
    assert_eq!(attested_unlinks(&c).await, 0, "nothing signed");
}

/// With the address the panel read, the unlink records exactly as before: A–C–B judged from a
/// THIRD chart (C, on screen) — the unlink stands on A–B but C still joins them.
#[tokio::test]
async fn an_unlink_over_the_pair_as_shown_records_and_reports_still_joined() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b, cc, set) = bridged_record(&c, &sk_a, &kid_a).await;
    let shown = panel_read(&c, a, b).await;
    let who = Reviewer {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };
    let out = unlink_charts_as_shown_from(&mut c, a, b, cc, &set, &shown, &who, ORIGIN)
        .await
        .unwrap();
    assert_eq!(out.effect, LinkEffect::StillJoined);
    assert_eq!(out.record_of, cc, "the record judged FROM");
    assert_eq!(out.charts, set, "C still joins A and B");
    assert_eq!(standing(&c, a, b).await, Some(("unlink".into(), true)));
}

/// …and on a two-chart record the unlink splits it, as `unlink_charts` always did.
#[tokio::test]
async fn an_unlink_over_the_pair_as_shown_splits_a_two_chart_record() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    common::submit_registration(&c, &sk_a, &kid_a, a, 1).await;
    common::submit_registration(&c, &sk_a, &kid_a, b, 2).await;
    machine_links(&c, &sk_a, &kid_a, &[(a, b, 40)]).await;
    let set = ChartSet::new([a, b]).unwrap();
    let shown = panel_read(&c, a, b).await;
    let who = Reviewer {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };
    let out = unlink_charts_as_shown_from(&mut c, a, b, a, &set, &shown, &who, ORIGIN)
        .await
        .unwrap();
    assert_eq!(out.effect, LinkEffect::TookEffect);
    assert_eq!(out.charts, ChartSet::single(a));
    assert_eq!(standing(&c, a, b).await, Some(("unlink".into(), true)));
}
