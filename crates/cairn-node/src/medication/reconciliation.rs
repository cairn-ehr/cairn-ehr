//! Reconciliation/separation — link or split TWO `medication_id` threads
//! declared to be the same real drug (§3.16, slice 3; never-merge-always-link).
//! Device-additive; offline-first (no local existence check on either thread).
use cairn_event::medication::{
    reconciliation_body, render_reconciliation_twin, render_separation_twin, separation_body,
    ReconciliationAssertion,
};
use cairn_event::{EventBody, Hlc, SigningKey};
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

/// Assert two medication threads are the same real drug. Device-additive when
/// `attest` is `None` (unchanged 1-arg submit door); offline-first (no local
/// existence check on either thread). Returns the event id. When `attest` is `Some`,
/// the reconciliation AND a human responsibility attestation for BOTH subject threads
/// run in ONE transaction (two attestation HLCs, minted up front, one per thread) — a
/// rejected attestation on either thread rolls the WHOLE reconciliation back. `author`
/// is ADR-0053's separable human-authorship overlay (`None` ⇒ device-additive, the
/// node signs and is the sole `recorded` contributor); it is independent of `attest`
/// (who vouches) — see `AuthorParams`.
#[allow(clippy::too_many_arguments)] // signer + node context + patient/2 subjects/input/author/attest, mirrors dose orchestrators
pub async fn reconcile_medications(
    client: &mut tokio_postgres::Client,
    node_sk: &SigningKey,
    node_kid: &str,
    node_origin: &str,
    patient: Uuid,
    subject_a: Uuid,
    subject_b: Uuid,
    input: &ReconcileInput<'_>,
    author: Option<&crate::medication::AuthorParams<'_>>,
    attest: Option<&crate::medication::AttestParams<'_>>,
) -> anyhow::Result<Uuid> {
    validate_distinct_subjects(subject_a, subject_b)?;
    let verb_hlc = crate::db::next_hlc(client, node_origin).await?;
    let event_id = Uuid::now_v7();
    let body = build_reconcile_body(
        event_id, subject_a, subject_b, patient, input, node_kid, verb_hlc,
    );
    submit_reconcile_like(
        client,
        node_sk,
        node_origin,
        body,
        patient,
        subject_a,
        subject_b,
        author,
        attest,
    )
    .await?;
    Ok(event_id)
}

/// Reverse a reconciliation ("actually two different drugs"). Device-additive when
/// `attest` is `None` (unchanged 1-arg submit door); offline-first. Returns the event
/// id. When `attest` is `Some`, the separation AND a human responsibility attestation
/// for BOTH subject threads run in ONE transaction (same shape as
/// `reconcile_medications`) — a rejected attestation on either thread rolls the WHOLE
/// separation back. `author` is ADR-0053's separable human-authorship overlay (`None`
/// ⇒ device-additive, the node signs and is the sole `recorded` contributor); it is
/// independent of `attest` (who vouches) — see `AuthorParams`.
#[allow(clippy::too_many_arguments)]
pub async fn separate_medications(
    client: &mut tokio_postgres::Client,
    node_sk: &SigningKey,
    node_kid: &str,
    node_origin: &str,
    patient: Uuid,
    subject_a: Uuid,
    subject_b: Uuid,
    input: &ReconcileInput<'_>,
    author: Option<&crate::medication::AuthorParams<'_>>,
    attest: Option<&crate::medication::AttestParams<'_>>,
) -> anyhow::Result<Uuid> {
    validate_distinct_subjects(subject_a, subject_b)?;
    let verb_hlc = crate::db::next_hlc(client, node_origin).await?;
    let event_id = Uuid::now_v7();
    let body = build_separate_body(
        event_id, subject_a, subject_b, patient, input, node_kid, verb_hlc,
    );
    submit_reconcile_like(
        client,
        node_sk,
        node_origin,
        body,
        patient,
        subject_a,
        subject_b,
        author,
        attest,
    )
    .await?;
    Ok(event_id)
}

/// ADR-0052 seal-at-write for the TWO-thread verbs (reconcile / separate). The single-
/// thread `sealed_submit::seal_sign_submit` can't express "vouch for both subjects", so
/// the paired shape lives here — shared by both verbs so the seal-then-sign discipline
/// and the atomic two-attestation txn are written once (house rule 4).
///
/// `attest = None`  → device-additive: seal + sign + one 4-arg strict-door submit
///                     (auto-commit).
/// `attest = Some`  → the reconcile/separate event AND a human responsibility
///                     attestation for BOTH subject threads run in ONE transaction (two
///                     HLCs minted up front); a rejected attestation on either thread
///                     rolls the WHOLE operation back.
///
/// `author` (ADR-0053) is applied in BOTH arms: `None` ⇒ device-additive (the node
/// signs, `recorded`-only); `Some` ⇒ the human authors the content event too — the
/// body is rewritten to an `authored`+`recorded` contributor pair and SIGNED by the
/// human key — while custody stays the NODE's regardless (`ensure_unwrap_key` always
/// registers the node's key, never the author's; born-sealed erasability, ADR-0052).
#[allow(clippy::too_many_arguments)] // signer + node context + body + patient/2 subjects/author/attest
async fn submit_reconcile_like(
    client: &mut tokio_postgres::Client,
    node_sk: &SigningKey,
    node_origin: &str,
    body: EventBody,
    patient: Uuid,
    subject_a: Uuid,
    subject_b: Uuid,
    author: Option<&crate::medication::AuthorParams<'_>>,
    attest: Option<&crate::medication::AttestParams<'_>>,
) -> anyhow::Result<()> {
    match attest {
        None => {
            crate::medication::sealed_submit::seal_sign_submit(client, node_sk, body, author, None)
                .await?;
        }
        Some(params) => {
            // Two attestation HLCs (one per subject thread), minted up front.
            let hlc_a = crate::db::next_hlc(client, node_origin).await?;
            let hlc_b = crate::db::next_hlc(client, node_origin).await?;
            // ADR-0053: the human authors the content event too — rewrite + sign with
            // the author key when present; the node still holds custody + signs the
            // attestations (attest_thread_in_tx below is unchanged). Shared with the
            // single-thread door via `apply_author` so the two can never drift.
            let (body, signing_sk) =
                crate::medication::sealed_submit::apply_author(body, author, node_sk);
            let (signed_bytes, dek) =
                crate::medication::sealed_submit::seal_and_sign(body, signing_sk)?;
            // The door needs the NODE's unwrap key registered before it can wrap this
            // event's DEK into custody (idempotent; committed ahead of the txn) — the
            // node keeps custody regardless of who signed.
            crate::medication::sealed_submit::ensure_unwrap_key(client, node_sk).await?;
            let tx = client.transaction().await?;
            tx.execute(
                "SELECT submit_event($1, NULL, NULL, $2)",
                &[&signed_bytes, &dek.as_slice()],
            )
            .await?;
            crate::medication::attest_thread_in_tx(&tx, params, patient, subject_a, hlc_a).await?;
            crate::medication::attest_thread_in_tx(&tx, params, patient, subject_b, hlc_b).await?;
            tx.commit().await?;
        }
    }
    Ok(())
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
