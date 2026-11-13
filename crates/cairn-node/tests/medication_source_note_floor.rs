//! Batch charting slice 2a, Task 2 — the in-database floor and the projection for the two
//! optional free-text keys on a medication assertion, `source_detail` ("From:") and `why`
//! (db/079 §1 for the floor, db/077 §2 for the `medication_charting` columns).
//!
//! The floor has two tiers (ADR-0056): the key's JSON TYPE is judged at BOTH doors (the
//! projection must never be handed a non-string), while "not blank" and "no control
//! character" are authoring-door-only so a newer peer's words never wedge sync.
//!
//! DB-gated on `$CAIRN_TEST_PG` (a skipped run prints `skipped:`), serialized via
//! `db::test_serial_guard`.
use cairn_event::medication::{
    medication_assertion_body, render_medication_twin, Charting, MedicationAssertion, SourceNote,
};
use cairn_event::SigningKey;
use cairn_node::db;
use cairn_node::medication::{build_assert_body, AssertMedicationInput};
use cairn_node::shred::shred_event;
use serde_json::{json, Value};
use tokio_postgres::Client;
use uuid::Uuid;

mod common;
#[path = "common/medication_kit.rs"]
mod medication_kit;
use medication_kit::*;

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

/// A plain assertion payload as the node writes it, for the direct floor calls.
fn base_payload() -> Value {
    build_assert_body(
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        &input(),
        "kid",
        cairn_event::Hlc {
            wall: 1_782_000_000_000,
            counter: 0,
            node_origin: "peer".into(),
        },
        None,
    )
    .payload
}

/// Call the assertion floor directly on `payload` at one door in a throwaway transaction.
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

/// A payload with one extra top-level key.
fn with_key(k: &str, v: Value) -> Value {
    let mut p = base_payload();
    p[k] = v;
    p
}

/// A born-sealed line on a fresh registered chart carrying `note` (and no other charting).
async fn chart_line_with_note(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    note: SourceNote<'_>,
) -> (Uuid, Uuid) {
    let patient = Uuid::now_v7();
    common::submit_registration(c, sk, kid, patient, 0).await;
    let (event_id, med) = (Uuid::now_v7(), Uuid::now_v7());
    let hlc = db::next_hlc(c, "test-node").await.unwrap();
    let i = input();
    let mut body = build_assert_body(event_id, med, patient, &i, kid, hlc, None);
    let mid = med.to_string();
    let a = MedicationAssertion {
        medication_id: &mid,
        term: i.term,
        coding: i.coding,
        formulation: i.formulation,
        dose_amount: i.dose_amount,
        dose_unit: i.dose_unit,
        sig: i.sig,
        source_note: Some(note),
        info_source: i.info_source,
        started: i.started,
        started_precision: i.started_precision,
        safety: None,
        charting: Some(Charting::default()),
    };
    body.payload = medication_assertion_body(&a);
    body.plaintext_twin = Some(render_medication_twin(&a));
    seal_and_submit(c, sk, body)
        .await
        .expect("a line with a source note signs through the strict door");
    (med, event_id)
}

/// The thread's (source_detail, why), None = no charting row.
async fn note_row(c: &Client, med: Uuid) -> Option<(Option<String>, Option<String>)> {
    c.query_opt(
        "SELECT source_detail, why FROM medication_charting WHERE medication_id = $1::text::uuid",
        &[&med.to_string()],
    )
    .await
    .unwrap()
    .map(|r| (r.get(0), r.get(1)))
}

macro_rules! rig {
    ($c:ident, $sk:ident, $kid:ident) => {
        let Some(base) = common::cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let _guard = db::test_serial_guard(&base).await.unwrap();
        #[allow(unused_mut)]
        let mut $c = db::connect_and_load_schema(&base).await.unwrap();
        let ($sk, $kid) = setup_node(&$c).await;
    };
}

const NOTE: SourceNote<'static> = SourceNote {
    source_detail: Some("GP letter, 2026-03"),
    why: Some("started here for the febrile child"),
};

#[tokio::test]
async fn a_charted_line_with_source_and_why_projects_both() {
    rig!(c, sk, kid);
    let (med, _) = chart_line_with_note(&c, &sk, &kid, NOTE).await;
    assert_eq!(
        note_row(&c, med).await,
        Some((
            Some("GP letter, 2026-03".into()),
            Some("started here for the febrile child".into())
        ))
    );
    // Either key alone is enough to project a row (the other is an honest NULL).
    let (med, _) = chart_line_with_note(
        &c,
        &sk,
        &kid,
        SourceNote {
            source_detail: None,
            why: Some("only why"),
        },
    )
    .await;
    assert_eq!(
        note_row(&c, med).await,
        Some((None, Some("only why".into())))
    );
}

#[tokio::test]
async fn a_non_string_key_is_refused_at_both_doors() {
    rig!(c, _sk, _kid);
    for k in ["source_detail", "why"] {
        for bad in [
            json!(7),
            json!(true),
            json!(["x"]),
            json!({"a": 1}),
            json!(null),
        ] {
            for remote in [false, true] {
                let e = floor(&mut c, &with_key(k, bad.clone()), remote)
                    .await
                    .expect_err(&format!("{k}={bad} must be refused (remote={remote})"));
                assert!(e.contains(k), "the refusal names the key: {e}");
            }
        }
        floor(&mut c, &with_key(k, json!("fine")), false)
            .await
            .unwrap();
        floor(&mut c, &with_key(k, json!("fine")), true)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn a_blank_or_control_char_is_refused_at_the_authoring_door_only() {
    rig!(c, _sk, _kid);
    for k in ["source_detail", "why"] {
        for bad in ["", "   ", "\t\n ", "a\nb", "bell\u{7}", "x\u{1f}y"] {
            let p = with_key(k, json!(bad));
            floor(&mut c, &p, false).await.expect_err(&format!(
                "{k}={bad:?} must be refused at the authoring door"
            ));
            floor(&mut c, &p, true)
                .await
                .unwrap_or_else(|e| panic!("{k}={bad:?} must pass the remote door: {e}"));
        }
    }
}

#[tokio::test]
async fn a_shred_removes_them_with_the_charting_row() {
    rig!(c, sk, kid);
    let (med, event) = chart_line_with_note(&c, &sk, &kid, NOTE).await;
    assert!(note_row(&c, med).await.is_some());
    shred_event(
        &mut c,
        &sk,
        &kid,
        "test-node",
        event,
        "retention ceiling",
        None,
    )
    .await
    .expect("shred");
    assert_eq!(
        note_row(&c, med).await,
        None,
        "free text is clinical: the shred takes it"
    );
}

#[tokio::test]
async fn a_reprojection_rebuilds_them() {
    rig!(c, sk, kid);
    let (med, _) = chart_line_with_note(&c, &sk, &kid, NOTE).await;
    c.batch_execute("TRUNCATE medication_charting")
        .await
        .unwrap();
    assert_eq!(note_row(&c, med).await, None);
    c.execute(
        "SELECT count(*) FROM cairn_reproject('clinical.medication', true, 'test')",
        &[],
    )
    .await
    .unwrap();
    assert_eq!(
        note_row(&c, med).await,
        Some((
            Some("GP letter, 2026-03".into()),
            Some("started here for the febrile child".into())
        ))
    );
}
