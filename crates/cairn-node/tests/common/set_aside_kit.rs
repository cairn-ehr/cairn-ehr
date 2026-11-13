//! The set-aside suites' fixtures (batch charting slice 2a): the reported drug, a signed-ready
//! version body, the authoring-door submit, the direct floor call and the `rig!` macro. Shared by
//! `medication_set_aside_floor.rs` and `medication_set_aside_peer.rs` (split for the size cap).
//!
//! Include it with `#[macro_use] #[path = "common/set_aside_kit.rs"] mod set_aside_kit;` AFTER
//! `mod common;` and `#[path = "common/medication_kit.rs"] mod medication_kit;`.
#![allow(dead_code)] // each including suite uses a different subset

use crate::medication_kit::*;
use cairn_event::medication::{
    render_set_aside_twin, set_aside_body, MedicationAssertion, MedicationSetAside, SourceNote,
    SET_ASIDE_EVENT_TYPE, SET_ASIDE_SCHEMA,
};
use cairn_event::{ClockGrade, EventBody, SigningKey};
use cairn_node::db;
use serde_json::{json, Value};
use tokio_postgres::Client;
use uuid::Uuid;

/// The drug the source reported (the "old" Lipitor 40 mg of a GP letter).
pub fn drug() -> MedicationAssertion<'static> {
    MedicationAssertion {
        medication_id: "",
        term: "Lipitor",
        coding: None,
        formulation: Some("tablet"),
        dose_amount: Some("40"),
        dose_unit: Some("mg"),
        sig: Some("one BD"),
        info_source: "external-record",
        started: None,
        started_precision: None,
        safety: None,
        charting: None,
        source_note: Some(SourceNote {
            source_detail: Some("GP letter, 2026-03"),
            why: Some("packet says 25 mg"),
        }),
    }
}

/// A signed-ready set-aside body on `patient`, beside thread `alt`.
pub async fn version_body(
    c: &Client,
    kid: &str,
    patient: Uuid,
    version: Uuid,
    alt: Uuid,
    tweak: impl FnOnce(&mut Value),
) -> EventBody {
    version_body_of(c, kid, patient, version, alt, &drug(), tweak).await
}

/// [`version_body`] of the drug `d` (its twin rendered from `d`, as the window's is).
pub async fn version_body_of(
    c: &Client,
    kid: &str,
    patient: Uuid,
    version: Uuid,
    alt: Uuid,
    d: &MedicationAssertion<'_>,
    tweak: impl FnOnce(&mut Value),
) -> EventBody {
    let (v, a) = (version.to_string(), alt.to_string());
    let s = MedicationSetAside {
        version_id: &v,
        alternative_to: &a,
        drug: d,
    };
    let mut payload = set_aside_body(&s);
    tweak(&mut payload);
    EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: SET_ASIDE_EVENT_TYPE.into(),
        schema_version: SET_ASIDE_SCHEMA.into(),
        hlc: db::next_hlc(c, "test-node").await.unwrap(),
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: json!([{"actor_id": kid, "role": "recorded"}]),
        payload,
        attachments: vec![],
        plaintext_twin: Some(render_set_aside_twin(&s)),
        clock_grade: ClockGrade::SelfAsserted,
        safety: None,
    }
}

/// Submit a version through the strict (authoring) door; returns its event id.
pub async fn add_version(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    patient: Uuid,
    alt: Uuid,
) -> Result<Uuid, String> {
    let body = version_body(c, kid, patient, Uuid::now_v7(), alt, |_| {}).await;
    let id: Uuid = body.event_id.parse().unwrap();
    seal_and_submit(c, sk, body)
        .await
        .map(|_| id)
        .map_err(|e| db_msg(&e))
}

/// Call the set-aside floor directly on `body` at one door, in a throwaway transaction.
pub async fn floor(c: &mut Client, body: &Value, remote: bool) -> Result<(), String> {
    let tx = c.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('cairn.remote_apply', $1, true)",
        &[&if remote { "on" } else { "" }],
    )
    .await
    .unwrap();
    let r = tx
        .execute(
            "SELECT cairn_check_medication_set_aside($1, $2::text::jsonb)",
            &[&SET_ASIDE_EVENT_TYPE, &body.to_string()],
        )
        .await;
    tx.rollback().await.unwrap();
    r.map(|_| ()).map_err(|e| db_msg(&e))
}

/// A bare floor body (payload + schema + patient) for the direct calls.
pub fn floor_body(payload: Value) -> Value {
    json!({"schema_version": SET_ASIDE_SCHEMA, "patient_id": Uuid::now_v7().to_string(),
           "payload": payload})
}

pub async fn count(c: &Client, sql: &str, id: Uuid) -> i64 {
    c.query_one(sql, &[&id.to_string()]).await.unwrap().get(0)
}

macro_rules! rig {
    ($c:ident, $sk:ident, $kid:ident) => {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let _guard = db::test_serial_guard(&base).await.unwrap();
        #[allow(unused_mut)]
        let mut $c = db::connect_and_load_schema(&base).await.unwrap();
        let ($sk, $kid) = setup_node(&$c).await;
    };
}
