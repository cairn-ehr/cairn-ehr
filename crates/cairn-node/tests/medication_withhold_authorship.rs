//! #837 item 1 — the withhold and its release on ADR-0053's human-authorship path, and across
//! authors.
//!
//! Every other withhold test passes `author = None` with one key, so two things were unpinned:
//!   * the human overlay for both verbs: the human signs and is the `authored` contributor, the
//!     node is `recorded` and keeps custody of the DEK;
//!   * a W written by someone OTHER than the prescriber. ADR-0081 promises "anyone may
//!     withhold" (the bedside nurse holding a doctor's antihypertensive for a low BP), and the
//!     doctor then releases the nurse's hold. Nothing at the door looks at WHO prescribed today
//!     (db/059 registers both types `targets_other_author FALSE`, so ADR-0043's owner-gate never
//!     runs); this is the only test with two humans, so it is what goes red if a later check
//!     starts refusing a W, or its release, written by someone other than the prescriber.
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`. Key material is
//! generated at runtime.

use cairn_node::db;
use cairn_node::medication::{
    assert_medication, release_medication_holds, withhold_medication, AuthorParams,
};
use tokio_postgres::Client;
use uuid::Uuid;

mod common;
#[path = "common/medication_kit.rs"]
mod medication_kit;
use medication_kit::*;

/// Assert that event `event_id` was signed by `human_kid`, names the human `authored` and the
/// node `recorded`, and that the NODE holds its DEK (ADR-0053: authorship never moves custody).
async fn assert_human_authored(c: &Client, event_id: Uuid, human_kid: &str, node_kid: &str) {
    let row = c
        .query_one(
            "SELECT signer_key_id, contributors::text, \
                    (SELECT count(*) FROM event_dek d WHERE d.event_id = e.event_id) AS custody \
               FROM event_log e WHERE e.event_id = $1::text::uuid",
            &[&event_id.to_string()],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, String>("signer_key_id"), human_kid);
    let contributors: serde_json::Value =
        serde_json::from_str(&row.get::<_, String>("contributors")).unwrap();
    assert_eq!(contributors[0]["actor_id"], human_kid);
    assert_eq!(contributors[0]["role"], "authored");
    assert_eq!(contributors[1]["actor_id"], node_kid);
    assert_eq!(contributors[1]["role"], "recorded");
    assert_eq!(
        row.get::<_, i64>("custody"),
        1,
        "the node keeps the DEK though a human signed"
    );
}

#[tokio::test]
async fn a_nurse_withholds_a_doctors_prescription_and_the_doctor_releases_it() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (doctor_sk, doctor_kid) = common::enroll_human_with_role(&c, "prescriber").await;
    let (nurse_sk, nurse_kid) = common::enroll_human_with_role(&c, "ward-nurse").await;
    let doctor = AuthorParams {
        human_sk: &doctor_sk,
        human_kid: &doctor_kid,
    };
    let nurse = AuthorParams {
        human_sk: &nurse_sk,
        human_kid: &nurse_kid,
    };

    // The doctor prescribes.
    let patient = Uuid::now_v7();
    common::submit_registration(&c, &sk, &kid, patient, 0).await;
    let med = assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        &sample_input(),
        Some(&doctor),
        None,
    )
    .await
    .expect("the doctor's prescription is admitted");

    // The nurse holds it at the bedside: another author's prescription.
    let wid = withhold_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        med,
        &until_reviewed("BP 85/50"),
        Some(&nurse),
    )
    .await
    .expect("ANYONE MAY WITHHOLD (ADR-0081): a nurse's W on a doctor's prescription is admitted");
    assert_human_authored(&c, wid, &nurse_kid, &kid).await;

    // The doctor reviews and releases the nurse's hold.
    let rid = release_medication_holds(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        med,
        &[wid],
        Some("BP recovered"),
        Some(&doctor),
    )
    .await
    .expect("the doctor's release of the nurse's hold is admitted");
    assert_human_authored(&c, rid, &doctor_kid, &kid).await;

    let ended: Option<String> = c
        .query_one(
            "SELECT cairn_medication_hold_ended($1::text::uuid, clock_timestamp())",
            &[&wid.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        ended.is_some_and(|why| why.starts_with("was already released")),
        "the release ended the hold"
    );
}
