//! Medication withhold and release (ADR-0081) — the paper chart's "W". A withhold puts a
//! time-bounded hold on an existing `medication_id` thread ("do not give this until X, or
//! until reviewed"); a release ends named holds early. Both reference the thread and are
//! offline-first like cessation: no local-presence check here. The in-DB floor (db/059)
//! is the real enforcement — it refuses a malformed span at both doors and, at the local
//! door only, a withhold on a ceased line or a release of something that is not a hold on
//! this line.
use cairn_event::medication::{
    medication_withhold_body, medication_withhold_release_body,
    render_medication_withhold_release_twin, render_medication_withhold_twin, HoldUntil,
    MedicationWithhold, MedicationWithholdRelease,
};
use cairn_event::{EventBody, Hlc, SigningKey};
use uuid::Uuid;

const MEDICATION_WITHHOLD_SCHEMA_VERSION: &str = "clinical.medication-withhold/1";
const MEDICATION_WITHHOLD_RELEASE_SCHEMA_VERSION: &str = "clinical.medication-withhold-release/1";

/// The clinician-supplied fields of a withhold. `from` absent means "from now"; `until` is
/// required — an instant, or `HoldUntil::Reviewed` for the explicit open end.
pub struct WithholdMedicationInput<'a> {
    pub reason: &'a str,
    pub from: Option<&'a str>,
    pub until: HoldUntil<'a>,
}

/// Wrap a payload + twin in the device-shaped EventBody every medication verb uses (the
/// node signs, sole `recorded` contributor; ADR-0053 authorship is layered later by
/// `seal_sign_submit`). `t_effective` stays absent: a hold's own start lives in the
/// payload's `from`, which may be in the future where the envelope cannot go.
#[allow(clippy::too_many_arguments)] // the envelope's fields, one per argument
fn device_body(
    event_id: Uuid,
    patient: Uuid,
    event_type: &str,
    schema_version: &str,
    payload: serde_json::Value,
    twin: String,
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
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    }
}

/// Assemble the signed-to-be `clinical.medication-withhold.asserted` EventBody.
pub fn build_withhold_body(
    event_id: Uuid,
    medication_id: Uuid,
    patient: Uuid,
    input: &WithholdMedicationInput<'_>,
    node_kid: &str,
    hlc: Hlc,
) -> EventBody {
    let mid = medication_id.to_string();
    let w = MedicationWithhold {
        medication_id: &mid,
        reason: input.reason,
        from: input.from,
        until: match input.until {
            HoldUntil::At(t) => HoldUntil::At(t),
            HoldUntil::Reviewed => HoldUntil::Reviewed,
        },
    };
    device_body(
        event_id,
        patient,
        "clinical.medication-withhold.asserted",
        MEDICATION_WITHHOLD_SCHEMA_VERSION,
        medication_withhold_body(&w),
        render_medication_withhold_twin(&w),
        node_kid,
        hlc,
    )
}

/// Assemble the `clinical.medication-withhold-release.asserted` EventBody. `withholds` are
/// the event ids of the holds it ends — only holds its author saw (ADR-0081 decision 4).
pub fn build_release_body(
    event_id: Uuid,
    medication_id: Uuid,
    patient: Uuid,
    withholds: &[Uuid],
    reason: Option<&str>,
    node_kid: &str,
    hlc: Hlc,
) -> EventBody {
    let mid = medication_id.to_string();
    let releases: Vec<String> = withholds.iter().map(Uuid::to_string).collect();
    let r = MedicationWithholdRelease {
        medication_id: &mid,
        releases: &releases,
        reason,
    };
    device_body(
        event_id,
        patient,
        "clinical.medication-withhold-release.asserted",
        MEDICATION_WITHHOLD_RELEASE_SCHEMA_VERSION,
        medication_withhold_release_body(&r),
        render_medication_withhold_release_twin(&r),
        node_kid,
        hlc,
    )
}

/// Withhold a medication thread — write a "W". Returns the withhold's event id, which is
/// what a later release names. `author` is ADR-0053's human-authorship overlay (`None` ⇒
/// the node signs alone). Sealed, signed and submitted through the one strict door.
#[allow(clippy::too_many_arguments)] // signer + node context + patient/thread/input/author, mirrors cease_medication
pub async fn withhold_medication(
    client: &mut tokio_postgres::Client,
    node_sk: &SigningKey,
    node_kid: &str,
    node_origin: &str,
    patient: Uuid,
    medication_id: Uuid,
    input: &WithholdMedicationInput<'_>,
    author: Option<&crate::medication::AuthorParams<'_>>,
) -> anyhow::Result<Uuid> {
    let hlc = crate::db::next_hlc(client, node_origin).await?;
    let event_id = Uuid::now_v7();
    let body = build_withhold_body(event_id, medication_id, patient, input, node_kid, hlc);
    crate::medication::sealed_submit::seal_sign_submit(client, node_sk, body, author, None).await?;
    Ok(event_id)
}

/// End the named holds on a thread early. Returns the release's event id.
#[allow(clippy::too_many_arguments)] // signer + node context + patient/thread/holds/reason/author
pub async fn release_medication_holds(
    client: &mut tokio_postgres::Client,
    node_sk: &SigningKey,
    node_kid: &str,
    node_origin: &str,
    patient: Uuid,
    medication_id: Uuid,
    withholds: &[Uuid],
    reason: Option<&str>,
    author: Option<&crate::medication::AuthorParams<'_>>,
) -> anyhow::Result<Uuid> {
    let hlc = crate::db::next_hlc(client, node_origin).await?;
    let event_id = Uuid::now_v7();
    let body = build_release_body(
        event_id,
        medication_id,
        patient,
        withholds,
        reason,
        node_kid,
        hlc,
    );
    crate::medication::sealed_submit::seal_sign_submit(client, node_sk, body, author, None).await?;
    Ok(event_id)
}
