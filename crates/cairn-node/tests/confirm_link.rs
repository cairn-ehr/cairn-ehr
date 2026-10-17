//! R6 (#716) — "Same person — confirm" a link INSIDE one record is signed only while the pair's
//! standing assertion is still the one the panel showed.
//!
//! # What a confirm is
//!
//! A record can be held together by a link nobody vouched for (a machine's, or a peer's matcher),
//! or — the bridge — by other links after a clinician recorded two of its charts as different
//! people. R6's confirm lets a clinician, from inside that one record, put an ATTESTED link on the
//! pair: an ordinary `identity.link.asserted` through `chart_link`'s judgement path
//! (`confirm_link_as_shown`), which lifts the record's sign-off hold once no pair is doubted.
//!
//! # The defect it must not have
//!
//! #708's `RecordsShown` re-reads each subject's RECORD under the link lock before signing. A
//! confirm is made inside one record, and a third chart C can still join A and B: a colleague's
//! attested UNLINK of A–B landing in the last milliseconds leaves the record's set unchanged, so
//! the set check alone passes — and our confirm, stamped later by this node's clock, wins the
//! attested-vs-attested HLC tie-break and silently overrules the colleague. So the confirm also
//! carries the pair's standing content address as the panel read it (`RecordsShown::pair`), and
//! the same CARNLK-held re-check refuses it as a stale picture (`RefusalScope::Stale`) if the pair's
//! winner has changed.
//!
//! # The harness
//!
//! The race test is deterministic, not timing-based (the `judgement_as_shown.rs` harness, copied
//! here because test helpers are file-local by house convention): T1 holds CARNLK, the confirm is
//! observed PARKED on it, and only then does T1 apply the colleague's event and commit.
//!
//! Real Postgres, gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.
use cairn_event::{event_address, generate_key, sign, sign_attestation, SigningKey};
use cairn_medication_view::ChartSet;
use cairn_node::chart_link::{
    canonical_pair, confirm_link_as_shown, standing_link, unlink_charts, LinkEffect, LinkOutcome,
    LinkVerb, Reviewer,
};
use cairn_node::db;
use cairn_node::db_diagnosis::{refusal_scope, RefusalScope};
use std::time::Duration;
use tokio::time::timeout;
use tokio_postgres::Client;
use uuid::Uuid;

mod common;
use common::{cs, link_assertion_event};

const ORIGIN: &str = "r6-confirm";

/// Clean identity + proposal state; enrol an agent (registers charts, plays the peer's machine)
/// and a human reviewer. Copied from `tests/judgement_as_shown.rs` (test helpers are file-local
/// by house convention).
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
        "SELECT enroll_actor('agent', '{\"model\":\"as-shown-stub\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
        &[&kid_a],
    )
    .await
    .unwrap();
    c.execute(
        "SELECT enroll_actor('human', '{\"role\":\"records-officer\",\"actor\":\"AS-SHOWN\"}', $1)",
        &[&kid_h],
    )
    .await
    .unwrap();
    (sk_a, kid_a, sk_h, kid_h)
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

/// How many ATTESTED judgements of `event_type` the log holds — every attester's, a colleague's
/// included (the query does not filter by key), so "nothing signed by us" is "the count did not
/// move".
async fn judgements_signed(c: &Client, event_type: &str) -> i64 {
    c.query_one(
        "SELECT count(*) FROM event_log WHERE event_type = $1 AND attestation IS NOT NULL",
        &[&event_type],
    )
    .await
    .unwrap()
    .get(0)
}

/// Three charts registered here, each its own record.
async fn three_charts(c: &Client, sk: &SigningKey, kid: &str) -> (Uuid, Uuid, Uuid) {
    let (a, b, x) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    common::submit_registration(c, sk, kid, a, 1).await;
    common::submit_registration(c, sk, kid, b, 2).await;
    common::submit_registration(c, sk, kid, x, 3).await;
    (a, b, x)
}

/// A peer's event, signed in advance, for T1 to apply through the sync door mid-judgement.
/// `attestation` carries `(token, attester_key)` for an attested judgement.
struct PeerEvent {
    signed: Vec<u8>,
    attestation: Option<(Vec<u8>, Vec<u8>)>,
}

/// A colleague's ATTESTED link of `x`–`y` — a human's judgement on a peer node, which the
/// colleague both signs and vouches for (the shape `chart_link.rs`'s peer judgements use).
fn colleague_link(sk_h: &SigningKey, kid_h: &str, x: Uuid, y: Uuid, wall: i64) -> PeerEvent {
    let ev = link_assertion_event(kid_h, x, y, LinkVerb::Link, wall, 0, "colleague", true);
    let signed = sign(&ev, sk_h).unwrap().signed_bytes;
    let token = sign_attestation(&event_address(&signed), kid_h, "attested", sk_h).unwrap();
    let vk_h = sk_h.verifying_key().to_bytes().to_vec();
    PeerEvent {
        signed,
        attestation: Some((token, vk_h)),
    }
}

/// Run `judge` on a connection of its own while T1 holds CARNLK; wait until the judgement is
/// PARKED on that advisory lock; then apply `meanwhile` inside T1 and commit it. Returns what the
/// judgement did once released.
async fn judged_while_the_record_changes<J>(
    base: &str,
    judge: J,
    meanwhile: PeerEvent,
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

    let (token, vk) = meanwhile.attestation.unzip();
    t1_tx
        .execute(
            "SELECT apply_remote_event($1, $2, $3)",
            &[&meanwhile.signed, &token, &vk],
        )
        .await
        .expect("the peer's event is admitted");
    t1_tx.commit().await.unwrap();
    timeout(Duration::from_secs(5), handle)
        .await
        .expect("the judgement finished within 5s of CARNLK being released")
        .expect("the spawned task did not panic")
}

/// A colleague's ATTESTED unlink of `x`–`y` (the mirror of `colleague_link`).
fn colleague_unlink(sk_h: &SigningKey, kid_h: &str, x: Uuid, y: Uuid, wall: i64) -> PeerEvent {
    let ev = link_assertion_event(kid_h, x, y, LinkVerb::Unlink, wall, 0, "colleague", true);
    let signed = sign(&ev, sk_h).unwrap().signed_bytes;
    let token = sign_attestation(&event_address(&signed), kid_h, "attested", sk_h).unwrap();
    let vk_h = sk_h.verifying_key().to_bytes().to_vec();
    PeerEvent {
        signed,
        attestation: Some((token, vk_h)),
    }
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

/// A record A–C–B joined by three UN-attested (machine) links, A–B among them. Returns the
/// charts, the record, and A–B's standing content address as a panel would read it.
async fn bridged_record(
    c: &Client,
    sk_a: &SigningKey,
    kid_a: &str,
) -> (Uuid, Uuid, Uuid, ChartSet, Vec<u8>) {
    let (a, b, cc) = three_charts(c, sk_a, kid_a).await;
    for (p, q, wall) in [(a, cc, 40), (cc, b, 41), (a, b, 42)] {
        let ev = link_assertion_event(kid_a, p, q, LinkVerb::Link, wall, 0, "peer", false);
        common::apply_remote_raw(c, sk_a, ev).await.unwrap();
    }
    let (lo, hi) = canonical_pair(a, b);
    let ca = standing_link(c, lo, hi)
        .await
        .unwrap()
        .unwrap()
        .content_address;
    (a, b, cc, ChartSet::new([a, b, cc]).unwrap(), ca)
}

/// Apply a peer's event through the sync door and commit it (outside any race).
async fn apply(c: &Client, ev: PeerEvent) {
    let (token, vk) = ev.attestation.unzip();
    c.execute(
        "SELECT apply_remote_event($1, $2, $3)",
        &[&ev.signed, &token, &vk],
    )
    .await
    .expect("the peer's event is admitted");
}

/// Assert `err` is the pair-changed refusal: a verdict about a stale picture (`Stale`), whose
/// sentence says only that nothing was done — the re-read is the scope's remedy, not the text's.
fn assert_review_again(err: &anyhow::Error) {
    assert_eq!(
        refusal_scope(err),
        Some(RefusalScope::Stale),
        "a stale picture, not an outage: {err:#}"
    );
    let text = format!("{err:#}");
    assert!(
        text.ends_with("nothing was done"),
        "the refusal must say only that nothing was done: {text}"
    );
}

#[tokio::test]
async fn a_confirm_records_an_attested_link_over_the_machines_and_took_effect() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b, _cc, set, ca) = bridged_record(&c, &sk_a, &kid_a).await;
    let who = Reviewer {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };
    let out = confirm_link_as_shown(&mut c, a, b, &set, &ca, &who, ORIGIN)
        .await
        .unwrap();
    assert_eq!(out.effect, LinkEffect::TookEffect);
    assert_eq!(standing(&c, a, b).await, Some(("link".into(), true)));
    assert_eq!(out.charts, set, "a confirm changes no membership");
}

/// The panel read the machine link; a colleague confirmed it since. The pair's winner changed, so
/// ours is refused rather than recorded over a picture that no longer holds.
#[tokio::test]
async fn a_confirm_is_refused_when_the_pair_changed_since_the_panel_read_it() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b, _cc, set, ca) = bridged_record(&c, &sk_a, &kid_a).await;
    let (sk_col, kid_col) = colleague(&c).await;
    apply(&c, colleague_link(&sk_col, &kid_col, a, b, 43)).await;
    // `judgements_signed` counts EVERY attested link in the log, and the colleague's is one: so
    // "nothing signed by us" is "the count did not move across our confirm", not "zero".
    let before = judgements_signed(&c, "identity.link.asserted").await;
    let who = Reviewer {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };
    let err = confirm_link_as_shown(&mut c, a, b, &set, &ca, &who, ORIGIN)
        .await
        .unwrap_err();
    assert_review_again(&err);
    assert_eq!(
        judgements_signed(&c, "identity.link.asserted").await,
        before,
        "nothing signed by us"
    );
}

/// THE RACE, deterministic. The confirm parks on CARNLK; meanwhile a colleague's ATTESTED
/// UNLINK of A–B lands. C still joins A and B, so the record's SET is unchanged and the set check
/// alone would pass. The colleague's wall is LOW (43): without the pair check our confirm, stamped
/// later by this node's clock, would win the attested-vs-attested HLC tie-break and silently
/// overrule it. With the check it is refused.
#[tokio::test]
async fn a_confirm_never_overrules_a_colleagues_unlink_that_landed_mid_judgement() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b, _cc, set, ca) = bridged_record(&c, &sk_a, &kid_a).await;
    let (sk_col, kid_col) = colleague(&c).await;
    let (sk_j, kid_j) = (sk_h.clone(), kid_h.clone());
    let set_j = set.clone();
    let out = judged_while_the_record_changes(
        &base,
        move |mut jc| {
            tokio::spawn(async move {
                let who = Reviewer {
                    human_sk: &sk_j,
                    human_kid: &kid_j,
                };
                confirm_link_as_shown(&mut jc, a, b, &set_j, &ca, &who, ORIGIN).await
            })
        },
        colleague_unlink(&sk_col, &kid_col, a, b, 43),
    )
    .await;
    let err = out.expect_err("the colleague's unlink landed after the panel read the pair");
    assert_review_again(&err);
    assert_eq!(
        standing(&c, a, b).await,
        Some(("unlink".into(), true)),
        "the colleague's stands"
    );
    // Positive control: the set really was unchanged, so only the pair check could refuse.
    let now = cairn_node::patient::person::person_charts(&c, a)
        .await
        .unwrap();
    assert_eq!(
        now, set,
        "C still joins A and B — the set check alone would have passed"
    );
}

/// The bridge re-link (#716's comment, item 2): A–C–X joined, a colleague attested A and X are
/// different people; this clinician links them. Attested-first-then-HLC: ours is newer, it stands.
#[tokio::test]
async fn a_bridge_relink_overrules_the_earlier_attested_unlink_and_lifts_the_hold() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, x, cc) = three_charts(&c, &sk_a, &kid_a).await;
    for (p, q, wall) in [(a, cc, 40), (cc, x, 41)] {
        let ev = link_assertion_event(&kid_a, p, q, LinkVerb::Link, wall, 0, "peer", false);
        common::apply_remote_raw(&c, &sk_a, ev).await.unwrap();
    }
    let (sk_col, kid_col) = colleague(&c).await;
    let col = Reviewer {
        human_sk: &sk_col,
        human_kid: &kid_col,
    };
    unlink_charts(&mut c, a, x, None, &col, ORIGIN)
        .await
        .unwrap();
    let set = ChartSet::new([a, cc, x]).unwrap();
    let ids: Vec<String> = set.members().iter().map(Uuid::to_string).collect();
    let held: bool = c
        .query_one(
            "SELECT cairn_chart_set_has_doubted_link($1::text[]::uuid[])",
            &[&ids],
        )
        .await
        .unwrap()
        .get(0);
    assert!(held, "precondition: the bridge is doubted");
    let (lo, hi) = canonical_pair(a, x);
    let ca = standing_link(&c, lo, hi)
        .await
        .unwrap()
        .unwrap()
        .content_address;
    let who = Reviewer {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };
    let out = confirm_link_as_shown(&mut c, a, x, &set, &ca, &who, ORIGIN)
        .await
        .unwrap();
    assert_eq!(out.effect, LinkEffect::TookEffect);
    assert_eq!(standing(&c, a, x).await, Some(("link".into(), true)));
    let held: bool = c
        .query_one(
            "SELECT cairn_chart_set_has_doubted_link($1::text[]::uuid[])",
            &[&ids],
        )
        .await
        .unwrap()
        .get(0);
    assert!(!held, "the hold lifts once no pair is doubted");
}

/// R1b's hold lifts for a vetoed machine link confirmed by a human.
#[tokio::test]
async fn a_confirm_lifts_the_sign_off_hold_on_a_vetoed_machine_link() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b) = common::vetoed_pair(&c, &sk_a, &kid_a).await;
    let ev = link_assertion_event(&kid_a, a, b, LinkVerb::Link, 50, 0, "peer-matcher", false);
    common::apply_remote_raw(&c, &sk_a, ev).await.unwrap();
    let set = ChartSet::new([a, b]).unwrap();
    let (lo, hi) = canonical_pair(a, b);
    let ca = standing_link(&c, lo, hi)
        .await
        .unwrap()
        .unwrap()
        .content_address;
    let who = Reviewer {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };
    let out = confirm_link_as_shown(&mut c, a, b, &set, &ca, &who, ORIGIN)
        .await
        .unwrap();
    assert_eq!(out.effect, LinkEffect::TookEffect);
    let ids = vec![a.to_string(), b.to_string()];
    let held: bool = c
        .query_one(
            "SELECT cairn_chart_set_has_doubted_link($1::text[]::uuid[])",
            &[&ids],
        )
        .await
        .unwrap()
        .get(0);
    assert!(!held);
}

/// An EMPTY standing address means the panel never read which judgement stands — not that the
/// pair changed. It is refused before anything else, as a fault in what the window sent (`Input`,
/// never `Stale`: nothing about the record moved), with the reason that is
/// true (PR #777 review): before this guard it fell through to the pair check, which no real
/// address ever matches, and was worded "changed while you were judging it".
#[tokio::test]
async fn a_confirm_without_the_address_the_panel_read_is_refused_for_that_reason() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b, _cc, set, _ca) = bridged_record(&c, &sk_a, &kid_a).await;
    let before = judgements_signed(&c, "identity.link.asserted").await;
    let who = Reviewer {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };
    let err = confirm_link_as_shown(&mut c, a, b, &set, &[], &who, ORIGIN)
        .await
        .unwrap_err();
    assert_eq!(refusal_scope(&err), Some(RefusalScope::Input), "{err:#}");
    let text = format!("{err:#}");
    assert!(
        text.contains("did not read which judgement stands"),
        "{text}"
    );
    assert!(!text.contains("changed while"), "{text}");
    assert_eq!(
        judgements_signed(&c, "identity.link.asserted").await,
        before,
        "nothing signed"
    );
}
