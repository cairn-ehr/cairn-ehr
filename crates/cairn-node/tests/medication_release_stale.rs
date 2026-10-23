//! #843 — a release naming a hold that is no longer standing (a colleague already released it,
//! or it ran out) is refused at the LOCAL door as a STALE picture: the clinician acted on a
//! screen that showed a hold the record has since ended, so re-reading is the whole remedy.
//! Before #843 the floor admitted it, and the record carried a release that released nothing.
//!
//! Two layers, pinned separately:
//!   * the orchestrator (`release_medication_holds`) refuses BEFORE signing, marked
//!     `RefusalScope::Stale`, so the window re-reads by itself (#840's "stale wins");
//!   * the floor (db/059 section 8) refuses the same release from any client at the local door
//!     (a raw `submit_event`), so the rule cannot be bypassed — and ADMITS it at sync apply,
//!     where a node-local veto would fork the event set.
//!
//! A hold not yet started (pending) is still standing: releasing it cancels it.
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`. Key material is
//! derived at runtime.
use cairn_event::medication::HoldUntil;
use cairn_node::db;
use cairn_node::db_diagnosis::{refusal_scope, RefusalScope};
use cairn_node::medication::{
    build_release_body, build_withhold_body, release_medication_holds, withhold_medication,
    WithholdMedicationInput,
};
use tokio_postgres::Client;
use uuid::Uuid;

mod common;
#[path = "common/medication_kit.rs"]
mod medication_kit;
use medication_kit::*;

async fn release_rows(c: &Client, med: Uuid) -> i64 {
    c.query_one(
        "SELECT count(*) FROM medication_withhold_release WHERE medication_id = $1::text::uuid",
        &[&med.to_string()],
    )
    .await
    .unwrap()
    .get(0)
}

/// An instant `secs` from the database's clock, as RFC 3339 UTC text.
async fn instant_from_now(c: &Client, secs: i64) -> String {
    c.query_one(
        "SELECT to_char((clock_timestamp() + make_interval(secs => $1::bigint)) \
                AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')",
        &[&secs],
    )
    .await
    .unwrap()
    .get(0)
}

#[tokio::test]
async fn a_second_release_of_a_released_hold_is_refused_as_stale() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (patient, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let wid = withhold(&mut c, &sk, &kid, patient, med, "BP 85/50").await;
    release_medication_holds(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        med,
        &[wid],
        None,
        None,
    )
    .await
    .expect("the first release ends the hold");

    // The colleague's screen still shows the W; their release must not write a second one.
    let err = release_medication_holds(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        med,
        &[wid],
        Some("BP recovered"),
        None,
    )
    .await
    .expect_err("a release of a hold already released is refused");
    assert_eq!(refusal_scope(&err), Some(RefusalScope::Stale), "{err:#}");
    let text = format!("{err:#}");
    assert!(
        text.contains(&wid.to_string()) && text.contains("already released"),
        "{text}"
    );
    assert!(text.contains("nothing was released"), "{text}");
    assert_eq!(
        release_rows(&c, med).await,
        1,
        "no second release was written"
    );
}

#[tokio::test]
async fn a_release_of_a_hold_that_ran_out_is_refused_as_stale() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (patient, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    // A hold already over cannot be written at the local door, so it arrives from a peer
    // (admitted at sync, never shown) — the state a stale screen would be releasing.
    let from = instant_from_now(&c, -7200).await;
    let until = instant_from_now(&c, -3600).await;
    let w = WithholdMedicationInput {
        reason: "NBM for theatre",
        from: Some(&from),
        until: HoldUntil::At(&until),
    };
    let local = db::next_hlc(&c, "test-node").await.unwrap();
    let wid = Uuid::now_v7();
    let body = build_withhold_body(wid, med, patient, &w, &kid, peer_hlc(local.wall + 1_000));
    apply_remote(&c, &sk, &body)
        .await
        .expect("sync admits a peer's hold that is already over");

    let err = release_medication_holds(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        med,
        &[wid],
        None,
        None,
    )
    .await
    .expect_err("a release of a hold that already ran out is refused");
    assert_eq!(refusal_scope(&err), Some(RefusalScope::Stale), "{err:#}");
    let text = format!("{err:#}");
    assert!(text.contains("already ended"), "{text}");
    assert_eq!(release_rows(&c, med).await, 0, "nothing was written");
}

#[tokio::test]
async fn a_release_of_a_pending_hold_is_still_admitted() {
    // An advance hold ("NBM from 06:00 tomorrow") has not ended: releasing it cancels it.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (patient, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let from = instant_from_now(&c, 86_400).await;
    let w = WithholdMedicationInput {
        reason: "NBM for theatre",
        from: Some(&from),
        until: HoldUntil::Reviewed,
    };
    let wid = withhold_medication(&mut c, &sk, &kid, "test-node", patient, med, &w, None)
        .await
        .unwrap();
    release_medication_holds(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        med,
        &[wid],
        None,
        None,
    )
    .await
    .expect("a pending hold is standing, so its release is admitted");
    assert_eq!(release_rows(&c, med).await, 1);
}

#[tokio::test]
async fn the_floor_refuses_a_dead_release_at_the_local_door_and_admits_it_at_sync() {
    // The orchestrator's check is the legible one; the floor's is the unbypassable one. A raw
    // `submit_event` (any client, the race the orchestrator cannot close) is refused, and the
    // same release arriving from a peer is admitted: the fold makes it a no-op.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (patient, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let wid = withhold(&mut c, &sk, &kid, patient, med, "HR 44").await;
    release_medication_holds(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        med,
        &[wid],
        None,
        None,
    )
    .await
    .unwrap();

    // Stamped at its HLC wall, which is never before the first release's physical instant.
    let hlc = db::next_hlc(&c, "test-node").await.unwrap();
    let local = effective_at_wall(
        &c,
        build_release_body(Uuid::now_v7(), med, patient, &[wid], None, &kid, hlc),
    )
    .await;
    let err = db_msg(&seal_and_submit(&c, &sk, local).await.unwrap_err());
    assert!(
        err.starts_with("medication release:") && err.contains("already released"),
        "{err}"
    );
    assert_eq!(release_rows(&c, med).await, 1);

    let peer = db::next_hlc(&c, "test-node").await.unwrap();
    let remote = effective_at_wall(
        &c,
        build_release_body(
            Uuid::now_v7(),
            med,
            patient,
            &[wid],
            None,
            &kid,
            peer_hlc(peer.wall + 1_000),
        ),
    )
    .await;
    apply_remote(&c, &sk, &remote)
        .await
        .expect("sync apply admits a peer's release of an ended hold");
    assert_eq!(release_rows(&c, med).await, 2);
}
