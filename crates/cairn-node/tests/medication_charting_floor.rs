//! Charting slice 1a, Task 1 — the floor for the five charting keys on
//! `clinical.medication.asserted` (`cairn_check_medication_charting`, db/077), at both doors.
//!
//! The split is db/041's: a key of the wrong TYPE is refused at both doors (the projection
//! could not read it), while a word outside the vocabulary, a wrong count, or an empty string
//! is refused at the authoring door only — a peer may speak a newer word, and refusing a
//! verifiable event wedges sync (ADR-0056).
//!
//! The local door is the floor called directly with `cairn.remote_apply` unset (as
//! allergy_overlay_floor.rs does: the authoring pipeline seals and needs a human author, which
//! this floor does not care about). The remote door is driven for real through
//! `apply_remote_event` with a peer-signed body.
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.
use cairn_event::{generate_key, sign, Hlc, SigningKey};
use cairn_node::db;
use cairn_node::medication::{build_assert_body, AssertMedicationInput};
use serde_json::{json, Value};
use tokio_postgres::Client;
use uuid::Uuid;

mod common;

/// A realistic HLC wall (ms since epoch, ≈ 2026-06-21), as medication_remote_apply.rs.
const WALL_2026: i64 = 1_782_000_000_000;

/// The postgres RAISE text of a failed call.
fn db_msg(e: &tokio_postgres::Error) -> String {
    e.as_db_error()
        .map(|d| d.message().to_string())
        .unwrap_or_else(|| e.to_string())
}

/// Truncate the log, reset the HLC, and enroll one device signer (the peer's key).
/// Copied from medication_remote_apply.rs, minus the human attester this file never uses.
async fn setup(c: &Client) -> (SigningKey, String) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, medication_statement, \
         medication_cessation, medication_dose_event, medication_dose_correction, \
         medication_reconciliation, medication_group_member, medication_projection_flag, \
         medication_attestation, medication_patient_conflict_flag CASCADE",
    )
    .await
    .unwrap();
    c.batch_execute("UPDATE hlc_state SET hlc_wall = 0, hlc_counter = 0")
        .await
        .unwrap();
    let (sk, kid) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('device', '{\"role\":\"peer-ward-terminal\"}', $1)",
        &[&kid],
    )
    .await
    .unwrap();
    (sk, kid)
}

fn input() -> AssertMedicationInput<'static> {
    AssertMedicationInput {
        term: "Augmentin",
        coding: None,
        formulation: Some("tablet"),
        dose_amount: None,
        dose_unit: None,
        sig: Some("1 bd"),
        info_source: "clinician-observed",
        started: None,
        started_precision: None,
    }
}

/// Merge `top` into a payload's top level and `sub` into its `substance` object.
fn with(mut p: Value, top: Value, sub: Value) -> Value {
    let o = p.as_object_mut().unwrap();
    o.extend(top.as_object().unwrap().clone());
    o["substance"]
        .as_object_mut()
        .unwrap()
        .extend(sub.as_object().unwrap().clone());
    p
}

/// A plain assertion payload, as `build_assert_body` writes it today.
fn base_payload() -> Value {
    build_assert_body(
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        &input(),
        "kid",
        Hlc {
            wall: WALL_2026,
            counter: 0,
            node_origin: "peer".into(),
        },
        None,
    )
    .payload
}

/// Call the assertion floor directly on `payload` at one door (`remote` = the sync-apply
/// door, as db/020 marks it) in a throwaway transaction; `Err` carries the refusal text.
async fn floor(c: &mut Client, payload: &Value, remote: bool) -> Result<(), String> {
    let body = json!({ "payload": payload });
    let tx = c.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('cairn.remote_apply', $1, true)",
        &[&if remote { "on" } else { "" }],
    )
    .await
    .unwrap();
    let r = tx
        .execute(
            "SELECT cairn_check_medication_assertion('clinical.medication.asserted', $1::text::jsonb)",
            &[&body.to_string()],
        )
        .await;
    tx.rollback().await.unwrap();
    r.map(|_| ()).map_err(|e| db_msg(&e))
}

/// Sign `payload` as a peer's assertion and apply it through the real remote door.
async fn apply_peer(c: &Client, sk: &SigningKey, kid: &str, payload: Value) -> Result<u64, String> {
    let mut body = build_assert_body(
        Uuid::now_v7(),
        payload["medication_id"].as_str().unwrap().parse().unwrap(),
        Uuid::now_v7(),
        &input(),
        kid,
        Hlc {
            wall: WALL_2026,
            counter: 0,
            node_origin: "peer".into(),
        },
        None,
    );
    body.payload = payload;
    let signed = sign(&body, sk).unwrap().signed_bytes;
    c.execute("SELECT apply_remote_event($1)", &[&signed])
        .await
        .map_err(|e| db_msg(&e))
}

const AMOX: &str = "0f8c4b1e-1b7a-5c2d-9a3e-2b6f7c8d9e01";

fn coded(term: &str) -> Value {
    json!({"term": term, "coding": {"system": "drugref-moiety", "code": AMOX, "display": term}})
}

#[tokio::test]
async fn local_door_refuses_each_bad_shape() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let allergy = Uuid::now_v7().to_string();

    // A full, well-formed charting passes the local door first, so each refusal below is
    // the one bad key and nothing else.
    let good = with(
        base_payload(),
        json!({"charted_as": "started-here", "route": "po",
               "allergy_override": {"reason": "no alternative", "against": [allergy]}}),
        json!({"strength": "875/125 mg",
               "components": [coded("amoxicillin"), {"term": "clav", "coding": null}]}),
    );
    floor(&mut c, &good, false)
        .await
        .expect("a well-formed charting passes the local door");

    let bad = [
        (json!({"charted_as": "stat"}), json!({}), "charted_as"),
        (json!({"route": ""}), json!({}), "route"),
        (
            json!({}),
            json!({"components": [coded("amoxicillin")]}),
            "substance.components",
        ),
        (
            json!({}),
            json!({"components": [coded("amoxicillin"), coded("clavulanic acid")],
                   "coding": {"system": "drugref-moiety", "code": AMOX, "display": "amoxicillin"}}),
            "substance.components",
        ),
        (
            json!({"allergy_override": {"reason": "  ", "against": [allergy]}}),
            json!({}),
            "allergy_override.reason",
        ),
        (
            json!({"allergy_override": {"reason": "no alternative", "against": []}}),
            json!({}),
            "allergy_override.against",
        ),
    ];
    for (top, sub, field) in bad {
        let p = with(base_payload(), top.clone(), sub.clone());
        let e = floor(&mut c, &p, false)
            .await
            .expect_err(&format!("the local door refuses {top} {sub}"));
        assert!(e.contains(field), "the refusal names {field:?}, got: {e}");
    }
}

#[tokio::test]
async fn remote_door_admits_a_newer_word() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;

    // A newer charted_as word and a one-element components list: refused locally, admitted
    // from a peer, through the real remote door.
    let newer = with(
        base_payload(),
        json!({"charted_as": "transferred-in"}),
        json!({"components": [coded("amoxicillin")]}),
    );
    assert!(floor(&mut c, &newer, false).await.is_err());
    apply_peer(&c, &sk, &kid, newer)
        .await
        .expect("the remote door admits a peer's newer word");

    // The type tier still holds at the remote door: a key the projection could not read.
    for (top, sub, field) in [
        (json!({"charted_as": 7}), json!({}), "charted_as"),
        (
            json!({}),
            json!({"components": "amox"}),
            "substance.components",
        ),
        (
            json!({"allergy_override": {"reason": "x", "against": ["E1"]}}),
            json!({}),
            "allergy_override.against",
        ),
    ] {
        let p = with(base_payload(), top, sub);
        let e = floor(&mut c, &p, true)
            .await
            .expect_err("a wrong type is refused at the remote door too");
        assert!(e.contains(field), "the refusal names {field:?}, got: {e}");
    }
}

#[tokio::test]
async fn an_event_without_charting_still_passes() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;

    let plain = base_payload();
    floor(&mut c, &plain, false)
        .await
        .expect("no charting passes the local door");
    apply_peer(&c, &sk, &kid, plain)
        .await
        .expect("no charting passes the remote door");
}
