//! §5.4 photo identity-evidence author path. Splits into pure helpers (unit-tested here) and
//! an async orchestrator (`assert_photo_evidence`, e2e-tested in tests/photo_evidence.rs):
//!   * `prepare_local_blob` — pure: content address + bao outboard + the "original" Rendition.
//!   * `build_photo_evidence_body` — pure: assemble the signed EventBody (payload + the
//!     top-level attachment + the authored twin).
//!   * `assert_photo_evidence` — store the bytes present (through the db/026 verify floor) and
//!     author the event in ONE transaction, so bytes + reference land atomically.

use cairn_event::attachment::{Attachment, Rendition, RENDITION_ROLE_ORIGINAL};
use cairn_event::identity_evidence::{
    photo_evidence_body, render_identity_evidence_twin, IDENTITY_EVIDENCE_EVENT_TYPE,
    IDENTITY_EVIDENCE_SCHEMA_VERSION, PHOTO_EVIDENCE_KIND,
};
use cairn_event::{blob_address, blob_outboard, sign, EventBody, Hlc, SigningKey};
use tokio_postgres::Client;
use uuid::Uuid;

/// A locally-prepared blob ready to INSERT present=TRUE plus the Rendition that references it.
pub struct LocalBlob {
    pub addr: Vec<u8>,      // multihash BLAKE3 content address (blob_store PK)
    pub outboard: Vec<u8>,  // bao verified-streaming tree (serves slices; stored with content)
    pub rendition: Rendition,
}

/// PURE: compute the content address, bao outboard, and the by-reference "original"
/// Rendition for `bytes`. No DB, so it is unit-testable; the orchestrator does the INSERT.
pub fn prepare_local_blob(bytes: &[u8], media_type: &str) -> LocalBlob {
    LocalBlob {
        addr: blob_address(bytes),
        outboard: blob_outboard(bytes),
        rendition: Rendition::reference(RENDITION_ROLE_ORIGINAL, bytes, media_type),
    }
}

/// PURE: enforce the honest-descriptor requirement (§5.4 / principle 4) — a photo attachment
/// must say what it shows, so an empty or whitespace-only descriptor is refused. This lives in
/// the library (not only the CLI edge) so EVERY caller — a future UI backend authoring directly
/// against this function included — inherits the guarantee, not just the one CLI subcommand.
pub fn validate_photo_descriptor(descriptor: &str) -> anyhow::Result<()> {
    if descriptor.trim().is_empty() {
        anyhow::bail!("photo descriptor must be non-empty (§5.4/principle 4: say what the photo shows)");
    }
    Ok(())
}

/// PURE: assemble the signed `EventBody` for a photo identity-evidence event. The photo
/// (`attachment`) rides the top-level `EventBody.attachments` (ADR-0042); the payload carries
/// the clinical framing (kind/provenance/basis); the twin is authored from the descriptor.
/// The sole contributor is the recording actor with role `recorded` — additive, no attestation.
pub fn build_photo_evidence_body(
    event_id: Uuid,
    patient_id: Uuid,
    kid: &str,
    hlc: Hlc,
    basis: Option<&str>,
    attachment: Attachment,
) -> EventBody {
    let twin = render_identity_evidence_twin(PHOTO_EVIDENCE_KIND, basis, &attachment);
    EventBody {
        event_id: event_id.to_string(),
        patient_id: patient_id.to_string(),
        event_type: IDENTITY_EVIDENCE_EVENT_TYPE.into(),
        schema_version: IDENTITY_EVIDENCE_SCHEMA_VERSION.into(),
        hlc,
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: photo_evidence_body(basis),
        attachments: vec![attachment],
        plaintext_twin: Some(twin),
    }
}

/// Store `bytes` as a present, self-verified local blob and author the identity-evidence
/// event referencing it, in ONE transaction (bytes + reference land atomically — a chart
/// never references a blob whose bytes failed to store). The blob INSERT passes present=TRUE
/// through the db/026 verify trigger (the first non-hostile writer through that floor); the
/// floor's own `blob_note_reference` for our rendition is a harmless ON CONFLICT no-op. The
/// caller (CLI edge) owns file I/O; this takes bytes so it stays testable. Returns the event id.
#[allow(clippy::too_many_arguments)] // signer + node context + the six blob/photo/basis inputs
pub async fn assert_photo_evidence(
    client: &mut Client,
    sk: &SigningKey,
    kid: &str,
    node_origin: &str,
    patient_id: Uuid,
    bytes: &[u8],
    media_type: &str,
    descriptor: &str,
    basis: Option<&str>,
) -> anyhow::Result<Uuid> {
    // Honest-descriptor floor for every caller (not only the CLI): refuse before any DB work.
    validate_photo_descriptor(descriptor)?;
    let lb = prepare_local_blob(bytes, media_type);
    let attachment = Attachment::single(descriptor, lb.rendition.clone());

    // Tick the HLC once (self-committing, like john_doe.rs) before the transaction.
    let hlc = crate::db::next_hlc(client, node_origin).await?;
    let event_id = Uuid::now_v7();
    let body = build_photo_evidence_body(event_id, patient_id, kid, hlc, basis, attachment);
    let signed = sign(&body, sk)?;
    let byte_len = bytes.len() as i64;

    let tx = client.transaction().await?;
    // Store the bytes present=TRUE — verified in-DB by the db/026 trigger before it commits.
    // DO UPDATE, not DO NOTHING: a row may already exist at this content address as a
    // present=FALSE reference-only placeholder (e.g. a remote-synced event referenced this
    // photo before this node held its bytes, or blob_note_reference created it). DO NOTHING
    // would leave that placeholder unfilled while the event still commits, so the chart would
    // reference a blob whose bytes were silently discarded — a content-address match guarantees
    // identical bytes, so overwriting (even a present=TRUE row) is always safe.
    tx.execute(
        "INSERT INTO blob_store (blob_address, media_type, byte_len, content, outboard, present, fetched_at)
         VALUES ($1, $2, $3, $4, $5, TRUE, clock_timestamp())
         ON CONFLICT (blob_address) DO UPDATE
            SET content = EXCLUDED.content, outboard = EXCLUDED.outboard, present = TRUE,
                media_type = EXCLUDED.media_type, byte_len = EXCLUDED.byte_len,
                fetched_at = EXCLUDED.fetched_at",
        &[&lb.addr, &media_type, &byte_len, &bytes, &lb.outboard],
    ).await?;
    // Author the event (its floor learns the reference — ON CONFLICT no-op against the row above).
    tx.execute("SELECT submit_event($1)", &[&signed.signed_bytes]).await?;
    tx.commit().await?;

    Ok(event_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hlc() -> Hlc { Hlc { wall: 7, counter: 0, node_origin: "n".into() } }

    #[test]
    fn descriptor_validation_refuses_empty_and_whitespace_only() {
        // The honest-descriptor floor is enforced in the library, so a caller bypassing the
        // CLI (e.g. a UI backend) still cannot author a §5.4 photo with no description.
        assert!(validate_photo_descriptor("").is_err(), "empty descriptor refused");
        assert!(validate_photo_descriptor("   \t\n").is_err(), "whitespace-only refused");
        assert!(validate_photo_descriptor("frontal face photograph").is_ok(), "real descriptor accepted");
    }

    #[test]
    fn prepare_local_blob_addresses_and_renders_the_original_rendition() {
        let lb = prepare_local_blob(b"jpegbytes", "image/jpeg");
        assert_eq!(lb.addr, blob_address(b"jpegbytes"));
        assert_eq!(lb.rendition.role, "original");
        assert_eq!(lb.rendition.digest_hex, hex::encode(blob_address(b"jpegbytes")));
        assert_eq!(lb.rendition.byte_len, 9);
        assert!(!lb.outboard.is_empty());
    }

    #[test]
    fn body_is_the_identity_evidence_event_with_the_photo_in_top_level_attachments() {
        let pid = Uuid::now_v7();
        let eid = Uuid::now_v7();
        let att = Attachment::single(
            "frontal face photograph",
            prepare_local_blob(b"x", "image/jpeg").rendition,
        );
        let body = build_photo_evidence_body(eid, pid, "kid", hlc(), Some("on arrival"), att.clone());

        assert_eq!(body.event_type, IDENTITY_EVIDENCE_EVENT_TYPE);
        assert_eq!(body.schema_version, IDENTITY_EVIDENCE_SCHEMA_VERSION);
        assert_eq!(body.patient_id, pid.to_string());
        assert_eq!(body.payload["kind"], PHOTO_EVIDENCE_KIND);
        assert_eq!(body.payload["basis"], "on arrival");
        assert_eq!(body.attachments, vec![att.clone()], "photo rides top-level attachments");
        // additive event → recorded role, no attestation demanded
        assert_eq!(body.contributors[0]["role"], "recorded");
        // authored twin, legible, no pixel bytes
        let twin = body.plaintext_twin.as_deref().unwrap();
        assert!(twin.contains("frontal face photograph"));
        assert_eq!(twin, &render_identity_evidence_twin("photo", Some("on arrival"), &att));
    }
}
