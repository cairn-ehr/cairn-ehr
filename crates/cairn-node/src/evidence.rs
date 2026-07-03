//! §5.4 clinician-observed evidence authoring. Composes the pure `cairn-event::evidence`
//! builders into `demographic.field.asserted` events and submits them on an EXISTING
//! patient chart (a John Doe, or any poorly-documented chart) through the reused
//! `submit_event` door. Additive events, low ceremony — the sole contributor is the
//! recording actor with role `recorded`, so no attestation is demanded (mirrors
//! `john_doe::build_callsign_name_body`).
//!
//! Split: pure body assembly (unit-tested, no DB) + the async `assert_observed_evidence`
//! orchestrator (one transaction, so age + sex land atomically).

use cairn_event::demographics::{render_administrative_sex_twin, render_dob_twin};
use cairn_event::evidence::{
    birth_year_range_from_age, estimated_dob_body, format_year_range, observed_sex_body,
    CLINICIAN_OBSERVED_PROVENANCE, YEAR_RANGE_PRECISION,
};
use cairn_event::{sign, EventBody, Hlc, SigningKey};
use tokio_postgres::Client;
use uuid::Uuid;

/// schema_version for a demographic field assertion (mirrors `john_doe.rs`).
const DEMOGRAPHIC_FIELD_SCHEMA_VERSION: &str = "demographic.field/1";

/// A clinician's estimated-age observation: `age_years` ± `tolerance_years`, with a
/// mandatory `basis` (§5.4 "estimated age WITH basis"; principle 4).
pub struct AgeObservation {
    pub age_years: u32,
    pub tolerance_years: u32,
    pub basis: String,
}

/// A clinician's observed-sex observation: an OPEN `value` (apparent/phenotypic) with an
/// optional `basis` (how it was observed).
pub struct SexObservation {
    pub value: String,
    pub basis: Option<String>,
}

/// The evidence to record in one call — either or both kinds. `assert_observed_evidence`
/// errors if both are None (nothing to assert).
pub struct ObservedEvidence {
    pub age: Option<AgeObservation>,
    pub sex: Option<SexObservation>,
}

/// Assemble the estimated-age `dob` `EventBody`. Pure: `event_id`/`hlc`/the resolved
/// year range are supplied so the body is fully testable.
pub fn build_estimated_dob_event(
    event_id: Uuid, patient_id: Uuid, min_year: i32, max_year: i32, basis: &str, kid: &str, hlc: Hlc,
) -> EventBody {
    let value = format_year_range(min_year, max_year);
    EventBody {
        event_id: event_id.to_string(),
        patient_id: patient_id.to_string(),
        event_type: "demographic.field.asserted".into(),
        schema_version: DEMOGRAPHIC_FIELD_SCHEMA_VERSION.into(),
        hlc,
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: estimated_dob_body(min_year, max_year, basis, CLINICIAN_OBSERVED_PROVENANCE),
        attachments: vec![],
        plaintext_twin: Some(render_dob_twin(&value, YEAR_RANGE_PRECISION, CLINICIAN_OBSERVED_PROVENANCE)),
    }
}

/// Assemble the observed-sex `administrative-sex` `EventBody`. Pure.
pub fn build_observed_sex_event(
    event_id: Uuid, patient_id: Uuid, value: &str, basis: Option<&str>, kid: &str, hlc: Hlc,
) -> EventBody {
    EventBody {
        event_id: event_id.to_string(),
        patient_id: patient_id.to_string(),
        event_type: "demographic.field.asserted".into(),
        schema_version: DEMOGRAPHIC_FIELD_SCHEMA_VERSION.into(),
        hlc,
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: observed_sex_body(value, basis, CLINICIAN_OBSERVED_PROVENANCE),
        attachments: vec![],
        plaintext_twin: Some(render_administrative_sex_twin(value, CLINICIAN_OBSERVED_PROVENANCE)),
    }
}

/// Author the supplied clinician-observed evidence on `patient_id` in ONE transaction.
/// `observed_year` is the year the estimate was made (the caller owns the clock). Ticks
/// the HLC once per event (age before sex when both present). Errors if `ev` carries
/// neither kind.
pub async fn assert_observed_evidence(
    client: &mut Client,
    sk: &SigningKey,
    kid: &str,
    node_origin: &str,
    patient_id: Uuid,
    ev: &ObservedEvidence,
    observed_year: i32,
) -> anyhow::Result<()> {
    if ev.age.is_none() && ev.sex.is_none() {
        anyhow::bail!("assert_observed_evidence: supply at least one of age or sex");
    }
    // Build + sign each event OUTSIDE the txn (HLC ticks self-commit and may gap safely).
    let mut signed = Vec::new();
    if let Some(a) = &ev.age {
        let (lo, hi) = birth_year_range_from_age(a.age_years, a.tolerance_years, observed_year);
        let h = crate::db::next_hlc(client, node_origin).await?;
        signed.push(sign(&build_estimated_dob_event(Uuid::now_v7(), patient_id, lo, hi, &a.basis, kid, h), sk)?);
    }
    if let Some(s) = &ev.sex {
        let h = crate::db::next_hlc(client, node_origin).await?;
        signed.push(sign(&build_observed_sex_event(Uuid::now_v7(), patient_id, &s.value, s.basis.as_deref(), kid, h), sk)?);
    }
    let tx = client.transaction().await?;
    for s in &signed {
        tx.execute("SELECT submit_event($1)", &[&s.signed_bytes]).await?;
    }
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pid() -> Uuid { Uuid::parse_str("00000000-0000-0000-0000-0000000000ab").unwrap() }
    fn eid() -> Uuid { Uuid::parse_str("22222222-0000-0000-0000-000000000000").unwrap() }
    fn hlc() -> Hlc { Hlc { wall: 5, counter: 0, node_origin: "n".into() } }

    #[test]
    fn estimated_dob_event_is_a_clinician_observed_year_range_dob_with_twin() {
        let body = build_estimated_dob_event(eid(), pid(), 1981, 1991,
            "apparent age ~40±5: dentition", "kid", hlc());
        assert_eq!(body.event_type, "demographic.field.asserted");
        assert_eq!(body.patient_id, pid().to_string());
        assert_eq!(body.payload["field"], "dob");
        assert_eq!(body.payload["value"], "1981/1991");
        assert_eq!(body.payload["facets"]["precision"], "year-range");
        assert_eq!(body.payload["provenance"], "clinician-observed");
        assert_eq!(body.contributors[0]["role"], "recorded");
        assert!(body.contributors[0].get("responsibility").is_none(), "additive: no attestation");
        assert!(!body.plaintext_twin.as_deref().unwrap().trim().is_empty());
    }

    #[test]
    fn observed_sex_event_is_clinician_observed_administrative_sex() {
        let body = build_observed_sex_event(eid(), pid(), "male", Some("external genitalia"), "kid", hlc());
        assert_eq!(body.payload["field"], "administrative-sex");
        assert_eq!(body.payload["value"], "male");
        assert_eq!(body.payload["facets"]["basis"], "external genitalia");
        assert_eq!(body.payload["provenance"], "clinician-observed");
        assert!(!body.plaintext_twin.as_deref().unwrap().trim().is_empty());
    }
}
