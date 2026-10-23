//! #835 — a release recorded for a LATER instant is said on the hold it will end.
//!
//! The read-time fold (`cairn_medication_group_holds`, db/059) drops a hold once a release
//! naming it has `released_at <= now`. A release whose instant is still ahead (a peer whose
//! clock runs fast, or a deliberate "resume from 08:00") left the hold showing "W until
//! reviewed" with no hint that a release exists: an unknown shown as nothing, which principle
//! 4 forbids. db/059's `cairn_medication_hold_release_pending` answers "when does a recorded
//! release end this hold?" and the list read carries it on `MedicationHold::release_pending`.
//!
//! The predicate takes the instant as a parameter, so its tests probe FIXED instants in 2030
//! (as `medication_withhold_fold.rs` does). The one read-path test runs against the real
//! clock, so its hold starts in the past and its release lies far in the future (2099).
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.
use cairn_event::medication::HoldUntil;
use cairn_event::SigningKey;
use cairn_node::db;
use cairn_node::medication::read::list_patient_medications;
use cairn_node::medication::{build_release_body, withhold_medication, WithholdMedicationInput};
use tokio_postgres::Client;
use uuid::Uuid;

mod common;
#[path = "common/medication_kit.rs"]
mod medication_kit;
use medication_kit::*;

/// An "until reviewed" hold starting at the fixed instant `from`.
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

/// A peer's release of `holds`, filed on thread `thread`, effective at the instant `at`
/// (through the sync door: the local orchestrator always stamps the real now).
async fn peer_release_at(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    p: Uuid,
    thread: Uuid,
    holds: &[Uuid],
    at: &str,
) {
    let wall = db::next_hlc(c, "test-node").await.unwrap().wall + 1_000;
    let mut body = build_release_body(Uuid::now_v7(), thread, p, holds, None, kid, peer_hlc(wall));
    body.t_effective = Some(at.into());
    apply_remote(c, sk, &body)
        .await
        .expect("the sync door admits the release");
}

/// The predicate under test at the fixed instant `now`, as RFC 3339 UTC text (NULL → None).
async fn pending_at(c: &Client, withhold: Uuid, now: &str) -> Option<String> {
    c.query_one(
        "SELECT to_char(cairn_medication_hold_release_pending($1::text::uuid, \
                $2::text::timestamptz) AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')",
        &[&withhold.to_string(), &now],
    )
    .await
    .unwrap()
    .get(0)
}

#[tokio::test]
async fn a_release_still_ahead_is_said_until_it_takes_effect() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let w = open_ended(&mut c, &sk, &kid, p, med, "2029-12-01T00:00:00Z").await;
    assert_eq!(
        pending_at(&c, w, "2029-12-31T00:00:00Z").await,
        None,
        "no release recorded: nothing pending"
    );

    peer_release_at(&c, &sk, &kid, p, med, &[w], "2030-01-01T00:00:00Z").await;
    assert_eq!(
        pending_at(&c, w, "2029-12-31T23:59:59Z").await.as_deref(),
        Some("2030-01-01T00:00:00Z"),
        "a second before it takes effect, the release is said"
    );
    // At its own instant it has happened (the fold's `<=`): the hold is gone, nothing pends.
    assert_eq!(pending_at(&c, w, "2030-01-01T00:00:00Z").await, None);
}

#[tokio::test]
async fn the_earliest_of_two_pending_releases_is_the_one_said() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let w = open_ended(&mut c, &sk, &kid, p, med, "2029-12-01T00:00:00Z").await;
    // Written latest-first, so arrival order cannot pass for "earliest".
    peer_release_at(&c, &sk, &kid, p, med, &[w], "2030-03-01T00:00:00Z").await;
    peer_release_at(&c, &sk, &kid, p, med, &[w], "2030-02-01T00:00:00Z").await;
    assert_eq!(
        pending_at(&c, w, "2030-01-01T00:00:00Z").await.as_deref(),
        Some("2030-02-01T00:00:00Z")
    );
}

#[tokio::test]
async fn a_release_of_another_hold_or_on_another_thread_is_not_this_holds() {
    // Mirrors `cairn_medication_hold_ended`: only a release on the hold's OWN thread that
    // names it can end it, so only such a release is said to be coming.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, a) = chart_with_one_med(&mut c, &sk, &kid).await;
    let b = add_med(&mut c, &sk, &kid, p).await;
    let w = open_ended(&mut c, &sk, &kid, p, a, "2029-12-01T00:00:00Z").await;
    let other = open_ended(&mut c, &sk, &kid, p, a, "2029-12-02T00:00:00Z").await;
    peer_release_at(&c, &sk, &kid, p, a, &[other], "2030-02-01T00:00:00Z").await;
    peer_release_at(&c, &sk, &kid, p, b, &[w], "2030-02-01T00:00:00Z").await;
    assert_eq!(pending_at(&c, w, "2030-01-01T00:00:00Z").await, None);
    assert_eq!(
        pending_at(&c, other, "2030-01-01T00:00:00Z")
            .await
            .as_deref(),
        Some("2030-02-01T00:00:00Z"),
        "the other hold's own release is said on it"
    );
}

#[tokio::test]
async fn a_release_after_the_hold_has_run_out_is_not_said() {
    // A hold until 10 Jan and a fast peer's release for 20 Jan: the hold is over on 10 Jan by
    // its own `until`, so the release will end nothing, and "release at 20 Jan" would be a
    // false sentence (PR review). A release before the `until` is still said. The sync door
    // admits both: only the local door refuses a release of an ended hold (ADR-0082).
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let input = WithholdMedicationInput {
        reason: "NBM for theatre",
        from: Some("2029-12-01T00:00:00Z"),
        until: HoldUntil::At("2030-01-10T00:00:00Z"),
    };
    let w = withhold_medication(&mut c, &sk, &kid, "test-node", p, med, &input, None)
        .await
        .expect("the timed withhold is admitted");

    peer_release_at(&c, &sk, &kid, p, med, &[w], "2030-01-20T00:00:00Z").await;
    assert_eq!(pending_at(&c, w, "2030-01-05T00:00:00Z").await, None);
    // Exactly at `until` the hold is already over (the fold's strict `>`): nothing to end.
    peer_release_at(&c, &sk, &kid, p, med, &[w], "2030-01-10T00:00:00Z").await;
    assert_eq!(pending_at(&c, w, "2030-01-05T00:00:00Z").await, None);

    peer_release_at(&c, &sk, &kid, p, med, &[w], "2030-01-08T00:00:00Z").await;
    assert_eq!(
        pending_at(&c, w, "2030-01-05T00:00:00Z").await.as_deref(),
        Some("2030-01-08T00:00:00Z")
    );
}

#[tokio::test]
async fn the_list_carries_a_pending_release_on_its_hold() {
    // The read path (`read_group_holds`) at the REAL now: a hold in force since 2026-01-01, a
    // release recorded for 2099. Without #835 the row said only "withheld until reviewed".
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let w = open_ended(&mut c, &sk, &kid, p, med, "2026-01-01T00:00:00Z").await;
    let kept = open_ended(&mut c, &sk, &kid, p, med, "2026-01-02T00:00:00Z").await;
    peer_release_at(&c, &sk, &kid, p, med, &[w], "2099-01-01T00:00:00Z").await;

    let list = list_patient_medications(&c, p).await.unwrap();
    let holds = &list.rows[0].holds;
    let pending_of = |id: Uuid| {
        holds
            .iter()
            .find(|h| h.withhold_event_id == id)
            .expect("both holds are still in force")
            .release_pending
            .clone()
    };
    assert_eq!(pending_of(w).as_deref(), Some("2099-01-01T00:00:00Z"));
    assert_eq!(pending_of(kept), None, "an unreleased hold says nothing");
}
