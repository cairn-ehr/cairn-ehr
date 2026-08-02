//! Whole-list medication sign-off — the record-layer half of #288.
//!
//! ADR-0049 attestation is per THREAD, so vouching for a chart means authoring one
//! attestation per thread that needs one. That is N cryptographic acts, but it must be
//! ONE human act: `attest_thread_in_tx` takes an already-unsealed key by reference, so one
//! unseal and one transaction cover all N. This module is what turns that permission into
//! a callable verb — and it lives in the node, not the UI, so the CLI has the same gesture
//! and the reference UI uses no privileged path (ADR-0021).
//!
//! The bundling shape (mint the HLCs, open one transaction, attest each thread, commit) is
//! the one `reconciliation.rs` already uses for exactly two threads, generalised to N.
use crate::medication::{read::list_patient_medications, AttestParams};
use cairn_medication_view::sign_off_targets;
use uuid::Uuid;

/// What one sign-off gesture did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignOffOutcome {
    /// The thread ids that were vouched, in the order they were attested.
    pub attested: Vec<Uuid>,
    /// The attestation event ids, positionally matching `attested`.
    pub event_ids: Vec<Uuid>,
}

/// Attest every thread on this patient's chart whose vouch is absent or stale, in one
/// transaction.
///
/// # Why the target set is read twice
///
/// HLCs must be minted BEFORE the transaction opens: `node_hlc_tick()` advances node state,
/// and minting inside a transaction that later aborts would roll the tick back. But the
/// attestations must be computed against the same snapshot they are written in. So the
/// list is read once outside the transaction (to size the HLC mint) and once inside it (to
/// decide what to sign), and the two must agree.
///
/// If they do not — a medication arrived, or someone else signed a thread, in the
/// milliseconds between — the gesture is REFUSED rather than silently adjusted. That is
/// the clinically correct answer: the clinician vouched for the list they were looking at,
/// and signing a different list on their behalf would be exactly the silent substitution
/// the "never silently refresh on screen" rule exists to prevent. The caller refreshes and
/// the clinician signs again.
pub async fn sign_off_medication_list(
    client: &mut tokio_postgres::Client,
    node_sk: &cairn_event::SigningKey,
    node_origin: &str,
    params: &AttestParams<'_>,
    patient: Uuid,
) -> anyhow::Result<SignOffOutcome> {
    // The node holds custody of every sealed body it writes, attestations included
    // (ADR-0052). Idempotent, and committed ahead of the transaction.
    crate::medication::sealed_submit::ensure_unwrap_key(client, node_sk).await?;

    let expected = sign_off_targets(&list_patient_medications(&*client, patient).await?);
    if expected.is_empty() {
        // Nothing to vouch for. NOT an error: an empty chart is a legitimate state. That
        // "no regular medications, reviewed" cannot itself be recorded is a real gap,
        // tracked as issue #331 — the caller renders the gesture as unavailable.
        return Ok(SignOffOutcome { attested: vec![], event_ids: vec![] });
    }

    // One HLC per attestation, minted up front and consumed in target order (which
    // `sign_off_targets` sorts, so the assignment is deterministic).
    let mut hlcs = Vec::with_capacity(expected.len());
    for _ in 0..expected.len() {
        hlcs.push(crate::db::next_hlc(client, node_origin).await?);
    }

    let tx = client.transaction().await?;
    let actual = sign_off_targets(&list_patient_medications(&tx, patient).await?);
    if actual != expected {
        anyhow::bail!(
            "the medication list changed while it was being signed ({} thread(s) when read, \
             {} in the signing transaction); nothing was signed — refresh the list and sign \
             again so the vouch covers what was actually reviewed",
            expected.len(),
            actual.len()
        );
    }

    let mut event_ids = Vec::with_capacity(actual.len());
    for (thread, hlc) in actual.iter().zip(hlcs) {
        event_ids.push(
            crate::medication::attest_thread_in_tx(&tx, params, patient, *thread, hlc).await?,
        );
    }
    tx.commit().await?;

    Ok(SignOffOutcome { attested: actual, event_ids })
}
