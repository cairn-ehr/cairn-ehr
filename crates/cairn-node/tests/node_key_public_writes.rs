//! #909 task 1 — a human-authored write needs only the node's PUBLIC id; a node-signed one
//! refuses cleanly without the secret.
//!
//! The window will stop holding `node.key`. Every write it makes is signed by the human, so the
//! node contributes only its kid as the `recorded` contributor. These tests pin both halves:
//! the authored path works with `NodeKey::Public`, and the one path that needs the node's own
//! signature (`author = None`) refuses with `NODE_KEY_NOT_HELD` and writes nothing.
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`. Keys are runtime-generated.

use cairn_node::db;
use cairn_node::medication::sealed_submit::seal_sign_submit;
use cairn_node::medication::{cease_medication, AuthorParams, CeaseMedicationInput};
use cairn_node::node_key::{NodeKey, NODE_KEY_NOT_HELD};
use uuid::Uuid;

mod common;
#[path = "common/medication_kit.rs"]
mod medication_kit;
use medication_kit::*;

#[tokio::test]
async fn an_authored_cessation_signs_with_only_the_public_node_id() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (human_sk, human_kid) = common::enroll_human_with_role(&c, "prescriber").await;
    let author = AuthorParams {
        human_sk: &human_sk,
        human_kid: &human_kid,
    };
    let (patient, med) = chart_with_one_med(&mut c, &sk, &kid).await;

    let input = CeaseMedicationInput {
        stopped: None,
        stopped_precision: None,
        reason: None,
    };
    // Only the public id: the secret `sk` is deliberately not passed.
    let id = cease_medication(
        &mut c,
        NodeKey::Public(&kid),
        &kid,
        "test-node",
        patient,
        med,
        &input,
        Some(&author),
        None,
    )
    .await
    .expect("a human-authored cessation needs only the node's public id");

    let row = c
        .query_one(
            "SELECT signer_key_id, contributors::text FROM event_log WHERE event_id = $1::text::uuid",
            &[&id.to_string()],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, String>("signer_key_id"), human_kid);
    let contributors: serde_json::Value =
        serde_json::from_str(&row.get::<_, String>("contributors")).unwrap();
    assert_eq!(contributors[0]["actor_id"], human_kid);
    assert_eq!(contributors[0]["role"], "authored");
    assert_eq!(contributors[1]["actor_id"], kid);
    assert_eq!(contributors[1]["role"], "recorded");
}

#[tokio::test]
async fn a_node_signed_body_refuses_with_only_the_public_id() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (patient, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let before: i64 = c
        .query_one("SELECT count(*) FROM event_log", &[])
        .await
        .unwrap()
        .get(0);

    let hlc = db::next_hlc(&c, "test-node").await.unwrap();
    let input = CeaseMedicationInput {
        stopped: None,
        stopped_precision: None,
        reason: None,
    };
    let body =
        cairn_node::medication::build_cease_body(Uuid::now_v7(), med, patient, &input, &kid, hlc);
    let err = seal_sign_submit(&mut c, NodeKey::Public(&kid), body, None, None)
        .await
        .expect_err("a node-signed body cannot be signed without the node's secret");
    assert_eq!(err.to_string(), NODE_KEY_NOT_HELD);

    let after: i64 = c
        .query_one("SELECT count(*) FROM event_log", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(before, after, "a refused write leaves no row");
}
