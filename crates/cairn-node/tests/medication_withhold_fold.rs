//! ADR-0081 (#803) — the read-time withhold fold, `cairn_medication_group_holds`.
//!
//! The fold answers "which holds are on these medication lines at instant `now`?" from the
//! two append-only projections (`medication_withhold`, `medication_withhold_release`). It
//! takes `now` as a PARAMETER, so every test here passes a FIXED instant (all in 2030, well
//! clear of any real clock) and every hold sets an explicit `from`: a verdict that depended
//! on when the suite ran would pin nothing. (A release is stamped with its event's own time,
//! i.e. the real now of 2026 — always `<=` the 2030 instants used, so it is "already
//! effective" at every probe unless a test says otherwise.)
//!
//! The rule under test: a hold is RETURNED when `(until_reviewed OR until_at > now)` and no
//! release naming its event id has `released_at <= now`; it is PENDING when `starts_at > now`.
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.
use cairn_event::medication::HoldUntil;
use cairn_event::SigningKey;
use cairn_node::db;
use cairn_node::medication::{
    build_release_body, build_withhold_body, reconcile_medications, release_medication_holds,
    separate_medications, withhold_medication, ReconcileInput, WithholdMedicationInput,
};
use tokio_postgres::Client;
use uuid::Uuid;

mod common;
#[path = "common/medication_kit.rs"]
mod medication_kit;
use medication_kit::*;

/// One returned hold, reduced to what the tests assert on.
#[derive(Debug, PartialEq)]
struct Hold {
    group: String,
    medication: String,
    withhold_event: String,
    reviewed: bool,
    pending: bool,
}

/// The line (group) a thread currently belongs to.
async fn group_of(c: &Client, med: Uuid) -> Uuid {
    let g: String = c
        .query_one(
            "SELECT group_id::text FROM medication_thread_group WHERE medication_id = $1::text::uuid",
            &[&med.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    g.parse().unwrap()
}

/// The fold, called for `groups` at the fixed instant `now` (RFC 3339 with an offset).
async fn holds_at(c: &Client, groups: &[Uuid], now: &str) -> Vec<Hold> {
    let groups: Vec<String> = groups.iter().map(|g| g.to_string()).collect();
    c.query(
        "SELECT group_id::text, medication_id::text, withhold_event_id::text, \
                until_reviewed, pending \
         FROM cairn_medication_group_holds($1::text[]::uuid[], $2::text::timestamptz)",
        &[&groups, &now],
    )
    .await
    .unwrap()
    .iter()
    .map(|r| Hold {
        group: r.get(0),
        medication: r.get(1),
        withhold_event: r.get(2),
        reviewed: r.get(3),
        pending: r.get(4),
    })
    .collect()
}

/// A hold with an explicit `from` and a closed `until` (both fixed instants).
async fn timed(
    c: &mut Client,
    sk: &SigningKey,
    kid: &str,
    p: Uuid,
    med: Uuid,
    from: &str,
    until: &str,
) -> Uuid {
    let input = WithholdMedicationInput {
        reason: "NBM for theatre",
        from: Some(from),
        until: HoldUntil::At(until),
    };
    withhold_medication(c, sk, kid, "test-node", p, med, &input, None)
        .await
        .expect("the timed withhold is admitted")
}

/// An "until reviewed" hold with an explicit `from`.
async fn open_ended(
    c: &mut Client,
    sk: &SigningKey,
    kid: &str,
    p: Uuid,
    med: Uuid,
    from: &str,
) -> Uuid {
    let input = WithholdMedicationInput {
        reason: "BP 85/50",
        from: Some(from),
        until: HoldUntil::Reviewed,
    };
    withhold_medication(c, sk, kid, "test-node", p, med, &input, None)
        .await
        .expect("the open-ended withhold is admitted")
}

async fn release(c: &mut Client, sk: &SigningKey, kid: &str, p: Uuid, med: Uuid, ids: &[Uuid]) {
    release_medication_holds(c, sk, kid, "test-node", p, med, ids, None, None)
        .await
        .expect("the release is admitted");
}

#[tokio::test]
async fn a_hold_is_in_force_inside_its_span_and_gone_after() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let w = timed(
        &mut c,
        &sk,
        &kid,
        p,
        med,
        "2030-01-01T00:00:00Z",
        "2030-01-02T00:00:00Z",
    )
    .await;
    let g = group_of(&c, med).await;

    let inside = holds_at(&c, &[g], "2030-01-01T23:59:00Z").await;
    assert_eq!(
        inside,
        vec![Hold {
            group: g.to_string(),
            medication: med.to_string(),
            withhold_event: w.to_string(),
            reviewed: false,
            pending: false,
        }]
    );
    // `until` is the first instant the hold is OVER (strict `>`): at T it is already gone.
    assert!(holds_at(&c, &[g], "2030-01-02T00:00:00Z").await.is_empty());
    assert!(holds_at(&c, &[g], "2030-01-02T00:01:00Z").await.is_empty());
}

#[tokio::test]
async fn until_in_another_offset_compares_as_an_instant() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    // 2030-01-02T08:00:00+10:00 is 2030-01-01T22:00:00Z.
    timed(
        &mut c,
        &sk,
        &kid,
        p,
        med,
        "2030-01-01T00:00:00Z",
        "2030-01-02T08:00:00+10:00",
    )
    .await;
    let g = group_of(&c, med).await;

    assert_eq!(holds_at(&c, &[g], "2030-01-01T21:59:00Z").await.len(), 1);
    assert!(holds_at(&c, &[g], "2030-01-01T22:00:00Z").await.is_empty());
}

#[tokio::test]
async fn an_advance_hold_is_pending_until_it_starts() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    timed(
        &mut c,
        &sk,
        &kid,
        p,
        med,
        "2030-01-01T10:00:00Z",
        "2030-01-01T20:00:00Z",
    )
    .await;
    let g = group_of(&c, med).await;

    let before = holds_at(&c, &[g], "2030-01-01T09:00:00Z").await;
    assert_eq!(before.len(), 1, "a pending hold is still returned");
    assert!(before[0].pending, "…flagged pending, not in force");
    // `starts_at > now` is strict: at the start instant the hold is in force.
    let at_start = holds_at(&c, &[g], "2030-01-01T10:00:00Z").await;
    assert_eq!(at_start.len(), 1);
    assert!(!at_start[0].pending);
}

#[tokio::test]
async fn a_release_cancels_a_pending_hold() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let w = timed(
        &mut c,
        &sk,
        &kid,
        p,
        med,
        "2030-01-05T10:00:00Z",
        "2030-01-05T20:00:00Z",
    )
    .await;
    let g = group_of(&c, med).await;
    assert_eq!(holds_at(&c, &[g], "2030-01-01T00:00:00Z").await.len(), 1);

    release(&mut c, &sk, &kid, p, med, &[w]).await;

    // Before it ever started, and after it would have: never returned again.
    assert!(holds_at(&c, &[g], "2030-01-01T00:00:00Z").await.is_empty());
    assert!(holds_at(&c, &[g], "2030-01-05T12:00:00Z").await.is_empty());
}

#[tokio::test]
async fn a_release_of_an_ended_hold_is_a_no_op() {
    // A release effective AFTER the hold's `until` (a peer event, so its t_effective can be
    // chosen): admitted, and it changes nothing — the hold still shows inside its span and is
    // gone after `until`, exactly as without the release.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let w = timed(
        &mut c,
        &sk,
        &kid,
        p,
        med,
        "2030-01-01T00:00:00Z",
        "2030-01-02T00:00:00Z",
    )
    .await;
    let g = group_of(&c, med).await;
    let wall = db::next_hlc(&c, "test-node").await.unwrap().wall + 1_000;
    let mut r_body = build_release_body(Uuid::now_v7(), med, p, &[w], None, &kid, peer_hlc(wall));
    r_body.t_effective = Some("2030-01-03T00:00:00Z".into());
    apply_remote(&c, &sk, &r_body)
        .await
        .expect("a late release of an ended hold is admitted");

    assert_eq!(holds_at(&c, &[g], "2030-01-01T12:00:00Z").await.len(), 1);
    assert!(holds_at(&c, &[g], "2030-01-02T00:00:00Z").await.is_empty());
    assert!(holds_at(&c, &[g], "2030-01-04T00:00:00Z").await.is_empty());
}

#[tokio::test]
async fn a_synced_release_on_another_line_ends_nothing() {
    // The local door refuses a release naming a withhold of another line; the sync door
    // admits it (no node-local veto). The fold must still not let it end line a's hold.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, a) = chart_with_one_med(&mut c, &sk, &kid).await;
    let b = add_med(&mut c, &sk, &kid, p).await;
    let w = open_ended(&mut c, &sk, &kid, p, a, "2030-01-01T00:00:00Z").await;
    let wall = db::next_hlc(&c, "test-node").await.unwrap().wall + 1_000;
    let r_body = effective_at_wall(
        &c,
        build_release_body(Uuid::now_v7(), b, p, &[w], None, &kid, peer_hlc(wall)),
    )
    .await;
    apply_remote(&c, &sk, &r_body)
        .await
        .expect("the sync door admits the stray release");

    let rows = holds_at(&c, &[group_of(&c, a).await], "2030-06-01T00:00:00Z").await;
    assert_eq!(rows.len(), 1, "line a's hold stands");
    assert_eq!(rows[0].withhold_event, w.to_string());
}

#[tokio::test]
async fn releasing_one_of_two_holds_leaves_the_other() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let w1 = open_ended(&mut c, &sk, &kid, p, med, "2030-01-01T00:00:00Z").await;
    let w2 = open_ended(&mut c, &sk, &kid, p, med, "2030-01-02T00:00:00Z").await;
    let g = group_of(&c, med).await;
    assert_eq!(holds_at(&c, &[g], "2030-06-01T00:00:00Z").await.len(), 2);

    release(&mut c, &sk, &kid, p, med, &[w1]).await;

    let left = holds_at(&c, &[g], "2030-06-01T00:00:00Z").await;
    assert_eq!(left.len(), 1, "the count is right");
    assert_eq!(
        left[0].withhold_event,
        w2.to_string(),
        "the right one stays"
    );
}

#[tokio::test]
async fn until_reviewed_holds_until_released() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let w = open_ended(&mut c, &sk, &kid, p, med, "2030-01-01T00:00:00Z").await;
    let g = group_of(&c, med).await;

    // No clock ends it: a century on, it still holds.
    let later = holds_at(&c, &[g], "2130-01-01T00:00:00Z").await;
    assert_eq!(later.len(), 1);
    assert!(later[0].reviewed);

    release(&mut c, &sk, &kid, p, med, &[w]).await;
    assert!(holds_at(&c, &[g], "2130-01-01T00:00:00Z").await.is_empty());
}

fn edge() -> ReconcileInput<'static> {
    ReconcileInput {
        provenance: "clinician-judgment",
        reason: None,
    }
}

#[tokio::test]
async fn a_hold_on_any_member_thread_shows_on_the_group() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, a) = chart_with_one_med(&mut c, &sk, &kid).await;
    let b = add_med(&mut c, &sk, &kid, p).await;
    reconcile_medications(&mut c, &sk, &kid, "test-node", p, a, b, &edge(), None, None)
        .await
        .unwrap();
    let g = group_of(&c, a).await;
    assert_eq!(g, group_of(&c, b).await, "reconciled: one line");
    // The group id is one member's id; hold the OTHER member so the join is what is tested.
    let held = if g == a { b } else { a };
    let w = open_ended(&mut c, &sk, &kid, p, held, "2030-01-01T00:00:00Z").await;

    let rows = holds_at(&c, &[g], "2030-06-01T00:00:00Z").await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].group, g.to_string());
    assert_eq!(rows[0].medication, held.to_string());
    assert_eq!(rows[0].withhold_event, w.to_string());
}

#[tokio::test]
async fn a_separation_moves_nothing() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, a) = chart_with_one_med(&mut c, &sk, &kid).await;
    let b = add_med(&mut c, &sk, &kid, p).await;
    reconcile_medications(&mut c, &sk, &kid, "test-node", p, a, b, &edge(), None, None)
        .await
        .unwrap();
    open_ended(&mut c, &sk, &kid, p, b, "2030-01-01T00:00:00Z").await;
    separate_medications(&mut c, &sk, &kid, "test-node", p, a, b, &edge(), None, None)
        .await
        .unwrap();

    // The hold is on b's own thread: it shows under b's line, and a's line stays clear.
    let (ga, gb) = (group_of(&c, a).await, group_of(&c, b).await);
    assert_ne!(ga, gb, "separated: two lines again");
    let now = "2030-06-01T00:00:00Z";
    let rows = holds_at(&c, &[gb], now).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].group, gb.to_string());
    assert_eq!(rows[0].medication, b.to_string());
    assert!(holds_at(&c, &[ga], now).await.is_empty());
}

#[tokio::test]
async fn a_release_synced_before_its_withhold_ends_it_on_arrival() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let g = group_of(&c, med).await;

    // Peer events (the sync door never checks that the named withhold exists): the withhold
    // is minted first, the release second — but they ARRIVE in the opposite order.
    let wall = db::next_hlc(&c, "test-node").await.unwrap().wall + 1_000;
    let wid = Uuid::now_v7();
    let w_in = WithholdMedicationInput {
        reason: "HR 44 on the ward",
        from: Some("2030-01-01T00:00:00Z"),
        until: HoldUntil::Reviewed,
    };
    let w_body = build_withhold_body(wid, med, p, &w_in, &kid, peer_hlc(wall));
    let r_body = effective_at_wall(
        &c,
        build_release_body(
            Uuid::now_v7(),
            med,
            p,
            &[wid],
            None,
            &kid,
            peer_hlc(wall + 10),
        ),
    )
    .await;
    apply_remote(&c, &sk, &r_body)
        .await
        .expect("release applies first");
    apply_remote(&c, &sk, &w_body)
        .await
        .expect("withhold applies second");

    // Before the release's instant the hold is there (pending: it starts in 2030) — proves
    // the withhold really landed, so the emptiness below is the release's doing.
    let early = holds_at(&c, &[g], "2020-01-01T00:00:00Z").await;
    assert_eq!(early.len(), 1);
    assert!(early[0].pending);

    assert!(
        holds_at(&c, &[g], "2030-06-01T00:00:00Z").await.is_empty(),
        "the hold was ended by a release that arrived before it"
    );
}

#[tokio::test]
async fn a_release_takes_effect_at_its_own_instant() {
    // `released_at <= p_now`: a release made AT `now` has already happened. A local release is
    // stamped with the real clock, which no fixed probe can equal, so mint it as a peer event
    // asserted effective at exactly 2030-01-01T00:00:00Z and probe at it. (t_effective, not a
    // 2030 HLC wall: a far-future wall ratchets the shared node clock a day ahead and breaks
    // every later suite that expects "starts_at is about now".)
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let w = open_ended(&mut c, &sk, &kid, p, med, "2029-12-01T00:00:00Z").await;
    let g = group_of(&c, med).await;

    let wall = db::next_hlc(&c, "test-node").await.unwrap().wall + 1_000;
    let mut r_body = build_release_body(Uuid::now_v7(), med, p, &[w], None, &kid, peer_hlc(wall));
    r_body.t_effective = Some("2030-01-01T00:00:00Z".into());
    apply_remote(&c, &sk, &r_body)
        .await
        .expect("release applies");

    assert_eq!(
        holds_at(&c, &[g], "2029-12-31T23:59:59Z").await.len(),
        1,
        "a second before the release the hold still stands"
    );
    assert!(holds_at(&c, &[g], "2030-01-01T00:00:00Z").await.is_empty());
}

#[tokio::test]
async fn holds_are_listed_latest_start_first() {
    // ADR-0081 decision 3: several holds can stand at once, all shown, latest start first —
    // written in the opposite order, so insertion order cannot pass for the rule.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let early = open_ended(&mut c, &sk, &kid, p, med, "2030-01-01T00:00:00Z").await;
    let late = open_ended(&mut c, &sk, &kid, p, med, "2030-01-02T00:00:00Z").await;
    let g = group_of(&c, med).await;

    let order: Vec<String> = holds_at(&c, &[g], "2030-06-01T00:00:00Z")
        .await
        .into_iter()
        .map(|h| h.withhold_event)
        .collect();
    assert_eq!(order, vec![late.to_string(), early.to_string()]);
}

#[tokio::test]
async fn a_peer_hold_already_over_is_admitted_at_sync_but_never_shown() {
    // A node offline for hours syncs a timed W whose `until` has since passed. The "until
    // already past" check is CLINICAL STATE (it reads the event's time against this node's
    // view), so the sync door must admit the validly-signed event — refusing it would fork
    // the event set. And an empty span is never in force, so the fold never shows it, not
    // even as "pending" at an instant before its (later) start.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let g = group_of(&c, med).await;

    let wall = db::next_hlc(&c, "test-node").await.unwrap().wall + 1_000;
    let w_in = WithholdMedicationInput {
        reason: "NBM for theatre",
        from: None,
        until: HoldUntil::At("2001-01-01T00:00:00Z"),
    };
    let body = effective_at_wall(
        &c,
        build_withhold_body(Uuid::now_v7(), med, p, &w_in, &kid, peer_hlc(wall)),
    )
    .await;
    apply_remote(&c, &sk, &body)
        .await
        .expect("the sync door admits a hold whose until has passed");
    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM medication_withhold WHERE medication_id = $1::text::uuid",
            &[&med.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 1, "the row is stored");

    // Before `until` (2000) its start (now) is still ahead, yet it is never "coming".
    assert!(holds_at(&c, &[g], "2000-06-01T00:00:00Z").await.is_empty());
    assert!(holds_at(&c, &[g], "2030-06-01T00:00:00Z").await.is_empty());
}
