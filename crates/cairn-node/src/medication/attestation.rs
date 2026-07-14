//! Slice-4 human-attested clinical responsibility on a medication thread. Device-
//! signed medication events stay unchanged; this authors a SEPARATE
//! `clinical.medication-attestation.asserted` event carrying the responsible human as
//! a responsibility-bearing contributor -> the db/005 gate demands a valid human
//! attestation token (the 3-arg door). A sign-off pins a convergent commitment of the
//! thread's content-event set (via `cairn_medication_thread_commitment`, db/034) so a
//! later change flips the vouch stale. Reused by the post-hoc CLI and the author-time
//! `--attest-as` path (attestation.rs is the single owner of the attestation seam).
use cairn_event::medication::{
    medication_attestation_body, render_medication_attestation_twin, MedicationAttestation,
};
use cairn_event::{event_address, sign, sign_attestation, EventBody, Hlc, SigningKey};
use uuid::Uuid;

const ATTESTATION_SCHEMA_VERSION: &str = "clinical.medication-attestation/1";

/// The human who takes responsibility, plus optional context. Threaded explicitly so
/// the author paths stay pure functions of their arguments. The human key both signs
/// AND attests the attestation event (the `identify --link` precedent).
pub struct AttestParams<'a> {
    pub human_sk: &'a SigningKey,
    pub human_kid: &'a str,
    pub basis: Option<&'a str>,
    pub note: Option<&'a str>,
}

/// Assemble the signed `clinical.medication-attestation.asserted` `EventBody`. Pure.
/// The sole contributor is the vouching human with a `responsibility` marker — this is
/// what makes submit_event demand a valid human attestation token (db/005 gate).
#[allow(clippy::too_many_arguments)] // event id + thread + patient + pin + count + basis/note + human + hlc
pub fn build_attestation_body(
    event_id: Uuid,
    medication_id: Uuid,
    patient: Uuid,
    reviewed_commitment: &str,
    reviewed_count: u32,
    basis: Option<&str>,
    note: Option<&str>,
    human_kid: &str,
    hlc: Hlc,
) -> EventBody {
    let mid = medication_id.to_string();
    let a = MedicationAttestation {
        medication_id: &mid,
        reviewed_commitment,
        reviewed_count,
        basis,
        note,
    };
    EventBody {
        event_id: event_id.to_string(),
        patient_id: patient.to_string(),
        event_type: "clinical.medication-attestation.asserted".into(),
        schema_version: ATTESTATION_SCHEMA_VERSION.into(),
        hlc,
        t_effective: None,
        signer_key_id: human_kid.into(),
        contributors: serde_json::json!([
            {"actor_id": human_kid, "role": "attested", "responsibility": "attested"}
        ]),
        payload: medication_attestation_body(&a),
        attachments: vec![],
        plaintext_twin: Some(render_medication_attestation_twin(&a)),
    }
}

/// Read the thread's current set commitment (hex) + content-event count from the
/// single-source SQL fn. `None` when the thread has no LOCAL content events (orphan) —
/// the caller then refuses to author a meaningless vouch.
pub async fn thread_commitment(
    client: &tokio_postgres::Client,
    medication_id: Uuid,
) -> anyhow::Result<Option<(String, u32)>> {
    let row = client
        .query_one(
            "SELECT encode(cairn_medication_thread_commitment($1::text::uuid), 'hex') AS c, \
             (SELECT count(*) FROM event_log \
                WHERE event_type IN ('clinical.medication.asserted', \
                    'clinical.medication-cessation.asserted', \
                    'clinical.medication-dose-change.asserted', \
                    'clinical.medication-dose-correction.asserted') \
                  AND (body ->> 'medication_id')::uuid = $1::text::uuid) AS n",
            &[&medication_id.to_string()],
        )
        .await?;
    let commitment: Option<String> = row.get("c");
    let count: i64 = row.get("n");
    Ok(commitment.map(|c| (c, count as u32)))
}

/// Author one attestation for `medication_id` INSIDE the caller's transaction (so it
/// sees any content event the caller just submitted in the same txn). Computes the
/// commitment, signs with the human key, mints the token, submits the 3-arg door.
/// Returns the attestation event id. Errors (rolling the caller's txn back) if the
/// thread has no local content events or the db/005 gate refuses.
pub async fn attest_thread_in_tx(
    tx: &tokio_postgres::Transaction<'_>,
    params: &AttestParams<'_>,
    patient: Uuid,
    medication_id: Uuid,
    hlc: Hlc,
) -> anyhow::Result<Uuid> {
    // thread_commitment takes &Client; a Transaction derefs to a GenericClient — run
    // the same query directly on the tx to keep one txn/snapshot.
    let row = tx
        .query_one(
            "SELECT encode(cairn_medication_thread_commitment($1::text::uuid), 'hex') AS c, \
             (SELECT count(*) FROM event_log \
                WHERE event_type IN ('clinical.medication.asserted', \
                    'clinical.medication-cessation.asserted', \
                    'clinical.medication-dose-change.asserted', \
                    'clinical.medication-dose-correction.asserted') \
                  AND (body ->> 'medication_id')::uuid = $1::text::uuid) AS n",
            &[&medication_id.to_string()],
        )
        .await?;
    let commitment: Option<String> = row.get("c");
    let count: i64 = row.get("n");
    let commitment = commitment.ok_or_else(|| {
        anyhow::anyhow!(
            "no local content for medication thread {medication_id}; nothing to vouch for \
             (author or sync the thread first)"
        )
    })?;

    let event_id = Uuid::now_v7();
    let body = build_attestation_body(
        event_id,
        medication_id,
        patient,
        &commitment,
        count as u32,
        params.basis,
        params.note,
        params.human_kid,
        hlc,
    );
    let signed = sign(&body, params.human_sk)?;
    let ca = event_address(&signed.signed_bytes);
    let token = sign_attestation(&ca, params.human_kid, "attested", params.human_sk)?;
    let attester_vk = params.human_sk.verifying_key().to_bytes().to_vec();
    tx.execute(
        "SELECT submit_event($1,$2,$3)",
        &[&signed.signed_bytes, &token, &attester_vk],
    )
    .await?;
    Ok(event_id)
}

/// Post-hoc standalone sign-off: mint an HLC, open a one-statement txn, attest, commit.
pub async fn attest_medication_thread(
    client: &mut tokio_postgres::Client,
    node_origin: &str,
    params: &AttestParams<'_>,
    patient: Uuid,
    medication_id: Uuid,
) -> anyhow::Result<Uuid> {
    let hlc = crate::db::next_hlc(client, node_origin).await?;
    let tx = client.transaction().await?;
    let id = attest_thread_in_tx(&tx, params, patient, medication_id, hlc).await?;
    tx.commit().await?;
    Ok(id)
}
