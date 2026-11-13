//! Batch charting slice 2a, Task 3 (split from `medication_set_aside_floor.rs` for its size cap,
//! same rig and fixtures, `common/set_aside_kit.rs`): the set-aside version from a PEER and over
//! time — the remote door admits an orphan and flags another chart's companion, refuses a safety
//! claim; a shred removes the row, a reprojection rebuilds it; and a companion submitted earlier
//! in the same transaction satisfies the authoring floor.
//!
//! DB-gated on `$CAIRN_TEST_PG` (a skipped run prints `skipped:`), serialized via
//! `db::test_serial_guard`.
use cairn_event::sign;
use cairn_node::db;
use cairn_node::medication::build_assert_body;
use cairn_node::shred::shred_event;
use serde_json::json;
use uuid::Uuid;

mod common;
#[path = "common/medication_kit.rs"]
mod medication_kit;
use medication_kit::*;
#[macro_use]
#[path = "common/set_aside_kit.rs"]
mod set_aside_kit;
use set_aside_kit::*;

#[tokio::test]
async fn the_remote_door_admits_an_orphan_and_no_reader_shows_it() {
    rig!(c, sk, kid);
    let p = Uuid::now_v7();
    let orphan_of = Uuid::now_v7();
    let wall = db::next_hlc(&c, "test-node").await.unwrap().wall + 1_000;
    let mut body = version_body(&c, &kid, p, Uuid::now_v7(), orphan_of, |_| {}).await;
    body.hlc = peer_hlc(wall);
    apply_remote(&c, &sk, &body)
        .await
        .expect("the sync door admits a version whose companion has not arrived");
    assert_eq!(
        count(
            &c,
            "SELECT count(*) FROM medication_set_aside WHERE alternative_to = $1::text::uuid",
            orphan_of
        )
        .await,
        1,
        "held"
    );
    // Readers join on alternative_to: no thread exists, so nothing shows it.
    assert_eq!(
        count(
            &c,
            "SELECT count(*) FROM medication_set_aside s \
               JOIN medication_statement m ON m.medication_id = s.alternative_to \
              WHERE s.alternative_to = $1::text::uuid",
            orphan_of
        )
        .await,
        0
    );
    assert_eq!(
        count(
            &c,
            "SELECT count(*) FROM patient_medication WHERE patient_id = $1::text::uuid",
            p
        )
        .await,
        0
    );
}

#[tokio::test]
async fn a_remote_companion_on_another_chart_is_flagged_not_refused() {
    rig!(c, sk, kid);
    let (_p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let elsewhere = Uuid::now_v7();
    let wall = db::next_hlc(&c, "test-node").await.unwrap().wall + 1_000;
    let mut body = version_body(&c, &kid, elsewhere, Uuid::now_v7(), med, |_| {}).await;
    body.hlc = peer_hlc(wall);
    apply_remote(&c, &sk, &body).await.expect("admitted");
    assert_eq!(
        count(
            &c,
            "SELECT count(*) FROM medication_patient_conflict_flag WHERE medication_id = $1::text::uuid",
            med
        )
        .await,
        1
    );
}

#[tokio::test]
async fn a_shred_removes_the_row() {
    rig!(c, sk, kid);
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let ev = add_version(&c, &sk, &kid, p, med).await.unwrap();
    let n = "SELECT count(*) FROM medication_set_aside WHERE event_id = $1::text::uuid";
    assert_eq!(count(&c, n, ev).await, 1);
    shred_event(
        &mut c,
        &sk,
        &kid,
        "test-node",
        ev,
        "retention ceiling",
        None,
    )
    .await
    .expect("shred");
    assert_eq!(
        count(&c, n, ev).await,
        0,
        "the reported drug is clinical text: the shred takes it"
    );
}

#[tokio::test]
async fn a_reprojection_rebuilds_it() {
    rig!(c, sk, kid);
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let ev = add_version(&c, &sk, &kid, p, med).await.unwrap();
    c.batch_execute("TRUNCATE medication_set_aside")
        .await
        .unwrap();
    let n = "SELECT count(*) FROM medication_set_aside WHERE event_id = $1::text::uuid";
    assert_eq!(count(&c, n, ev).await, 0);
    c.execute(
        "SELECT count(*) FROM cairn_reproject('clinical.medication-set-aside', true, 'test')",
        &[],
    )
    .await
    .unwrap();
    assert_eq!(count(&c, n, ev).await, 1);
}

#[tokio::test]
async fn a_companion_in_the_same_transaction_satisfies_the_floor() {
    rig!(c, sk, kid);
    let p = Uuid::now_v7();
    common::submit_registration(&c, &sk, &kid, p, 0).await;
    let med = Uuid::now_v7();
    let companion = build_assert_body(
        Uuid::now_v7(),
        med,
        p,
        &sample_input(),
        &kid,
        db::next_hlc(&c, "test-node").await.unwrap(),
        None,
    );
    let version = version_body(&c, &kid, p, Uuid::now_v7(), med, |_| {}).await;
    let tx = c.transaction().await.unwrap();
    for mut body in [companion, version] {
        // Seal as seal_and_submit does, but submit on the transaction.
        let twin = body.plaintext_twin.take().unwrap();
        let (container, dek) =
            cairn_event::seal::seal_event_payload(&body.payload, &twin, &body.event_id).unwrap();
        body.payload = container;
        body.plaintext_twin = Some(cairn_event::seal::seal_stub_twin(&body.event_type));
        let signed = sign(&body, &sk).unwrap();
        tx.execute(
            "SELECT submit_event($1, NULL, NULL, $2)",
            &[&signed.signed_bytes, &dek.as_bytes().as_slice()],
        )
        .await
        .unwrap_or_else(|e| panic!("{}: {}", body.event_type, db_msg(&e)));
    }
    tx.commit().await.unwrap();
    assert_eq!(
        count(
            &c,
            "SELECT count(*) FROM medication_set_aside WHERE alternative_to = $1::text::uuid",
            med
        )
        .await,
        1
    );
}

#[tokio::test]
async fn a_peers_set_aside_with_a_safety_claim_is_not_admitted_and_raises_no_signal() {
    rig!(c, sk, kid);
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let wall = db::next_hlc(&c, "test-node").await.unwrap().wall + 1_000;
    let mut body = version_body(&c, &kid, p, Uuid::now_v7(), med, |_| {}).await;
    body.hlc = peer_hlc(wall);
    body.safety = Some(json!({"rung": "existence"}));
    // The remote door refuses it (the floor's both-doors safety rule): the only pin of that
    // rule through a real peer apply, so the refusal itself is asserted, not discarded.
    let refused = apply_remote(&c, &sk, &body)
        .await
        .map_err(|e| db_msg(&e))
        .expect_err("a peer's set-aside with a safety claim is refused at the remote door");
    assert!(refused.contains("safety"), "{refused}");
    let held: i64 = c
        .query_one(
            "SELECT count(*) FROM event_log WHERE event_id = $1::text::uuid",
            &[&body.event_id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(held, 0, "nothing of it is in the log");
    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM cairn_patient_safety($1::text::uuid)",
            &[&p.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 0, "no safety signal from a set-aside version");
}
