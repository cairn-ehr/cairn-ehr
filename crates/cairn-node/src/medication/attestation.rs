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
use cairn_event::{event_address, sign_attestation, EventBody, Hlc, SigningKey};
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
        // The ADR-0051 wire shape: responsibility is an object naming held_by (the
        // spec §3.9 {held_by, on_behalf_of?} form); held_by must be the entry's own
        // actor AND the verified attester (the db/005 #195 binding chain).
        contributors: serde_json::json!([
            {"actor_id": human_kid, "role": "attested",
             "responsibility": {"held_by": human_kid}}
        ]),
        payload: medication_attestation_body(&a),
        attachments: vec![],
        plaintext_twin: Some(render_medication_attestation_twin(&a)),
    }
}

/// Single-source SQL for a thread's current content-event commitment (hex) + count —
/// the DRY point behind both `thread_commitment` (post-hoc, runs on a plain `&Client`)
/// and `attest_thread_in_tx` (author-time, runs on an open `&Transaction` so it reads
/// through the SAME txn/snapshot as a content event the caller just submitted). Bound
/// on `GenericClient` (impl'd for both `Client` and `Transaction` in tokio-postgres
/// 0.7.18) rather than duplicated per call site — a future edit to the content-event-
/// type list now touches exactly one place. `+ Sync` is required because
/// `GenericClient` is `#[async_trait]`: its returned future captures `&self`, which
/// must be `Sync` for the future itself to be `Send`.
async fn thread_commitment_on(
    client: &(impl tokio_postgres::GenericClient + Sync),
    medication_id: Uuid,
) -> anyhow::Result<Option<(String, u32)>> {
    let row = client
        .query_one(
            // Both numbers come from the db/034 single-source SQL fns — the commitment
            // AND the count that sizes reviewed_count. The count previously inlined the
            // four content-event types here as a hand copy of the SQL filter (#212 drift
            // pair 2: slice 5 adds a verb to db/034 and every later attestation carries a
            // permanently wrong SIGNED reviewed_count); calling
            // cairn_medication_thread_readable_count deletes the copy AND makes
            // reviewed_count definitionally the same measure the ADR-0049 false-fresh
            // staleness gate later compares it against.
            // CAUTION (ADR-0049 false-fresh, Tasks 8/9): both fns read through
            // cairn_clear_payload, so they are CUSTODY-dependent, not event-set-dependent
            // — see db/034's caution. Here the count only sizes the vouch, not yet unsafe.
            "SELECT encode(cairn_medication_thread_commitment($1::text::uuid), 'hex') AS c, \
             cairn_medication_thread_readable_count($1::text::uuid) AS n",
            &[&medication_id.to_string()],
        )
        .await?;
    let commitment: Option<String> = row.get("c");
    let count: i64 = row.get("n");
    Ok(commitment.map(|c| (c, count as u32)))
}

/// Read the thread's current set commitment (hex) + content-event count from the
/// single-source SQL fn. `None` when the thread has no LOCAL content events (orphan) —
/// the caller then refuses to author a meaningless vouch. No caller inside this crate
/// needs this standalone `&Client` entry point right now (both production call sites —
/// post-hoc `attest_medication_thread` and the author-time verb orchestrators — go
/// through `attest_thread_in_tx`'s txn-scoped read instead); kept `pub` as the
/// reusable Client-side entry point for any future caller that wants the commitment
/// without opening a transaction (e.g. a "preview what you'd be vouching for" CLI
/// command, read-only, ahead of the actual attest call).
pub async fn thread_commitment(
    client: &tokio_postgres::Client,
    medication_id: Uuid,
) -> anyhow::Result<Option<(String, u32)>> {
    thread_commitment_on(client, medication_id).await
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
    // thread_commitment_on is generic over GenericClient (Client OR Transaction) —
    // called here bound to `tx` so the read runs through the SAME txn/snapshot as any
    // content event the caller just submitted (the atomic author-time shape depends
    // on this: it's what lets Task 6's verb orchestrators see their own just-submitted
    // event before the human's vouch is computed).
    let (commitment, count) = thread_commitment_on(tx, medication_id)
        .await?
        .ok_or_else(|| {
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
        count,
        params.basis,
        params.note,
        params.human_kid,
        hlc,
    );
    // ADR-0052: the attestation event is clinical.* and must be born-sealed too. Seal +
    // sign via the shared helper (seal-then-sign, so the content_address the token binds
    // to covers the ciphertext and survives a shred), then submit through the 4-arg
    // strict door carrying the human token AND the DEK. The node's unwrap key is already
    // registered by the content-authoring path that precedes every attestation — the
    // verb's own submit (author-time), or the earlier authoring of the thread a post-hoc
    // vouch reviews — so no ensure_unwrap_key is needed here.
    let (signed_bytes, dek) =
        crate::medication::sealed_submit::seal_and_sign(body, params.human_sk)?;
    let ca = event_address(&signed_bytes);
    let token = sign_attestation(&ca, params.human_kid, "attested", params.human_sk)?;
    let attester_vk = params.human_sk.verifying_key().to_bytes().to_vec();
    tx.execute(
        "SELECT submit_event($1, $2, $3, $4)",
        &[&signed_bytes, &token, &attester_vk, &dek.as_slice()],
    )
    .await?;
    Ok(event_id)
}

/// Post-hoc standalone sign-off: mint an HLC, open a one-statement txn, attest, commit.
///
/// `node_sk` is the NODE's own signing key — used ONLY to register the node's DEK-unwrap
/// key defensively before the sealed attestation submits (ADR-0052). The author-time verb
/// paths always register it via the content submit, but a post-hoc vouch on a thread this
/// node acquired by SYNC (Tasks 8/9, when synced threads gain custody) may reach the door
/// with no unwrap key registered yet, and the attestation's custody wrap would then fail
/// with "node unwrap key not registered". Registration MUST be the node key, never the
/// human attester (`params.human_sk`): the node holds custody, and the node_unwrap_key
/// singleton refuses a second, different key (`cairn_register_unwrap_key`). Idempotent, so
/// calling it when the content path already registered the same key is a safe no-op.
///
/// SAFETY FOLLOW-UP (Tasks 8/9): this only makes the CUSTODY key present; it does NOT fix
/// the ADR-0049 false-fresh hazard a partial-custody node can still hit at the staleness
/// view (see cairn_medication_thread_commitment in db/034). That gate is Tasks 8/9's job.
pub async fn attest_medication_thread(
    client: &mut tokio_postgres::Client,
    node_sk: &SigningKey,
    node_origin: &str,
    params: &AttestParams<'_>,
    patient: Uuid,
    medication_id: Uuid,
) -> anyhow::Result<Uuid> {
    crate::medication::sealed_submit::ensure_unwrap_key(client, node_sk).await?;
    let hlc = crate::db::next_hlc(client, node_origin).await?;
    let tx = client.transaction().await?;
    let id = attest_thread_in_tx(&tx, params, patient, medication_id, hlc).await?;
    tx.commit().await?;
    Ok(id)
}
