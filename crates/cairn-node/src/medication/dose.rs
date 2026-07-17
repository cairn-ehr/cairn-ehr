//! Dose change/correction — two more orchestrators over the `medication_id`
//! thread (§3.15, slice 2). Change is a new clinical claim (`info_source`
//! required); correction targets a specific prior dose event and may correct
//! to unknown. Offline-first throughout.
use cairn_event::medication::{
    dose_change_body, dose_correction_body, render_dose_change_twin, render_dose_correction_twin,
    DoseChange, DoseCorrection,
};
use cairn_event::{EventBody, Hlc, SigningKey};
use uuid::Uuid;

const DOSE_CHANGE_SCHEMA_VERSION: &str = "clinical.medication-dose-change/1";
const DOSE_CORRECTION_SCHEMA_VERSION: &str = "clinical.medication-dose-correction/2";

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
/// check on the thread). Returns the change event id. Device-additive when `attest`
/// is `None` (unchanged 1-arg submit door). When `attest` is `Some`, the change AND
/// the human's responsibility attestation for the thread run in ONE transaction (same
/// atomic shape as `assert_medication`); a rejected attestation rolls the change back
/// with it.
#[allow(clippy::too_many_arguments)] // signer + node context + patient/thread/input/attest, mirrors dose orchestrators
pub async fn change_dose(
    client: &mut tokio_postgres::Client,
    node_sk: &SigningKey,
    node_kid: &str,
    node_origin: &str,
    patient: Uuid,
    medication_id: Uuid,
    input: &ChangeDoseInput<'_>,
    attest: Option<&crate::medication::AttestParams<'_>>,
) -> anyhow::Result<Uuid> {
    let verb_hlc = crate::db::next_hlc(client, node_origin).await?;
    let event_id = Uuid::now_v7();
    let body = build_dose_change_body(event_id, medication_id, patient, input, node_kid, verb_hlc);
    // ADR-0052 seal-at-write: seal + sign + submit through the ONE strict door, with the
    // atomic author-time attestation folded in when `attest` is Some (see sealed_submit).
    crate::medication::sealed_submit::seal_sign_submit(client, node_sk, body, attest).await?;
    Ok(event_id)
}

/// Clinician-supplied fields of a dose correction (per-field patch; see ADR-0050).
/// A group is *set* (Some / value), *struck* (named in `strike` → unknown), or *kept*
/// (absent). `reason` = the point's clinical reason; `note` = why this correction exists.
pub struct CorrectDoseInput<'a> {
    pub dose_amount: Option<&'a str>,
    pub dose_unit: Option<&'a str>,
    pub effective: Option<&'a str>,
    pub effective_precision: Option<&'a str>,
    pub reason: Option<&'a str>,
    pub strike: &'a [&'a str],
    pub note: Option<&'a str>,
    pub info_source: Option<&'a str>,
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
        effective: input.effective,
        effective_precision: input.effective_precision,
        reason: input.reason,
        strike: input.strike,
        note: input.note,
        info_source: input.info_source,
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
/// target need not exist locally). Returns the correction event id. Device-additive
/// when `attest` is `None` (unchanged 1-arg submit door). When `attest` is `Some`,
/// the correction AND the human's responsibility attestation for the THREAD
/// (`medication_id`, not the targeted `corrects` event) run in ONE transaction (same
/// atomic shape as `assert_medication`); a rejected attestation rolls the correction
/// back with it.
#[allow(clippy::too_many_arguments)] // signer + node context + patient/thread/target/input/attest, mirrors photo_evidence.rs / identity_evidence.rs / john_doe.rs
pub async fn correct_dose(
    client: &mut tokio_postgres::Client,
    node_sk: &SigningKey,
    node_kid: &str,
    node_origin: &str,
    patient: Uuid,
    medication_id: Uuid,
    corrects: Uuid,
    input: &CorrectDoseInput<'_>,
    attest: Option<&crate::medication::AttestParams<'_>>,
) -> anyhow::Result<Uuid> {
    let verb_hlc = crate::db::next_hlc(client, node_origin).await?;
    let event_id = Uuid::now_v7();
    let body = build_dose_correction_body(
        event_id,
        medication_id,
        patient,
        corrects,
        input,
        node_kid,
        verb_hlc,
    );
    // ADR-0052 seal-at-write: seal + sign + submit through the ONE strict door. The
    // attestation (when Some) vouches for the THREAD (`medication_id`), not the targeted
    // `corrects` event — seal_sign_submit reads the thread from payload.medication_id.
    crate::medication::sealed_submit::seal_sign_submit(client, node_sk, body, attest).await?;
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
            effective: Some("2024-01"),
            effective_precision: Some("month"),
            reason: Some("titration"),
            strike: &[],
            note: Some("mis-keyed"),
            info_source: None,
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
        assert_eq!(b.schema_version, "clinical.medication-dose-correction/2");
        assert_eq!(b.payload["corrects"], corrects.to_string());
        assert!(b.plaintext_twin.as_deref().unwrap().contains("20 mg"));
        assert_eq!(b.payload["effective"]["value"], "2024-01");
        assert_eq!(b.payload["reason"], "titration");
    }
}
