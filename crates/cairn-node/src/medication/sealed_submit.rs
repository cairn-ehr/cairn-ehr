//! ADR-0052 seal-at-write: the ONE path every clinical verb submits through.
//!
//! WHY THIS EXISTS: every clinical body must be born sealed so the ADR-0005
//! erasure ladder stays reachable forever (erasability, not confidentiality —
//! the node holds custody by default). The pure verb builders keep producing a
//! CLEAR `EventBody` (payload + `Some(clear twin)`) so they stay testable in
//! cairn-event; this module seals that body under a fresh per-event DEK, signs
//! the SEALED form (seal-then-sign: the signature covers ciphertext and survives
//! a shred), registers the node's unwrap key, and hands the DEK to the strict
//! door for its floor checks, custody wrap, and the operational clear view.
//!
//! For a junior reader: think of it as "encrypt the clinical content, sign the
//! envelope, then knock on the one write door with the key the door needs to
//! peek inside and stash a recoverable copy of that key". The verb never touches
//! `submit_event` directly any more — it hands its clear body here.

use anyhow::Context;
use cairn_event::seal::{seal_event_payload, seal_stub_twin};
use cairn_event::{sign, EventBody, SigningKey};

use crate::keystore;

/// The human who AUTHORS a clinical event (#204 / ADR-0053) — the FIRST half of
/// principle 10, the mirror of `AttestParams` (attestation.rs, the second half). The
/// human's key SIGNS the sealed content event (`session ≠ author`); the node still
/// holds custody. Threaded explicitly so the verb paths stay pure functions of their
/// arguments. `None` ⇒ device-additive (the node signs, `recorded`-only), unchanged.
pub struct AuthorParams<'a> {
    pub human_sk: &'a SigningKey,
    pub human_kid: &'a str,
}

/// Apply an optional human author to a clear clinical body and pick the key that must
/// SIGN it — the ONE place ADR-0053's authoring rewrite happens (house rule 4).
///
/// `Some` ⇒ the body is rewritten to `{human, authored} + {node, recorded}` and the
/// HUMAN's key signs; `None` ⇒ the body is untouched and the NODE signs (device-additive,
/// unchanged). Custody is NOT decided here and never follows the signature: every caller
/// still registers the node's unwrap key (born-sealed erasability, ADR-0052).
///
/// WHY A HELPER: the single-thread door (`seal_sign_submit`) and the two-thread
/// reconcile/separate path both need this pair, and `with_human_author` is NOT idempotent
/// — calling it twice prepends a SECOND `authored` contributor. Funnelling both callers
/// through one function is what guarantees it is applied exactly once per body, and that
/// the rewrite and the signing-key choice can never drift apart.
pub fn apply_author<'a>(
    body: EventBody,
    author: Option<&'a AuthorParams<'a>>,
    node_sk: &'a SigningKey,
) -> (EventBody, &'a SigningKey) {
    match author {
        Some(a) => (
            cairn_event::contributor::with_human_author(body, a.human_kid),
            a.human_sk,
        ),
        None => (body, node_sk),
    }
}

/// Register this node's X25519 public unwrap key so the strict door can wrap every
/// sealed event's DEK into recoverable custody. Idempotent: `cairn_register_unwrap_key`
/// is a no-op once the same key is present (and refuses a *different* key — rotation is
/// a separate ceremony, ADR-0052). Derives the public half from the node's Ed25519 seed
/// via the domain-separated HKDF in cairn-event::seal; the secret half never leaves the
/// daemon, so a DB backup (public half only) can never unwrap a DEK.
pub async fn ensure_unwrap_key(
    client: &tokio_postgres::Client,
    sk: &SigningKey,
) -> anyhow::Result<()> {
    let secret = keystore::unwrap_secret(sk);
    let public = cairn_event::seal::unwrap_public(&secret);
    client
        .execute(
            "SELECT cairn_register_unwrap_key($1)",
            &[&public.as_slice()],
        )
        .await?;
    Ok(())
}

/// Seal a CLEAR clinical body and sign the sealed form. This is the single place
/// seal-then-sign happens: the clear payload + clear twin go under a fresh per-event
/// DEK, the outer `plaintext_twin` becomes the mechanical stub (principle 11 — the row
/// stays self-describing as WHAT it is), and the signature is computed over the
/// resulting ciphertext container. Returns the signed wire bytes and the DEK the strict
/// door needs as its 4th argument.
///
/// Pure (no I/O): both `seal_sign_submit` and the two-thread reconciliation/separation
/// verbs build on it so the seal-then-sign discipline lives in exactly one place.
pub fn seal_and_sign(
    mut body: EventBody,
    sk: &SigningKey,
) -> anyhow::Result<(Vec<u8>, zeroize::Zeroizing<[u8; 32]>)> {
    // The clear twin travels INSIDE the sealed region under the same DEK as its body
    // (ADR-0052 / #92 (a)); the OUTER twin is the mechanical stub. Take() the clear twin
    // out of the body before we overwrite plaintext_twin with the stub.
    let clear_twin = body.plaintext_twin.take().ok_or_else(|| {
        anyhow::anyhow!(
            "seal_and_sign: a clear clinical body must carry its plaintext twin to seal"
        )
    })?;
    let (container, dek) = seal_event_payload(&body.payload, &clear_twin, &body.event_id)?;
    body.payload = container;
    body.plaintext_twin = Some(seal_stub_twin(&body.event_type));
    let signed = sign(&body, sk)?;
    Ok((signed.signed_bytes, dek))
}

/// The thread a single-thread attested verb vouches for lives in `payload.medication_id`
/// (the immortal thread key, distinct from the event's own id). Read it out of the CLEAR
/// body before it is consumed by the seal.
fn thread_id_of(body: &EventBody) -> anyhow::Result<uuid::Uuid> {
    body.payload
        .get("medication_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "seal_sign_submit: an attested verb body must name its thread in payload.medication_id"
            )
        })?
        .parse()
        .context("seal_sign_submit: payload.medication_id is not a valid uuid")
}

/// Seal, sign, register the unwrap key, and submit a CLEAR clinical body through the
/// strict door — the single-thread path shared by assert / cessation / dose-change /
/// dose-correction (and any future single-thread clinical verb).
///
/// `attest = None` → device-additive: one 4-arg `submit_event` call (auto-commit), no
/// attestation. Unchanged clinical semantics, now sealed.
///
/// `attest = Some` → the verb's submit AND the human's responsibility attestation for the
/// thread the body names run in ONE transaction (the atomic author-time shape
/// `assert_medication` used before ADR-0052), so the attestation's commitment sees the
/// content event just submitted. A rejected attestation rolls the whole verb back with it.
///
/// Returns the content event's id (`body.event_id`). The routing facts (event id,
/// patient, node origin, and — only when attesting — the thread) are read out of the
/// CLEAR body before it is consumed by the seal, so the verb call sites keep the tidy
/// `(client, node_sk, body, author, attest)` shape.
pub async fn seal_sign_submit(
    client: &mut tokio_postgres::Client,
    node_sk: &SigningKey,
    body: EventBody,
    author: Option<&AuthorParams<'_>>,
    attest: Option<&super::AttestParams<'_>>,
) -> anyhow::Result<uuid::Uuid> {
    // ADR-0053: when a human authors, rewrite the device-shaped body so the human is
    // an `authored` contributor AND the signer; the node stays `recorded` + custodian.
    let (body, signing_sk) = apply_author(body, author, node_sk);

    let event_id: uuid::Uuid = body.event_id.parse().with_context(|| {
        format!(
            "seal_sign_submit: event_id {:?} is not a uuid",
            body.event_id
        )
    })?;
    let patient: uuid::Uuid = body.patient_id.parse().with_context(|| {
        format!(
            "seal_sign_submit: patient_id {:?} is not a uuid",
            body.patient_id
        )
    })?;
    let node_origin = body.hlc.node_origin.clone();
    // Only the attested path needs the thread; extract it before consuming the body.
    let thread = match attest {
        Some(_) => Some(thread_id_of(&body)?),
        None => None,
    };

    let (signed_bytes, dek) = seal_and_sign(body, signing_sk)?;
    // Custody is the NODE's regardless of who signed (born-sealed erasability, ADR-0052).
    // Register the node's unwrap key first (idempotent, node-scoped): the door needs it
    // committed and visible so it can wrap this event's DEK into recoverable custody.
    ensure_unwrap_key(client, node_sk).await?;

    match attest {
        None => {
            client
                .execute(
                    "SELECT submit_event($1, NULL, NULL, $2)",
                    &[&signed_bytes, &dek.as_slice()],
                )
                .await?;
        }
        Some(params) => {
            let thread = thread.expect("thread id is extracted whenever attest is Some");
            let attest_hlc = crate::db::next_hlc(client, &node_origin).await?;
            let tx = client.transaction().await?;
            tx.execute(
                "SELECT submit_event($1, NULL, NULL, $2)",
                &[&signed_bytes, &dek.as_slice()],
            )
            .await?;
            super::attest_thread_in_tx(&tx, params, patient, thread, attest_hlc).await?;
            tx.commit().await?;
        }
    }
    Ok(event_id)
}
