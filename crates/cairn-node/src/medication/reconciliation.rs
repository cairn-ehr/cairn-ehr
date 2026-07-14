//! Reconciliation/separation — link or split TWO `medication_id` threads
//! declared to be the same real drug (§3.16, slice 3; never-merge-always-link).
//! Device-additive; offline-first (no local existence check on either thread).
use cairn_event::medication::{
    reconciliation_body, render_reconciliation_twin, render_separation_twin, separation_body,
    ReconciliationAssertion,
};
use cairn_event::{sign, EventBody, Hlc, SigningKey};
use uuid::Uuid;

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
