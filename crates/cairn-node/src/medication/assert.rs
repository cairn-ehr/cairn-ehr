//! Medication assertion — mints the immortal `medication_id` thread (§3.15).
//! Device-additive: signed by the node/clinician key with a `recorded`
//! contributor and NO responsibility attestation (mirrors identify.rs).
use cairn_event::medication::{
    medication_assertion_body, render_medication_twin, MedicationAssertion,
};
use cairn_event::{EventBody, Hlc, SigningKey};
use uuid::Uuid;

const MEDICATION_SCHEMA_VERSION: &str = "clinical.medication/1";

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
/// `medication_id`. Device-additive when `attest` is `None` (goes through the 1-arg
/// submit door, auto-commit — unchanged from the pre-attestation path). When `attest`
/// is `Some`, the assert AND the human's responsibility attestation for the
/// newly-minted thread run in ONE transaction, so the attestation's commitment sees
/// the assert event just submitted (mirrors `identify_patient`'s identify+link atomic
/// shape). A rejected attestation rolls the assert back with it.
#[allow(clippy::too_many_arguments)] // signer + node context + patient/input/author/attest, mirrors dose orchestrators
pub async fn assert_medication(
    client: &mut tokio_postgres::Client,
    node_sk: &SigningKey,
    node_kid: &str,
    node_origin: &str,
    patient: Uuid,
    input: &AssertMedicationInput<'_>,
    author: Option<&crate::medication::AuthorParams<'_>>,
    attest: Option<&crate::medication::AttestParams<'_>>,
) -> anyhow::Result<Uuid> {
    validate_term(input.term)?;
    // Mint HLCs up front (self-committing; a rolled-back submit just leaves a gap —
    // the HLC is monotonic and gaps are allowed, exactly like identify_patient).
    let verb_hlc = crate::db::next_hlc(client, node_origin).await?;
    let event_id = Uuid::now_v7();
    let medication_id = Uuid::now_v7();
    let body = build_assert_body(event_id, medication_id, patient, input, node_kid, verb_hlc);
    // ADR-0052 seal-at-write: the clear body is sealed, signed, and submitted through the
    // ONE strict door by seal_sign_submit — which also runs the atomic author-time
    // attestation when `attest` is Some (it vouches for the thread named in the body's
    // payload.medication_id), and rewrites the body to human-signed authorship when
    // `author` is Some (#204 / ADR-0053; the node still keeps DEK custody). We return the
    // THREAD id, not the content event id.
    crate::medication::sealed_submit::seal_sign_submit(client, node_sk, body, author, attest)
        .await?;
    Ok(medication_id)
}
