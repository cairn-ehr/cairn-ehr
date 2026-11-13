//! The medication-restore suites' fixtures (batch charting slice 2b, db/080 §1 — not the DR
//! restore of `restore_kit.rs`): a chart with a set-aside version, a restore line, the
//! companion's stop, and the one-transaction strict-door submit. Shared by
//! `medication_restore_floor.rs` and `medication_restore_replaces.rs` (split for the size cap).
//!
//! Include it with `#[path = "common/med_restore_kit.rs"] mod med_restore_kit;` AFTER
//! `mod common;`, `medication_kit` and `set_aside_kit`.
#![allow(dead_code)] // each including suite uses a different subset

use crate::medication_kit::*;
use crate::set_aside_kit::*;
use cairn_event::medication::Charting;
use cairn_event::{sign, EventBody, SigningKey};
use cairn_node::db;
use cairn_node::medication::{build_cease_body, build_charted_body, CeaseMedicationInput};
use tokio_postgres::Client;
use uuid::Uuid;

/// A registered chart with one current line `med` and a set-aside version `version` beside it.
/// Returns (patient, med, version).
pub async fn chart_with_version(c: &mut Client, sk: &SigningKey, kid: &str) -> (Uuid, Uuid, Uuid) {
    let (p, med) = chart_with_one_med(c, sk, kid).await;
    let version = version_beside(c, sk, kid, p, med).await;
    (p, med, version)
}

/// Set a new version aside beside the current line `med`; returns its version_id.
pub async fn version_beside(c: &Client, sk: &SigningKey, kid: &str, p: Uuid, med: Uuid) -> Uuid {
    let version = Uuid::now_v7();
    let body = version_body(c, kid, p, version, med, |_| {}).await;
    seal_and_submit(c, sk, body)
        .await
        .map_err(|e| db_msg(&e))
        .expect("the version is admitted");
    version
}

/// A restore line: a NEW thread on `patient` naming `version` in `restores`. Returns the body
/// and the new thread's id.
pub async fn restore_body(
    c: &Client,
    kid: &str,
    patient: Uuid,
    version: Uuid,
) -> (EventBody, Uuid) {
    let med = Uuid::now_v7();
    (restore_body_on(c, kid, patient, version, med).await, med)
}

/// [`restore_body`] on the thread `med` (a new one, unless a test means to break that rule).
pub async fn restore_body_on(
    c: &Client,
    kid: &str,
    patient: Uuid,
    version: Uuid,
    med: Uuid,
) -> EventBody {
    let v = version.to_string();
    let charting = Charting {
        restores: Some(&v),
        ..Charting::default()
    };
    let hlc = db::next_hlc(c, "test-node").await.unwrap();
    build_charted_body(
        Uuid::now_v7(),
        med,
        patient,
        &sample_input(),
        &charting,
        kid,
        hlc,
        None,
    )
}

/// The companion's stop, as the restore's first event.
pub async fn stop_body(c: &Client, kid: &str, patient: Uuid, med: Uuid) -> EventBody {
    let none = CeaseMedicationInput {
        stopped: None,
        stopped_precision: None,
        reason: None,
    };
    let hlc = db::next_hlc(c, "test-node").await.unwrap();
    build_cease_body(Uuid::now_v7(), med, patient, &none, kid, hlc)
}

/// Seal, sign and submit `bodies` through the strict door in ONE transaction, then commit.
/// `Err` is the refusal text, whether a submit refused or the COMMIT did.
pub async fn submit_together(
    c: &mut Client,
    sk: &SigningKey,
    bodies: Vec<EventBody>,
) -> Result<(), String> {
    submit_with(c, sk, bodies, false).await
}

/// [`submit_together`], optionally under `SET CONSTRAINTS ALL IMMEDIATE` (the pairing check
/// then runs as each line is submitted, not at COMMIT).
pub async fn submit_with(
    c: &mut Client,
    sk: &SigningKey,
    bodies: Vec<EventBody>,
    immediate: bool,
) -> Result<(), String> {
    let sealed: Vec<Sealed> = bodies.into_iter().map(|b| seal_sign(b, sk)).collect();
    submit_sealed(c, &sealed, immediate).await
}

/// A sealed, signed event ready for the strict door: (signed bytes, its DEK).
pub type Sealed = (Vec<u8>, Vec<u8>);

/// Seal a CLEAR body as the node write path does, and sign it. Pure apart from the generated
/// DEK (never a literal). Kept apart from the submit so a test can submit the SAME bytes twice.
pub fn seal_sign(mut body: EventBody, sk: &SigningKey) -> Sealed {
    let twin = body.plaintext_twin.take().unwrap();
    let (container, dek) =
        cairn_event::seal::seal_event_payload(&body.payload, &twin, &body.event_id).unwrap();
    body.payload = container;
    body.plaintext_twin = Some(cairn_event::seal::seal_stub_twin(&body.event_type));
    let signed = sign(&body, sk).unwrap();
    (signed.signed_bytes.to_vec(), dek.as_bytes().to_vec())
}

/// Submit already-sealed events through the strict door in ONE transaction, then commit.
pub async fn submit_sealed(
    c: &mut Client,
    sealed: &[Sealed],
    immediate: bool,
) -> Result<(), String> {
    let tx = c.transaction().await.unwrap();
    if immediate {
        tx.batch_execute("SET CONSTRAINTS ALL IMMEDIATE")
            .await
            .unwrap();
    }
    for (bytes, dek) in sealed {
        tx.execute("SELECT submit_event($1, NULL, NULL, $2)", &[bytes, dek])
            .await
            .map_err(|e| db_msg(&e))?;
    }
    tx.commit().await.map_err(|e| db_msg(&e))
}

/// The version thread `med` restores, as `medication_restore` projects it (None = no row).
pub async fn projected_restores(c: &Client, med: Uuid) -> Option<Uuid> {
    c.query_opt(
        "SELECT version_id::text FROM medication_restore WHERE medication_id = $1::text::uuid",
        &[&med.to_string()],
    )
    .await
    .unwrap()
    .map(|r| r.get::<_, String>(0).parse().unwrap())
}

pub async fn pending_rows(c: &Client) -> i64 {
    c.query_one("SELECT count(*) FROM medication_restore_pending", &[])
        .await
        .unwrap()
        .get(0)
}
