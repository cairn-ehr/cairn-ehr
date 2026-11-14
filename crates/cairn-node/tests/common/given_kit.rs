//! The given-dose suites' fixtures (charting slice 2): a line (stat or not), its paired stop, a
//! given, and the stop+given signed in ONE transaction as the window signs them. Shared by
//! `medication_given_floor.rs` (Task 2) and `medication_given_retraction_floor.rs` (Task 3).
//!
//! Include it with `#[path = "common/given_kit.rs"] mod given_kit;` AFTER `mod common;` and
//! `#[path = "common/medication_kit.rs"] mod medication_kit;`.
#![allow(dead_code)] // each including suite uses a different subset

use crate::common;
use crate::medication_kit::*;
use cairn_event::medication::{
    given_body, render_given_twin, Charting, Given, GIVEN_EVENT_TYPE, GIVEN_SCHEMA,
    STAT_GIVEN_REASON,
};
use cairn_event::{EventBody, SigningKey};
use cairn_node::db;
use cairn_node::medication::{build_cease_body, build_charted_body, CeaseMedicationInput};
use serde_json::json;
use tokio_postgres::Client;
use uuid::Uuid;

/// A line on `patient` through the strict door: a stat order when `stat`, else a line charted
/// with a route only (so it has a charting row that is NOT a stat order). Returns its thread.
pub async fn line(c: &Client, sk: &SigningKey, kid: &str, patient: Uuid, stat: bool) -> Uuid {
    let med = Uuid::now_v7();
    let charting = Charting {
        route: (!stat).then_some("po"),
        stat,
        ..Charting::default()
    };
    let hlc = db::next_hlc(c, "test-node").await.unwrap();
    let body = build_charted_body(
        Uuid::now_v7(),
        med,
        patient,
        &sample_input(),
        &charting,
        kid,
        hlc,
        None,
    );
    seal_and_submit(c, sk, body)
        .await
        .map_err(|e| db_msg(&e))
        .expect("the line signs");
    med
}

/// A fresh registered chart with one line on it. Returns (patient, thread).
pub async fn chart(c: &Client, sk: &SigningKey, kid: &str, stat: bool) -> (Uuid, Uuid) {
    let patient = Uuid::now_v7();
    common::submit_registration(c, sk, kid, patient, 0).await;
    (patient, line(c, sk, kid, patient, stat).await)
}

/// The paired stop of a stat give ("stat dose given", no `stopped`).
pub async fn stop_body(c: &Client, kid: &str, patient: Uuid, med: Uuid) -> EventBody {
    let input = CeaseMedicationInput {
        stopped: None,
        stopped_precision: None,
        reason: Some(STAT_GIVEN_REASON),
    };
    let hlc = db::next_hlc(c, "test-node").await.unwrap();
    build_cease_body(Uuid::now_v7(), med, patient, &input, kid, hlc)
}

/// A signed-ready given of `med` on `patient`, given "at the HLC wall"; `completes` names a stop.
pub async fn given(
    c: &Client,
    kid: &str,
    patient: Uuid,
    med: Uuid,
    completes: Option<Uuid>,
) -> EventBody {
    let (m, k) = (med.to_string(), completes.map(|u| u.to_string()));
    let g = Given {
        medication_id: &m,
        completes: k.as_deref(),
    };
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: GIVEN_EVENT_TYPE.into(),
        schema_version: GIVEN_SCHEMA.into(),
        hlc: db::next_hlc(c, "test-node").await.unwrap(),
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: json!([{"actor_id": kid, "role": "recorded"}]),
        payload: given_body(&g),
        attachments: vec![],
        plaintext_twin: Some(render_given_twin(&g)),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    };
    effective_at_wall(c, body).await
}

/// Submit `body` through the strict door; its event id, or the refusal's text.
pub async fn submit(c: &Client, sk: &SigningKey, body: EventBody) -> Result<Uuid, String> {
    let id: Uuid = body.event_id.parse().unwrap();
    seal_and_submit(c, sk, body)
        .await
        .map(|_| id)
        .map_err(|e| db_msg(&e))
}

/// A stop then its given, in ONE transaction (the window's shape). Returns (stop, given).
pub async fn give_stat(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    patient: Uuid,
    med: Uuid,
) -> Result<(Uuid, Uuid), String> {
    c.batch_execute("BEGIN").await.unwrap();
    let r = async {
        let stop = submit(c, sk, stop_body(c, kid, patient, med).await).await?;
        let g = submit(c, sk, given(c, kid, patient, med, Some(stop)).await).await?;
        Ok((stop, g))
    }
    .await;
    c.batch_execute(if r.is_ok() { "COMMIT" } else { "ROLLBACK" })
        .await
        .unwrap();
    r
}
