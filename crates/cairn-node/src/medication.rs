//! §3.15 medication recording — the node authoring surface (the first clinical
//! content written on this node). Device-additive: signed by the node/clinician
//! key with a `recorded` contributor and NO responsibility attestation in slice 1
//! (mirrors identify.rs). Two orchestrators — assert (mints a thread) and cease
//! (references it). Offline-first: cease does NOT require the assert to be present
//! locally (a cessation may legitimately be authored before its assert replicates).
use cairn_event::medication::{
    medication_assertion_body, medication_cessation_body, render_medication_cessation_twin,
    render_medication_twin, MedicationAssertion, MedicationCessation,
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
