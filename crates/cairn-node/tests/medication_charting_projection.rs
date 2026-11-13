//! Charting slice 1a, Task 3 — the charting projection (`medication_charting`, db/077 §2).
//!
//! A charted line's five charting keys (charted-as, route, strength, components, warning
//! override) land in their OWN table, written by a second apply fn on
//! `clinical.medication.asserted`, so no medication view is widened across files. This file
//! pins that the row is written from the clear payload of a born-sealed assertion, that an
//! assertion without charting writes no row, that a shred takes the row with it (an override
//! reason is clinical text), that the generic rebuild (ADR-0057) restores it, and that a peer's
//! newer word (admitted by the remote door, Task 1) still projects rather than wedging sync.
//!
//! The line is hand-built (the node's orchestrator gains charting in Task 4): the node's
//! `build_assert_body` supplies the envelope, and cairn-event's own builder supplies a payload
//! and twin that carry the charting keys.
//!
//! DB-gated on `$CAIRN_TEST_PG` (a skipped run prints `skipped:`), serialized via
//! `db::test_serial_guard`.
use cairn_event::medication::{
    medication_assertion_body, render_medication_twin, ChartedAs, Charting, Component,
    MedicationAssertion, OverrideRef, SubstanceCoding, WarningOverride, ALLERGY_SOURCE,
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

/// Any well-formed moiety uuid will do: the floor checks a coding's shape, never drugref.
const AMOX: &str = "0f8c4b1e-1b7a-5c2d-9a3e-2b6f7c8d9e01";

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

/// The node's assertion envelope for a new thread on `patient`, with the payload and twin
/// rebuilt by cairn-event so they carry `charting`. Returns (body, medication_id).
async fn charted_body(
    c: &Client,
    kid: &str,
    patient: Uuid,
    origin: &str,
    charting: Charting<'_>,
) -> (cairn_event::EventBody, Uuid) {
    let (event_id, med) = (Uuid::now_v7(), Uuid::now_v7());
    let hlc = db::next_hlc(c, origin).await.unwrap();
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
        source_note: None,
        info_source: i.info_source,
        started: i.started,
        started_precision: i.started_precision,
        safety: None,
        charting: Some(charting),
    };
    body.payload = medication_assertion_body(&a);
    body.plaintext_twin = Some(render_medication_twin(&a));
    (body, med)
}

/// A registered chart, and a born-sealed charted line on it through the strict door.
/// Returns (patient, medication_id, event_id).
async fn chart_line(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    charting: Charting<'_>,
) -> (Uuid, Uuid, Uuid) {
    let patient = Uuid::now_v7();
    common::submit_registration(c, sk, kid, patient, 0).await;
    let (body, med) = charted_body(c, kid, patient, "test-node", charting).await;
    let event_id: Uuid = body.event_id.parse().unwrap();
    seal_and_submit(c, sk, body)
        .await
        .expect("a charted line signs through the strict door");
    (patient, med, event_id)
}

/// The full charting the first test signs: every key present.
fn full<'a>(components: &'a [Component<'a>], against: &'a [OverrideRef<'a>]) -> Charting<'a> {
    Charting {
        charted_as: Some(ChartedAs::StartedHere),
        route: Some("po"),
        strength: Some("875/125 mg"),
        components,
        warning_override: Some(WarningOverride {
            reason: "no alternative in the remote clinic",
            against,
        }),
    }
}

/// One `against` entry naming the allergy record `id`.
fn allergy(id: &str) -> OverrideRef<'_> {
    OverrideRef {
        source: ALLERGY_SOURCE,
        reference: id,
    }
}

fn components() -> [Component<'static>; 2] {
    [
        Component {
            term: "amoxicillin",
            coding: Some(SubstanceCoding {
                system: "drugref-moiety",
                code: AMOX,
                display: "amoxicillin",
            }),
        },
        Component {
            term: "clavulanic acid",
            coding: None,
        },
    ]
}

/// The thread's charting row as one JSON object (None = no row), so a test compares the
/// whole row at once. `override_against` is jsonb: the `[{source, ref}]` array as signed.
async fn charting_row(c: &Client, med: Uuid) -> Option<Value> {
    c.query_opt(
        "SELECT jsonb_build_object('patient_id', patient_id::text, 'charted_as', charted_as, \
                'route', route, 'strength', strength, 'components', components, \
                'override_reason', override_reason, \
                'override_against', override_against)::text \
           FROM medication_charting WHERE medication_id = $1::text::uuid",
        &[&med.to_string()],
    )
    .await
    .unwrap()
    .map(|r| serde_json::from_str(r.get::<_, &str>(0)).unwrap())
}

/// The row the full charting must project for `patient`, overriding `e1`.
fn expected_full(patient: Uuid, e1: &str) -> Value {
    json!({
        "patient_id": patient.to_string(),
        "charted_as": "started-here",
        "route": "po",
        "strength": "875/125 mg",
        "components": [
            {"term": "amoxicillin",
             "coding": {"system": "drugref-moiety", "code": AMOX, "display": "amoxicillin"}},
            {"term": "clavulanic acid", "coding": null}
        ],
        "override_reason": "no alternative in the remote clinic",
        "override_against": [{"source": "allergy", "ref": e1}],
    })
}

/// A schema-loaded connection with the serial guard, the node's device key and unwrap key.
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

#[tokio::test]
async fn a_charted_line_projects_its_fields() {
    rig!(c, sk, kid);
    let e1 = Uuid::now_v7().to_string();
    let (parts, against) = (components(), [allergy(&e1)]);
    let (patient, med, _) = chart_line(&c, &sk, &kid, full(&parts, &against)).await;
    assert_eq!(
        charting_row(&c, med).await,
        Some(expected_full(patient, &e1))
    );
}

#[tokio::test]
async fn an_event_without_charting_projects_no_row() {
    rig!(c, sk, kid);
    let (_, med, _) = chart_line(&c, &sk, &kid, Charting::default()).await;
    assert_eq!(charting_row(&c, med).await, None);
    // ...and a line carrying only a route still projects (any one key is a charting).
    let only_route = Charting {
        route: Some("iv"),
        ..Charting::default()
    };
    let (_, med, _) = chart_line(&c, &sk, &kid, only_route).await;
    let row = charting_row(&c, med).await.expect("a route alone projects");
    assert_eq!(row["route"], "iv");
    assert_eq!(
        row["override_against"],
        Value::Null,
        "no override: NULL, never an error"
    );
    assert_eq!(row["components"], Value::Null);
}

#[tokio::test]
async fn a_shred_removes_the_charting_row() {
    rig!(c, sk, kid);
    let e1 = Uuid::now_v7().to_string();
    let (parts, against) = (components(), [allergy(&e1)]);
    let (_, med, event) = chart_line(&c, &sk, &kid, full(&parts, &against)).await;
    assert!(charting_row(&c, med).await.is_some());
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
        charting_row(&c, med).await,
        None,
        "an override reason is clinical text: the shred takes the row (ADR-0005 rung 3)"
    );
}

#[tokio::test]
async fn a_reprojection_rebuilds_it() {
    rig!(c, sk, kid);
    let e1 = Uuid::now_v7().to_string();
    let (parts, against) = (components(), [allergy(&e1)]);
    let (patient, med, _) = chart_line(&c, &sk, &kid, full(&parts, &against)).await;
    c.batch_execute("TRUNCATE medication_charting")
        .await
        .unwrap();
    assert_eq!(charting_row(&c, med).await, None);
    c.execute(
        "SELECT count(*) FROM cairn_reproject('clinical.medication', true, 'test')",
        &[],
    )
    .await
    .unwrap();
    assert_eq!(
        charting_row(&c, med).await,
        Some(expected_full(patient, &e1))
    );
}

#[tokio::test]
async fn a_peer_newer_word_still_projects() {
    rig!(c, sk, kid);
    // A word this build does not know, a one-element combination, and an override against a
    // warning source this build does not know (with a ref that is no uuid, and a second entry
    // missing its ref): refused at the authoring door, admitted from a peer (Task 1). The
    // projection must store each as given, not raise.
    let patient = Uuid::now_v7();
    common::submit_registration(&c, &sk, &kid, patient, 0).await;
    let (mut body, med) = charted_body(&c, &kid, patient, "peer-node", Charting::default()).await;
    let one = json!([{"term": "amoxicillin", "coding": null}]);
    body.payload["charted_as"] = json!("transferred-in");
    body.payload["substance"]["components"] = one.clone();
    let against = json!([{"source": "renal-dose", "ref": "egfr-below-30"}, {"source": "allergy"}]);
    body.payload["warning_override"] = json!({"reason": "dose halved", "against": against});
    apply_remote(&c, &sk, &body)
        .await
        .expect("the remote door admits a peer's newer word");
    let row = charting_row(&c, med)
        .await
        .expect("the newer word projects");
    assert_eq!(row["charted_as"], "transferred-in");
    assert_eq!(row["components"], one);
    assert_eq!(row["override_reason"], "dose halved");
    assert_eq!(row["override_against"], against);
}

#[tokio::test]
async fn the_chart_gesture_is_a_timing_kind() {
    // db/044's closed kind list gains the window's charting gesture (Task 5 records it).
    rig!(c, _sk, _kid);
    let tx = c.transaction().await.unwrap();
    tx.execute(
        "INSERT INTO ui_gesture_timing (gesture_kind, size_bucket, n) \
         VALUES ('medication-chart', '1-3', 1) \
         ON CONFLICT (gesture_kind, size_bucket) DO NOTHING",
        &[],
    )
    .await
    .expect("medication-chart is a timing kind");
    tx.rollback().await.unwrap();
}
