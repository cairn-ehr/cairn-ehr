//! The first clinical READ path (#288 med-list slice): `list_patient_medications` over the
//! existing medication projections.
//!
//! The group/thread asymmetry is what these tests exist for. `patient_medication_current`
//! emits one row per GROUP (reconciled duplicates collapse, ADR-0047) while attestation is
//! per THREAD (ADR-0049). Every test below pins one way that asymmetry can be got wrong.
//!
//! DB-gated on $CAIRN_TEST_PG, serialized cluster-wide via db::test_serial_guard. Key
//! material is minted at runtime (house rule 6).
mod common;

use cairn_event::SigningKey;
use cairn_medication_view::{MedicationStatus, VouchState};
use cairn_node::db;
use cairn_node::medication::read::list_patient_medications;
use cairn_node::medication::{
    assert_medication, attest_medication_thread, cease_medication, reconcile_medications,
    AssertMedicationInput, AttestParams, CeaseMedicationInput, ReconcileInput,
};
use common::{cs, medication_setup as setup};
use tokio_postgres::Client;
use uuid::Uuid;

/// Assert one medication and return its thread id.
async fn assert_one(
    c: &mut Client,
    sk: &SigningKey,
    kid: &str,
    origin: &str,
    patient: Uuid,
    term: &str,
) -> Uuid {
    assert_medication(
        c,
        sk,
        kid,
        origin,
        patient,
        &AssertMedicationInput {
            term,
            coding: None,
            formulation: None,
            dose_amount: Some("500"),
            dose_unit: Some("mg"),
            sig: None,
            info_source: "patient",
            started: None,
            started_precision: None,
        },
        None,
        None,
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn a_single_unvouched_medication_reads_as_absent() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let (sk, kid, _hsk, _hkid) = setup(&c).await;
    let patient = Uuid::now_v7();

    let thread = assert_one(&mut c, &sk, &kid, "origin-a", patient, "metformin").await;

    let rows = list_patient_medications(&c, patient).await.unwrap();
    assert_eq!(rows.len(), 1, "one assert, one displayed row");
    assert_eq!(rows[0].term, "metformin");
    assert_eq!(rows[0].status, MedicationStatus::Active);
    assert_eq!(rows[0].members.len(), 1);
    assert_eq!(rows[0].members[0].medication_id, thread);
    assert_eq!(rows[0].members[0].vouch, VouchState::Absent);
}

#[tokio::test]
async fn an_attested_thread_reads_as_fresh_with_its_attester() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let (sk, kid, hsk, hkid) = setup(&c).await;
    let patient = Uuid::now_v7();

    let thread = assert_one(&mut c, &sk, &kid, "origin-a", patient, "metformin").await;
    let params = AttestParams {
        human_sk: &hsk,
        human_kid: &hkid,
        basis: None,
        note: None,
    };
    attest_medication_thread(&mut c, &sk, "origin-a", &params, patient, thread)
        .await
        .unwrap();

    let rows = list_patient_medications(&c, patient).await.unwrap();
    assert_eq!(
        rows[0].members[0].vouch,
        VouchState::Fresh { by: hkid.clone() }
    );
}

/// A reconciled pair is ONE row over TWO member threads — the group/thread asymmetry.
#[tokio::test]
async fn a_reconciled_pair_reads_as_one_row_with_two_members() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let (sk, kid, _hsk, _hkid) = setup(&c).await;
    let patient = Uuid::now_v7();

    let a = assert_one(&mut c, &sk, &kid, "origin-a", patient, "metformin").await;
    let b = assert_one(&mut c, &sk, &kid, "origin-a", patient, "Metformin XR").await;
    // DEVIATION FROM THE BRIEF: the brief's `ReconcileInput { patient, thread_a, thread_b,
    // note }` and a 3-positional-arg `reconcile_medications` do not match the orchestrator
    // that Task 1's review cycle actually landed (`crates/cairn-node/src/medication/
    // reconciliation.rs`). The real shapes are `ReconcileInput { provenance, reason }` and
    // `reconcile_medications(client, node_sk, node_kid, node_origin, patient, subject_a,
    // subject_b, input, author, attest)` — patient and the two subject threads are separate
    // positional arguments, not struct fields. See task-2-report.md for detail.
    reconcile_medications(
        &mut c,
        &sk,
        &kid,
        "origin-a",
        patient,
        a,
        b,
        &ReconcileInput {
            provenance: "clinician-judgment",
            reason: None,
        },
        None,
        None,
    )
    .await
    .unwrap();

    let rows = list_patient_medications(&c, patient).await.unwrap();
    assert_eq!(rows.len(), 1, "a reconciled pair collapses to ONE displayed row");
    let mut members: Vec<Uuid> = rows[0].members.iter().map(|m| m.medication_id).collect();
    members.sort();
    let mut expected = vec![a, b];
    expected.sort();
    assert_eq!(members, expected, "both threads are members of the one row");
}

/// A ceased medication stays VISIBLE, marked ceased — a struck line on a paper chart is
/// not erased (refinement 2 of the plan).
#[tokio::test]
async fn a_ceased_medication_is_retained_and_marked_ceased() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let (sk, kid, _hsk, _hkid) = setup(&c).await;
    let patient = Uuid::now_v7();

    let thread = assert_one(&mut c, &sk, &kid, "origin-a", patient, "metformin").await;
    cease_medication(
        &mut c,
        &sk,
        &kid,
        "origin-a",
        patient,
        thread,
        &CeaseMedicationInput {
            stopped: None,
            stopped_precision: None,
            reason: Some("rash"),
        },
        None,
        None,
    )
    .await
    .unwrap();

    let rows = list_patient_medications(&c, patient).await.unwrap();
    assert_eq!(rows.len(), 1, "a ceased drug is still on the chart");
    assert_eq!(rows[0].status, MedicationStatus::Ceased);
}

#[tokio::test]
async fn another_patients_medications_are_not_returned() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let (sk, kid, _hsk, _hkid) = setup(&c).await;
    let mine = Uuid::now_v7();
    let theirs = Uuid::now_v7();

    assert_one(&mut c, &sk, &kid, "origin-a", theirs, "warfarin").await;

    assert!(list_patient_medications(&c, mine).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_patient_with_no_medications_reads_as_an_empty_list() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let _ = setup(&c).await;

    assert!(list_patient_medications(&c, Uuid::now_v7())
        .await
        .unwrap()
        .is_empty());
}
