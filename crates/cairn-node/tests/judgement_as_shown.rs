//! #708 — a human's link/unlink judgement is signed only over the records the clinician was SHOWN.
//!
//! # The defect
//!
//! The window's "Same person" checks both compared chart sets against fresh reads and then calls
//! `link_charts`, which received neither set. A peer link landing between that check and
//! `submit_event` was signed over: the clinician compared {A} with {B}, and the event joined
//! {A, C} with {B}. "Different people" had the mirror image (the #708 comment): a colleague's
//! attested LINK of the pair arriving by sync mid-judgement, overruled by our unlink's higher HLC,
//! reported as a plain `TookEffect`.
//!
//! The fix: the judgement carries the two records it was made over (`RecordsShown`), and
//! `assert_link_in_tx` re-reads each subject's record under db/018's link lock (CARNLK) — which
//! every link/unlink apply holds until it commits — before anything is signed. A record that no
//! longer reads as shown refuses the judgement as a stale picture (`RefusalScope::Stale`).
//!
//! The race tests are deterministic, not timing-based (the `unlink_from_record.rs` harness): T1
//! holds CARNLK, the judgement is observed PARKED on it, and only then does T1 apply the peer's
//! link and commit.
//!
//! Real Postgres, gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.
use cairn_event::{event_address, generate_key, sign, sign_attestation, SigningKey};
use cairn_medication_view::ChartSet;
use cairn_node::chart_link::{
    canonical_pair, link_charts_as_shown, unlink_charts_as_shown, LinkEffect, LinkOutcome,
    LinkVerb, RecordsShown, Reviewer,
};
use cairn_node::db;
use cairn_node::db_diagnosis::{refusal_scope, RefusalScope};
use std::time::Duration;
use tokio::time::timeout;
use tokio_postgres::Client;
use uuid::Uuid;

mod common;
use common::link_assertion_event;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

const ORIGIN: &str = "judgement-as-shown";

/// Clean identity + proposal state; enrol an agent (registers charts, plays the peer's machine)
/// and a human reviewer. Copied from `tests/unlink_from_record.rs` (test helpers are file-local
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

/// How many ATTESTED judgements of `event_type` the log holds — every attester's, a peer's
/// included (the query does not filter by key).
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

/// A peer machine's UN-attested link of `x`–`y`.
fn peer_link(sk_a: &SigningKey, kid_a: &str, x: Uuid, y: Uuid, wall: i64) -> PeerEvent {
    let ev = link_assertion_event(kid_a, x, y, LinkVerb::Link, wall, 0, "peer", false);
    PeerEvent {
        signed: sign(&ev, sk_a).unwrap().signed_bytes,
        attestation: None,
    }
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

/// Assert `err` is the as-shown refusal: a verdict about a stale picture, worded to compare again.
fn assert_compare_again(err: &anyhow::Error) {
    assert_eq!(
        refusal_scope(err),
        Some(RefusalScope::Stale),
        "a changed record is stale input, not an outage: {err:#}"
    );
    let text = format!("{err:#}");
    assert!(
        text.ends_with("nothing was done"),
        "the refusal must say what to do: {text}"
    );
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

/// RACE 1 — "Same person" over {A} and {B}; a peer links A–X while the judgement waits for the
/// link lock. Signing would join {A, X} with {B}, a record nobody compared.
#[tokio::test]
async fn a_link_is_refused_when_a_peer_link_changes_a_compared_record_mid_judgement() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b, x) = three_charts(&c, &sk_a, &kid_a).await;

    let (sk_j, kid_j) = (sk_h.clone(), kid_h.clone());
    let out = judged_while_the_record_changes(
        &base,
        move |mut jc| {
            tokio::spawn(async move {
                let who = Reviewer {
                    human_sk: &sk_j,
                    human_kid: &kid_j,
                };
                let (left, right) = (ChartSet::single(a), ChartSet::single(b));
                let shown = RecordsShown {
                    this: &left,
                    other: &right,
                    pair: None,
                };
                link_charts_as_shown(&mut jc, a, b, shown, &who, ORIGIN).await
            })
        },
        peer_link(&sk_a, &kid_a, a, x, 70),
    )
    .await;

    let err = out.expect_err(
        "A's record became {A, X} after the comparison: the link must not be signed over it",
    );
    assert_compare_again(&err);
    assert_eq!(
        standing(&c, a, b).await,
        None,
        "nothing about A–B was written"
    );
    assert_eq!(judgements_signed(&c, "identity.link.asserted").await, 0);
}

/// RACE 2 — "Different people" over {A} and {B}; a colleague's ATTESTED link of A–B lands while
/// the judgement waits. Our unlink would be stamped above it and silently overrule a colleague's
/// judgement the clinician never saw (the #708 comment).
#[tokio::test]
async fn an_unlink_never_silently_overrules_a_colleagues_link_that_landed_mid_judgement() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b, _) = three_charts(&c, &sk_a, &kid_a).await;
    let (sk_colleague, kid_colleague) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('human', '{\"role\":\"records-officer\",\"actor\":\"COLLEAGUE\"}', $1)",
        &[&kid_colleague],
    )
    .await
    .unwrap();

    let (sk_j, kid_j) = (sk_h.clone(), kid_h.clone());
    let out = judged_while_the_record_changes(
        &base,
        move |mut jc| {
            tokio::spawn(async move {
                let who = Reviewer {
                    human_sk: &sk_j,
                    human_kid: &kid_j,
                };
                let (left, right) = (ChartSet::single(a), ChartSet::single(b));
                let shown = RecordsShown {
                    this: &left,
                    other: &right,
                    pair: None,
                };
                unlink_charts_as_shown(&mut jc, a, b, shown, &who, ORIGIN).await
            })
        },
        colleague_link(&sk_colleague, &kid_colleague, a, b, 70),
    )
    .await;

    let err = out.expect_err(
        "A and B became one record after the comparison: the unlink must not overrule that",
    );
    assert_compare_again(&err);
    assert_eq!(
        standing(&c, a, b).await,
        Some(("link".into(), true)),
        "the colleague's attested link still stands"
    );
    assert_eq!(judgements_signed(&c, "identity.unlink.asserted").await, 0);
}

/// The check reads the record, not the timing: a shown record that is ALREADY stale when the
/// judgement starts is refused the same way, for both verbs — and a shown pair that matches the
/// record goes through (positive control: a check refusing everything would pass the races).
#[tokio::test]
async fn a_stale_shown_record_is_refused_and_a_current_one_is_judged() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b, x) = three_charts(&c, &sk_a, &kid_a).await;
    let who = Reviewer {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };
    // A and X are joined by a peer BEFORE the judgement; the caller still shows A alone.
    let ev = link_assertion_event(&kid_a, a, x, LinkVerb::Link, 70, 0, "peer", false);
    common::apply_remote_raw(&c, &sk_a, ev).await.unwrap();
    let (stale, right) = (ChartSet::single(a), ChartSet::single(b));
    let shown = RecordsShown {
        this: &stale,
        other: &right,
        pair: None,
    };
    for verb in [LinkVerb::Link, LinkVerb::Unlink] {
        let err = match verb {
            LinkVerb::Link => link_charts_as_shown(&mut c, a, b, shown, &who, ORIGIN).await,
            LinkVerb::Unlink => unlink_charts_as_shown(&mut c, a, b, shown, &who, ORIGIN).await,
        }
        .expect_err("a record shown as {A} that now reads {A, X} is stale");
        assert_compare_again(&err);
    }
    assert_eq!(
        standing(&c, a, b).await,
        None,
        "nothing about A–B was written"
    );

    // Positive control: the record as it now reads.
    let current = ChartSet::new([a, x]).unwrap();
    let shown = RecordsShown {
        this: &current,
        other: &right,
        pair: None,
    };
    let out = link_charts_as_shown(&mut c, a, b, shown, &who, ORIGIN)
        .await
        .expect("a judgement over the current records is signed");
    assert_eq!(out.effect, LinkEffect::TookEffect);
    assert_eq!(standing(&c, a, b).await, Some(("link".into(), true)));
}
