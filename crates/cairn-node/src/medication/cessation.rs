//! Medication cessation — marks an existing `medication_id` thread "past"
//! (§3.15). References the thread; offline-first (no local-presence check).
use cairn_event::medication::{
    medication_cessation_body, render_medication_cessation_twin, MedicationCessation,
};
use cairn_event::{sign, EventBody, Hlc, SigningKey};
use uuid::Uuid;

const MEDICATION_CESSATION_SCHEMA_VERSION: &str = "clinical.medication-cessation/1";

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
