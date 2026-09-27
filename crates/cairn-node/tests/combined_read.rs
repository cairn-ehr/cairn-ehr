//! ADR-0076 R1: a linked chart reads as ONE list over its chart set, each row naming its
//! source chart; a group is found through its members (the #334 fix); a group reaching a
//! chart OUTSIDE the set is still a hazard; the same drug on two linked charts is flagged.
//!
//! DB-gated on $CAIRN_TEST_PG, serialized cluster-wide via `db::test_serial_guard`, taken
//! BEFORE connecting (see `medication_read.rs`'s header for why that order is load-bearing
//! for this read in particular). Key material is minted at runtime (house rule 6).
mod common;
use cairn_event::SigningKey;
use cairn_node::db;
use cairn_node::medication::read::list_patient_medications;
use cairn_node::medication::{
    assert_medication, attest_medication_thread, cease_medication, reconcile_medications,
    AssertMedicationInput, AttestParams, CeaseMedicationInput, ReconcileInput, SubstanceCoding,
};
use common::{cs, medication_setup as setup, submit_registration};
use serde_json::Value;
use std::collections::HashMap;
use tokio_postgres::Client;
use uuid::Uuid;

/// One registered chart (#345: the birth act precedes everything recorded about it).
async fn chart(c: &Client, sk: &SigningKey, kid: &str) -> Uuid {
    let p = Uuid::now_v7();
    submit_registration(c, sk, kid, p, 0).await;
    p
}

/// Assert one active medication on `patient`, optionally coded (ADR-0059); returns its
/// thread id. The one place this suite builds an `AssertMedicationInput`, so every drug in
/// it carries the same 500 mg dose and differs only in what the test is about.
async fn assert_drug(
    c: &mut Client,
    sk: &SigningKey,
    kid: &str,
    patient: Uuid,
    term: &str,
    coding: Option<SubstanceCoding<'_>>,
) -> Uuid {
    assert_medication(
        c,
        sk,
        kid,
        "origin-a",
        patient,
        &AssertMedicationInput {
            term,
            coding,
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

/// Assert one uncoded active medication on `patient`; returns its thread id. Same shape as
/// `medication_read.rs`'s helper of the same name (kept file-local: two suites, one line of
/// difference each, is below the bar for `common/`).
async fn assert_one(c: &mut Client, sk: &SigningKey, kid: &str, patient: Uuid, term: &str) -> Uuid {
    assert_drug(c, sk, kid, patient, term, None).await
}

/// Two ids in ascending order. The golden names a pair's threads by their ORDER rather than
/// by which was minted first, so the literal cannot depend on two `now_v7()` calls in the
/// same millisecond happening to sort the way they were minted.
fn lo_hi(x: Uuid, y: Uuid) -> (Uuid, Uuid) {
    (x.min(y), x.max(y))
}

/// Replace, anywhere in `v`, every string that `roles` names by its role — so a list read
/// over ids minted fresh on every run can be compared against ONE literal. Object keys are
/// renamed too (`separation_targets` is keyed by group id).
fn name_roles(v: &mut Value, roles: &HashMap<String, String>) {
    match v {
        Value::String(s) => {
            if let Some(role) = roles.get(s.as_str()) {
                *s = role.clone();
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|i| name_roles(i, roles)),
        Value::Object(map) => {
            let entries: Vec<(String, Value)> = std::mem::take(map).into_iter().collect();
            for (key, mut value) in entries {
                name_roles(&mut value, roles);
                map.insert(roles.get(&key).cloned().unwrap_or(key), value);
            }
        }
        _ => {}
    }
}

/// Strip the fields Task 3 of R1 ADDED (`charts`, each row's `source_charts`, each member's
/// `patient_id`). The golden pins everything that existed BEFORE the combined read, so the
/// new fields — whose values the combined read is entitled to derive differently — are not
/// part of it; they are pinned by the tests that are about them.
fn strip_new_fields(list: &mut Value) {
    list.as_object_mut().unwrap().remove("charts");
    for row in list["rows"].as_array_mut().unwrap() {
        row.as_object_mut().unwrap().remove("source_charts");
        for member in row["members"].as_array_mut().unwrap() {
            member.as_object_mut().unwrap().remove("patient_id");
        }
    }
}

/// THE GOLDEN (ADR-0076 R1, plan constraint "a single never-linked chart reads exactly as
/// before"). Captured against the read as it stood BEFORE the combined read replaced it
/// (the post-Task-3 tree, controller ruling R2), and required to pass unchanged after.
///
/// One never-linked chart holding one of each thing the read distinguishes: an attested
/// active drug (a Fresh vouch), a ceased drug (the past view), a reconciled same-chart pair
/// (one row, two members — through the real reconcile orchestrator, which the local door
/// permits within one chart), a coded drug (sorted under its coded display name), and an
/// un-reconciled duplicate pair (both flagged). Every advisory flag appears at least once
/// false, and `reconciliation_flagged` once true — the query behind it is one this slice
/// replaces. Ids are minted per run, so each is replaced by its role name before the compare.
#[tokio::test]
async fn a_never_linked_chart_reads_exactly_as_before() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE patient_link, person_member")
        .await
        .unwrap();
    let (sk, kid, hsk, hkid) = setup(&c).await;
    let p = chart(&c, &sk, &kid).await;

    let active = assert_one(&mut c, &sk, &kid, p, "amlodipine").await;
    let params = AttestParams {
        human_sk: &hsk,
        human_kid: &hkid,
        basis: None,
        note: None,
    };
    attest_medication_thread(&mut c, &sk, "origin-a", &params, p, active)
        .await
        .unwrap();

    let ceased = assert_one(&mut c, &sk, &kid, p, "penicillin").await;
    cease_medication(
        &mut c,
        &sk,
        &kid,
        "origin-a",
        p,
        ceased,
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

    let x = assert_one(&mut c, &sk, &kid, p, "metformin").await;
    let y = assert_one(&mut c, &sk, &kid, p, "Metformin XR").await;
    reconcile_medications(
        &mut c,
        &sk,
        &kid,
        "origin-a",
        p,
        x,
        y,
        &ReconcileInput {
            provenance: "clinician-judgment",
            reason: None,
        },
        None,
        None,
    )
    .await
    .unwrap();
    let (pair_lo, pair_hi) = lo_hi(x, y);
    // The group takes its display term from the member whose id IS the group id (db/033's
    // `medication_group_display` tiebreak), so the golden's "metformin" depends on `x`
    // sorting first. `now_v7` is monotonic within a process; say so rather than rely on it.
    assert_eq!(pair_lo, x, "the first-minted thread sorts first");

    const MOIETY_ATORVASTATIN: &str = "0f8c4b1e-1b7a-5c2d-9a3e-2b6f7c8d9e01";
    let coded = assert_drug(
        &mut c,
        &sk,
        &kid,
        p,
        "atorvastatin 20",
        Some(SubstanceCoding {
            system: "drugref-moiety",
            code: MOIETY_ATORVASTATIN,
            display: "Lipitor",
        }),
    )
    .await;

    let d1 = assert_one(&mut c, &sk, &kid, p, "aspirin").await;
    let d2 = assert_one(&mut c, &sk, &kid, p, "aspirin").await;
    let (dup_lo, dup_hi) = lo_hi(d1, d2);

    let roles: HashMap<String, String> = [
        (p.to_string(), "<chart>"),
        (active.to_string(), "<active>"),
        (ceased.to_string(), "<ceased>"),
        (pair_lo.to_string(), "<pair-lo>"),
        (pair_hi.to_string(), "<pair-hi>"),
        (coded.to_string(), "<coded>"),
        (dup_lo.to_string(), "<dup-lo>"),
        (dup_hi.to_string(), "<dup-hi>"),
        (hkid.clone(), "<human-kid>"),
    ]
    .into_iter()
    .map(|(k, v)| (k, v.to_string()))
    .collect();

    let mut got = serde_json::to_value(list_patient_medications(&c, p).await.unwrap()).unwrap();
    strip_new_fields(&mut got);
    name_roles(&mut got, &roles);

    // Captured 2026-09-27 from the single-chart read (post-Task-3 tree, before the combined
    // read replaced it) — see the doc comment. Laid out one field-family per line for review;
    // compared as parsed JSON, so key order inside an object is immaterial while ROW order
    // (the display sort) is pinned.
    const GOLDEN: &str = r#"{
  "groups_missing_from_chart": [],
  "separation_targets": {},
  "rows": [
    {
      "group_id": "<coded>", "patient_id": "<chart>", "status": "Active", "term": "atorvastatin 20", "coding_display": "Lipitor",
      "dose_amount": "500", "dose_unit": "mg", "formulation": null, "sig": null, "started_value": null, "started_precision": null,
      "reconciliation_flagged": false, "coding_conflict": false, "cross_patient": false,
      "members": [{"medication_id": "<coded>", "vouch": "Absent"}]
    },
    {
      "group_id": "<active>", "patient_id": "<chart>", "status": "Active", "term": "amlodipine", "coding_display": null,
      "dose_amount": "500", "dose_unit": "mg", "formulation": null, "sig": null, "started_value": null, "started_precision": null,
      "reconciliation_flagged": false, "coding_conflict": false, "cross_patient": false,
      "members": [{"medication_id": "<active>", "vouch": {"Fresh": {"by": "<human-kid>"}}}]
    },
    {
      "group_id": "<dup-lo>", "patient_id": "<chart>", "status": "Active", "term": "aspirin", "coding_display": null,
      "dose_amount": "500", "dose_unit": "mg", "formulation": null, "sig": null, "started_value": null, "started_precision": null,
      "reconciliation_flagged": true, "coding_conflict": false, "cross_patient": false,
      "members": [{"medication_id": "<dup-lo>", "vouch": "Absent"}]
    },
    {
      "group_id": "<dup-hi>", "patient_id": "<chart>", "status": "Active", "term": "aspirin", "coding_display": null,
      "dose_amount": "500", "dose_unit": "mg", "formulation": null, "sig": null, "started_value": null, "started_precision": null,
      "reconciliation_flagged": true, "coding_conflict": false, "cross_patient": false,
      "members": [{"medication_id": "<dup-hi>", "vouch": "Absent"}]
    },
    {
      "group_id": "<pair-lo>", "patient_id": "<chart>", "status": "Active", "term": "metformin", "coding_display": null,
      "dose_amount": "500", "dose_unit": "mg", "formulation": null, "sig": null, "started_value": null, "started_precision": null,
      "reconciliation_flagged": false, "coding_conflict": false, "cross_patient": false,
      "members": [{"medication_id": "<pair-lo>", "vouch": "Absent"}, {"medication_id": "<pair-hi>", "vouch": "Absent"}]
    },
    {
      "group_id": "<ceased>", "patient_id": "<chart>", "status": "Ceased", "term": "penicillin", "coding_display": null,
      "dose_amount": "500", "dose_unit": "mg", "formulation": null, "sig": null, "started_value": null, "started_precision": null,
      "reconciliation_flagged": false, "coding_conflict": false, "cross_patient": false,
      "members": [{"medication_id": "<ceased>", "vouch": "Absent"}]
    }
  ]
}"#;
    let want: Value = serde_json::from_str(GOLDEN).unwrap();
    assert_eq!(
        got,
        want,
        "a never-linked chart must read exactly as it did before the combined read:\n{}",
        serde_json::to_string_pretty(&got).unwrap()
    );
}
