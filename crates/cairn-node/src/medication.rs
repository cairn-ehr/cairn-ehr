//! §3.15 medication recording — the node authoring surface (the first clinical
//! content written on this node). Device-additive: signed by the node/clinician
//! key with a `recorded` contributor and NO responsibility attestation in slice 1
//! (mirrors identify.rs). Two orchestrators — assert (mints a thread) and cease
//! (references it). Offline-first: cease does NOT require the assert to be present
//! locally (a cessation may legitimately be authored before its assert replicates).
use cairn_event::medication::{
    dose_change_body, dose_correction_body, medication_assertion_body, medication_cessation_body,
    reconciliation_body, render_dose_change_twin, render_dose_correction_twin,
    render_medication_cessation_twin, render_medication_twin, render_reconciliation_twin,
    render_separation_twin, separation_body, DoseChange, DoseCorrection, MedicationAssertion,
    MedicationCessation, ReconciliationAssertion,
};
use cairn_event::{sign, EventBody, Hlc, SigningKey};
use uuid::Uuid;

const MEDICATION_SCHEMA_VERSION: &str = "clinical.medication/1";
const MEDICATION_CESSATION_SCHEMA_VERSION: &str = "clinical.medication-cessation/1";

/// The clinician-supplied fields of a medication statement. `term` is required;
/// everything else is an honest Option (unknown when None).
pub struct AssertMedicationInput<'a> {
    pub term: &'a str,
    pub inn_code: Option<&'a str>,
    pub formulation: Option<&'a str>,
    pub dose_amount: Option<&'a str>,
    pub dose_unit: Option<&'a str>,
    pub sig: Option<&'a str>,
    pub info_source: &'a str,
    pub started: Option<&'a str>,
    pub started_precision: Option<&'a str>,
}

/// Advisory Rust-side guard mirroring the DB floor: refuse a blank term with a
/// clinical message. The DB floor is the real, unbypassable enforcement.
pub fn validate_term(term: &str) -> anyhow::Result<()> {
    if term.trim().is_empty() {
        anyhow::bail!(
            "medication term must not be empty: record WHAT the patient takes (even if vague)"
        );
    }
    Ok(())
}

/// Assemble the signed `clinical.medication.asserted` EventBody. Pure — the caller
/// mints `event_id`/`medication_id`, supplies the HLC, and signs.
pub fn build_assert_body(
    event_id: Uuid,
    medication_id: Uuid,
    patient: Uuid,
    input: &AssertMedicationInput<'_>,
    node_kid: &str,
    hlc: Hlc,
) -> EventBody {
    let mid = medication_id.to_string();
    let a = MedicationAssertion {
        medication_id: &mid,
        term: input.term,
        inn_code: input.inn_code,
        formulation: input.formulation,
        dose_amount: input.dose_amount,
        dose_unit: input.dose_unit,
        sig: input.sig,
        info_source: input.info_source,
        started: input.started,
        started_precision: input.started_precision,
    };
    EventBody {
        event_id: event_id.to_string(),
        patient_id: patient.to_string(),
        event_type: "clinical.medication.asserted".into(),
        schema_version: MEDICATION_SCHEMA_VERSION.into(),
        hlc,
        t_effective: None,
        signer_key_id: node_kid.into(),
        contributors: serde_json::json!([{"actor_id": node_kid, "role": "recorded"}]),
        payload: medication_assertion_body(&a),
        attachments: vec![],
        plaintext_twin: Some(render_medication_twin(&a)),
    }
}

/// Record a medication the patient takes/took. Mints and returns the thread's
/// `medication_id`. Device-additive; goes through the 1-arg submit door.
pub async fn assert_medication(
    client: &tokio_postgres::Client,
    node_sk: &SigningKey,
    node_kid: &str,
    node_origin: &str,
    patient: Uuid,
    input: &AssertMedicationInput<'_>,
) -> anyhow::Result<Uuid> {
    validate_term(input.term)?;
    let hlc = crate::db::next_hlc(client, node_origin).await?;
    let event_id = Uuid::now_v7();
    let medication_id = Uuid::now_v7();
    let body = build_assert_body(event_id, medication_id, patient, input, node_kid, hlc);
    let signed = sign(&body, node_sk)?;
    client
        .execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await?;
    Ok(medication_id)
}

/// The clinician-supplied fields of a cessation. All optional.
pub struct CeaseMedicationInput<'a> {
    pub stopped: Option<&'a str>,
    pub stopped_precision: Option<&'a str>,
    pub reason: Option<&'a str>,
}

/// Assemble the signed `clinical.medication-cessation.asserted` EventBody.
pub fn build_cease_body(
    event_id: Uuid,
    medication_id: Uuid,
    patient: Uuid,
    input: &CeaseMedicationInput<'_>,
    node_kid: &str,
    hlc: Hlc,
) -> EventBody {
    let mid = medication_id.to_string();
    let csn = MedicationCessation {
        medication_id: &mid,
        stopped: input.stopped,
        stopped_precision: input.stopped_precision,
        reason: input.reason,
    };
    EventBody {
        event_id: event_id.to_string(),
        patient_id: patient.to_string(),
        event_type: "clinical.medication-cessation.asserted".into(),
        schema_version: MEDICATION_CESSATION_SCHEMA_VERSION.into(),
        hlc,
        t_effective: None,
        signer_key_id: node_kid.into(),
        contributors: serde_json::json!([{"actor_id": node_kid, "role": "recorded"}]),
        payload: medication_cessation_body(&csn),
        attachments: vec![],
        plaintext_twin: Some(render_medication_cessation_twin(&csn)),
    }
}

/// Cease a medication thread — makes it "past". Offline-first: does NOT check the
/// assert is present locally. Returns the cessation event id.
pub async fn cease_medication(
    client: &tokio_postgres::Client,
    node_sk: &SigningKey,
    node_kid: &str,
    node_origin: &str,
    patient: Uuid,
    medication_id: Uuid,
    input: &CeaseMedicationInput<'_>,
) -> anyhow::Result<Uuid> {
    let hlc = crate::db::next_hlc(client, node_origin).await?;
    let event_id = Uuid::now_v7();
    let body = build_cease_body(event_id, medication_id, patient, input, node_kid, hlc);
    let signed = sign(&body, node_sk)?;
    client
        .execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await?;
    Ok(event_id)
}

const DOSE_CHANGE_SCHEMA_VERSION: &str = "clinical.medication-dose-change/1";
const DOSE_CORRECTION_SCHEMA_VERSION: &str = "clinical.medication-dose-correction/1";

/// Clinician-supplied fields of a dose change. `info_source` required (a new clinical
/// claim); dose fields honest-unknown ("upped it, dunno to what").
pub struct ChangeDoseInput<'a> {
    pub dose_amount: Option<&'a str>,
    pub dose_unit: Option<&'a str>,
    pub effective: Option<&'a str>,
    pub effective_precision: Option<&'a str>,
    pub info_source: &'a str,
    pub reason: Option<&'a str>,
}

/// Assemble the signed `clinical.medication-dose-change.asserted` EventBody. Pure.
pub fn build_dose_change_body(
    event_id: Uuid,
    medication_id: Uuid,
    patient: Uuid,
    input: &ChangeDoseInput<'_>,
    node_kid: &str,
    hlc: Hlc,
) -> EventBody {
    let mid = medication_id.to_string();
    let d = DoseChange {
        medication_id: &mid,
        dose_amount: input.dose_amount,
        dose_unit: input.dose_unit,
        effective: input.effective,
        effective_precision: input.effective_precision,
        info_source: input.info_source,
        reason: input.reason,
    };
    EventBody {
        event_id: event_id.to_string(),
        patient_id: patient.to_string(),
        event_type: "clinical.medication-dose-change.asserted".into(),
        schema_version: DOSE_CHANGE_SCHEMA_VERSION.into(),
        hlc,
        t_effective: None,
        signer_key_id: node_kid.into(),
        contributors: serde_json::json!([{"actor_id": node_kid, "role": "recorded"}]),
        payload: dose_change_body(&d),
        attachments: vec![],
        plaintext_twin: Some(render_dose_change_twin(&d)),
    }
}

/// Record a dose change on an existing thread. Offline-first (no local existence
/// check on the thread). Returns the change event id.
pub async fn change_dose(
    client: &tokio_postgres::Client,
    node_sk: &SigningKey,
    node_kid: &str,
    node_origin: &str,
    patient: Uuid,
    medication_id: Uuid,
    input: &ChangeDoseInput<'_>,
) -> anyhow::Result<Uuid> {
    let hlc = crate::db::next_hlc(client, node_origin).await?;
    let event_id = Uuid::now_v7();
    let body = build_dose_change_body(event_id, medication_id, patient, input, node_kid, hlc);
    let signed = sign(&body, node_sk)?;
    client
        .execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await?;
    Ok(event_id)
}

/// Clinician-supplied fields of a dose correction. All optional (correct-to-unknown).
pub struct CorrectDoseInput<'a> {
    pub dose_amount: Option<&'a str>,
    pub dose_unit: Option<&'a str>,
    pub info_source: Option<&'a str>,
    pub reason: Option<&'a str>,
}

/// Assemble the signed `clinical.medication-dose-correction.asserted` EventBody. Pure.
pub fn build_dose_correction_body(
    event_id: Uuid,
    medication_id: Uuid,
    patient: Uuid,
    corrects: Uuid,
    input: &CorrectDoseInput<'_>,
    node_kid: &str,
    hlc: Hlc,
) -> EventBody {
    let mid = medication_id.to_string();
    let corrects_s = corrects.to_string();
    let d = DoseCorrection {
        medication_id: &mid,
        corrects: &corrects_s,
        dose_amount: input.dose_amount,
        dose_unit: input.dose_unit,
        info_source: input.info_source,
        reason: input.reason,
    };
    EventBody {
        event_id: event_id.to_string(),
        patient_id: patient.to_string(),
        event_type: "clinical.medication-dose-correction.asserted".into(),
        schema_version: DOSE_CORRECTION_SCHEMA_VERSION.into(),
        hlc,
        t_effective: None,
        signer_key_id: node_kid.into(),
        contributors: serde_json::json!([{"actor_id": node_kid, "role": "recorded"}]),
        payload: dose_correction_body(&d),
        attachments: vec![],
        plaintext_twin: Some(render_dose_correction_twin(&d)),
    }
}

/// Correct a wrongly-recorded dose on a targeted dose event. Offline-first (the
/// target need not exist locally). Returns the correction event id.
#[allow(clippy::too_many_arguments)] // signer + node context + patient/thread/target/input, mirrors photo_evidence.rs / identity_evidence.rs / john_doe.rs
pub async fn correct_dose(
    client: &tokio_postgres::Client,
    node_sk: &SigningKey,
    node_kid: &str,
    node_origin: &str,
    patient: Uuid,
    medication_id: Uuid,
    corrects: Uuid,
    input: &CorrectDoseInput<'_>,
) -> anyhow::Result<Uuid> {
    let hlc = crate::db::next_hlc(client, node_origin).await?;
    let event_id = Uuid::now_v7();
    let body = build_dose_correction_body(
        event_id,
        medication_id,
        patient,
        corrects,
        input,
        node_kid,
        hlc,
    );
    let signed = sign(&body, node_sk)?;
    client
        .execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await?;
    Ok(event_id)
}

/// Resolve the dose event a correction should target. If `explicit` is given, use it;
/// otherwise the current (latest-effective) dose point of the thread. Errors if the
/// thread has no local dose timeline (offline-first: pass --target explicitly then).
pub async fn resolve_correction_target(
    client: &tokio_postgres::Client,
    medication_id: Uuid,
    explicit: Option<Uuid>,
) -> anyhow::Result<Uuid> {
    if let Some(t) = explicit {
        return Ok(t);
    }
    let row = client
        .query_opt(
            "SELECT dose_event_id::text FROM medication_current_dose WHERE medication_id = $1::text::uuid",
            &[&medication_id.to_string()],
        )
        .await?;
    match row {
        Some(r) => Ok(r.get::<_, String>(0).parse()?),
        None => anyhow::bail!(
            "no local dose timeline for thread {medication_id}; pass --target <dose_event_id> explicitly"
        ),
    }
}

const RECONCILIATION_SCHEMA_VERSION: &str = "clinical.medication-reconciliation/1";
const SEPARATION_SCHEMA_VERSION: &str = "clinical.medication-separation/1";

/// Clinician-supplied fields of a reconciliation/separation. `provenance` is
/// required by the floor; the CLI defaults it to "clinician-judgment".
pub struct ReconcileInput<'a> {
    pub provenance: &'a str,
    pub reason: Option<&'a str>,
}

/// Advisory Rust guard mirroring the DB floor: refuse a self-reconcile. The DB
/// floor is the real, unbypassable enforcement.
pub fn validate_distinct_subjects(a: Uuid, b: Uuid) -> anyhow::Result<()> {
    if a == b {
        anyhow::bail!("reconciliation subjects must be two DIFFERENT medication threads");
    }
    Ok(())
}

/// Shared body assembler. `event_type` and `schema_version` select the event type;
/// the payload is identical either way (mirrors identity link/unlink).
#[allow(clippy::too_many_arguments)]
fn build_reconcile_like_body(
    event_type: &str,
    schema_version: &str,
    twin: String,
    event_id: Uuid,
    patient: Uuid,
    payload: serde_json::Value,
    node_kid: &str,
    hlc: Hlc,
) -> EventBody {
    EventBody {
        event_id: event_id.to_string(),
        patient_id: patient.to_string(),
        event_type: event_type.into(),
        schema_version: schema_version.into(),
        hlc,
        t_effective: None,
        signer_key_id: node_kid.into(),
        contributors: serde_json::json!([{"actor_id": node_kid, "role": "recorded"}]),
        payload,
        attachments: vec![],
        plaintext_twin: Some(twin),
    }
}

/// Assemble the signed `clinical.medication-reconciliation.asserted` EventBody. Pure.
pub fn build_reconcile_body(
    event_id: Uuid,
    subject_a: Uuid,
    subject_b: Uuid,
    patient: Uuid,
    input: &ReconcileInput<'_>,
    node_kid: &str,
    hlc: Hlc,
) -> EventBody {
    let sa = subject_a.to_string();
    let sb = subject_b.to_string();
    let a = ReconciliationAssertion {
        subject_a: &sa,
        subject_b: &sb,
        provenance: input.provenance,
        reason: input.reason,
    };
    build_reconcile_like_body(
        "clinical.medication-reconciliation.asserted",
        RECONCILIATION_SCHEMA_VERSION,
        render_reconciliation_twin(&a),
        event_id,
        patient,
        reconciliation_body(&a),
        node_kid,
        hlc,
    )
}

/// Assemble the signed `clinical.medication-separation.asserted` EventBody. Pure.
pub fn build_separate_body(
    event_id: Uuid,
    subject_a: Uuid,
    subject_b: Uuid,
    patient: Uuid,
    input: &ReconcileInput<'_>,
    node_kid: &str,
    hlc: Hlc,
) -> EventBody {
    let sa = subject_a.to_string();
    let sb = subject_b.to_string();
    let a = ReconciliationAssertion {
        subject_a: &sa,
        subject_b: &sb,
        provenance: input.provenance,
        reason: input.reason,
    };
    build_reconcile_like_body(
        "clinical.medication-separation.asserted",
        SEPARATION_SCHEMA_VERSION,
        render_separation_twin(&a),
        event_id,
        patient,
        separation_body(&a),
        node_kid,
        hlc,
    )
}

/// Assert two medication threads are the same real drug. Device-additive; offline-
/// first (no local existence check on either thread). Returns the event id.
#[allow(clippy::too_many_arguments)] // signer + node context + patient/2 subjects/input, mirrors dose orchestrators
pub async fn reconcile_medications(
    client: &tokio_postgres::Client,
    node_sk: &SigningKey,
    node_kid: &str,
    node_origin: &str,
    patient: Uuid,
    subject_a: Uuid,
    subject_b: Uuid,
    input: &ReconcileInput<'_>,
) -> anyhow::Result<Uuid> {
    validate_distinct_subjects(subject_a, subject_b)?;
    let hlc = crate::db::next_hlc(client, node_origin).await?;
    let event_id = Uuid::now_v7();
    let body = build_reconcile_body(
        event_id, subject_a, subject_b, patient, input, node_kid, hlc,
    );
    let signed = sign(&body, node_sk)?;
    client
        .execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await?;
    Ok(event_id)
}

/// Reverse a reconciliation ("actually two different drugs"). Device-additive;
/// offline-first. Returns the event id.
#[allow(clippy::too_many_arguments)]
pub async fn separate_medications(
    client: &tokio_postgres::Client,
    node_sk: &SigningKey,
    node_kid: &str,
    node_origin: &str,
    patient: Uuid,
    subject_a: Uuid,
    subject_b: Uuid,
    input: &ReconcileInput<'_>,
) -> anyhow::Result<Uuid> {
    validate_distinct_subjects(subject_a, subject_b)?;
    let hlc = crate::db::next_hlc(client, node_origin).await?;
    let event_id = Uuid::now_v7();
    let body = build_separate_body(
        event_id, subject_a, subject_b, patient, input, node_kid, hlc,
    );
    let signed = sign(&body, node_sk)?;
    client
        .execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await?;
    Ok(event_id)
}

#[cfg(test)]
mod dose_build_tests {
    use super::*;
    use cairn_event::Hlc;

    fn hlc() -> Hlc {
        Hlc {
            wall: 1_700_000_000_000,
            counter: 0,
            node_origin: "test-node".into(),
        }
    }

    #[test]
    fn build_change_sets_type_schema_twin() {
        let input = ChangeDoseInput {
            dose_amount: Some("80"),
            dose_unit: Some("mg"),
            effective: Some("2025-06"),
            effective_precision: Some("month"),
            info_source: "clinician-observed",
            reason: Some("titration"),
        };
        let b = build_dose_change_body(
            Uuid::now_v7(),
            Uuid::now_v7(),
            Uuid::now_v7(),
            &input,
            "kid",
            hlc(),
        );
        assert_eq!(b.event_type, "clinical.medication-dose-change.asserted");
        assert_eq!(b.schema_version, "clinical.medication-dose-change/1");
        assert!(b.plaintext_twin.as_deref().unwrap().contains("80 mg"));
        assert_eq!(b.payload["dose"]["amount"], "80");
        assert_eq!(b.contributors[0]["role"], "recorded");
        assert!(b.t_effective.is_none());
    }

    #[test]
    fn build_correction_sets_type_schema_corrects() {
        let corrects = Uuid::now_v7();
        let input = CorrectDoseInput {
            dose_amount: Some("20"),
            dose_unit: Some("mg"),
            info_source: None,
            reason: Some("mis-keyed"),
        };
        let b = build_dose_correction_body(
            Uuid::now_v7(),
            Uuid::now_v7(),
            Uuid::now_v7(),
            corrects,
            &input,
            "kid",
            hlc(),
        );
        assert_eq!(b.event_type, "clinical.medication-dose-correction.asserted");
        assert_eq!(b.schema_version, "clinical.medication-dose-correction/1");
        assert_eq!(b.payload["corrects"], corrects.to_string());
        assert!(b.plaintext_twin.as_deref().unwrap().contains("20 mg"));
    }
}

#[cfg(test)]
mod reconciliation_build_tests {
    use super::*;
    use cairn_event::Hlc;

    fn hlc() -> Hlc {
        Hlc {
            wall: 1_700_000_000_000,
            counter: 0,
            node_origin: "test-node".into(),
        }
    }

    #[test]
    fn build_reconcile_sets_type_schema_twin() {
        let a = Uuid::now_v7();
        let b = Uuid::now_v7();
        let input = ReconcileInput {
            provenance: "clinician-judgment",
            reason: Some("brand vs generic"),
        };
        let body = build_reconcile_body(Uuid::now_v7(), a, b, Uuid::now_v7(), &input, "kid", hlc());
        assert_eq!(
            body.event_type,
            "clinical.medication-reconciliation.asserted"
        );
        assert_eq!(body.schema_version, "clinical.medication-reconciliation/1");
        assert_eq!(body.payload["subject_a"], a.to_string());
        assert_eq!(body.payload["subject_b"], b.to_string());
        assert_eq!(body.payload["provenance"], "clinician-judgment");
        assert_eq!(body.contributors[0]["role"], "recorded");
        assert!(body.t_effective.is_none());
        assert!(body
            .plaintext_twin
            .as_deref()
            .unwrap()
            .contains("Reconciled"));
    }

    #[test]
    fn build_separate_sets_type_schema_twin() {
        let a = Uuid::now_v7();
        let b = Uuid::now_v7();
        let input = ReconcileInput {
            provenance: "clinician-judgment",
            reason: None,
        };
        let body = build_separate_body(Uuid::now_v7(), a, b, Uuid::now_v7(), &input, "kid", hlc());
        assert_eq!(body.event_type, "clinical.medication-separation.asserted");
        assert_eq!(body.schema_version, "clinical.medication-separation/1");
        assert!(body
            .plaintext_twin
            .as_deref()
            .unwrap()
            .contains("Separated"));
    }

    #[test]
    fn distinct_subjects_guard() {
        let a = Uuid::now_v7();
        assert!(validate_distinct_subjects(a, Uuid::now_v7()).is_ok());
        assert!(validate_distinct_subjects(a, a).is_err());
    }
}
