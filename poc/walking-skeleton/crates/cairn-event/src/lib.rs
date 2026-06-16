//! Cairn walking skeleton — the signed event envelope (Spike 0001 §4).
//!
//! This crate is the safety-critical core, kept small and reviewable per the §9
//! blast-radius rule. It encodes the three structural moves the spike validates:
//!
//!   1. **Sign the bytes; never re-serialize.** [`sign`] produces `signed_bytes`
//!      — a COSE_Sign1 (RFC 9052) wire blob whose payload is the canonical-CBOR
//!      body. That blob is stored verbatim; [`verify_with`] checks the signature
//!      over those exact bytes. Nothing ever round-trips the structure back to
//!      bytes for verification.
//!   2. **Self-describing, algorithm-tagged.** [`event_address`] and
//!      [`blob_address`] are multihashes (sha2-256 = 0x12, BLAKE3 = 0x1e), so the
//!      algorithm travels with the digest and the choice is migratable.
//!   3. **Re-attestation is overlay.** Not exercised here, but the COSE `alg`
//!      header is what lets a future event re-sign an old one under a stronger
//!      primitive as an ordinary overlay event.

use ed25519_dalek::{Signer, Verifier};
use serde::{Deserialize, Serialize};

// Re-exported so downstream crates (cairn-sync) need not depend on ed25519-dalek
// directly — the keypair type travels with this crate's signing API.
pub use ed25519_dalek::{SigningKey, VerifyingKey};

pub const SHA2_256_MULTIHASH_PREFIX: [u8; 2] = [0x12, 0x20]; // sha2-256, 32 bytes
pub const BLAKE3_MULTIHASH_PREFIX: [u8; 2] = [0x1e, 0x20]; // blake3, 32 bytes

#[derive(Debug, thiserror::Error)]
pub enum EventError {
    #[error("CBOR encode/decode: {0}")]
    Cbor(String),
    #[error("COSE: {0}")]
    Cose(String),
    #[error("signature verification failed")]
    BadSignature,
    #[error("malformed key id (expected 32-byte Ed25519 public key)")]
    BadKeyId,
    #[error("missing COSE payload")]
    NoPayload,
    #[error("entropy: {0}")]
    Entropy(String),
}

/// Hybrid Logical Clock stamp — the objective `t_recorded` ceiling (§3.6).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Hlc {
    pub wall: i64,
    pub counter: i32,
    pub node_origin: String,
}

/// A §3.14 attachment reference: eager (it rides in the signed event) while the
/// bytes are lazy (the §6.6 byte tier). `digest_hex` is the BLAKE3 multihash.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AttachmentRef {
    pub alg: String, // "blake3"
    pub digest_hex: String,
    pub media_type: String,
    pub descriptor: String,
    pub byte_len: i64,
}

/// The canonical event body — the thing that is CBOR-encoded and signed. Field
/// order here IS the canonical encoding order (structural move 1): one writer,
/// one serialization; verifiers byte-compare and never re-encode.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EventBody {
    pub event_id: String,      // UUIDv7
    pub patient_id: String,    // immortal subject UUID
    pub event_type: String,    // patient.created | patient.amended | note.added
    pub schema_version: String,
    pub hlc: Hlc,
    pub t_effective: Option<String>, // asserted effective time (ISO-8601); None = unknown
    pub signer_key_id: String,       // hex(Ed25519 public key) — see note on the registry below
    pub contributors: serde_json::Value, // §3.9 contributor set (skeleton: a single author)
    pub payload: serde_json::Value,      // clinical/demographic content; becomes the DB `body`
    #[serde(default)]
    pub attachments: Vec<AttachmentRef>,
}

/// A signed event ready to enter `event_log`: the verbatim signed bytes plus
/// their self-describing content address.
#[derive(Debug, Clone)]
pub struct SignedEvent {
    pub signed_bytes: Vec<u8>,
    pub content_address: Vec<u8>,
}

/// Generate a fresh Ed25519 keypair. The skeleton's `signer_key_id` is the hex
/// of the public key, so an event is self-describing for verification.
///
/// NOTE: trusting the key embedded in the event is a *skeleton* shortcut. In
/// production the `signer_key_id` is resolved against the enrolled actor registry
/// (ADR-0011): origin is proven by signature, but *which* keys are trusted is a
/// registry decision, not a property of the event asserting its own key.
pub fn generate_key() -> Result<(SigningKey, String), EventError> {
    let mut seed = [0u8; 32];
    getrandom::getrandom(&mut seed).map_err(|e| EventError::Entropy(e.to_string()))?;
    let sk = SigningKey::from_bytes(&seed);
    let kid = hex::encode(sk.verifying_key().to_bytes());
    Ok((sk, kid))
}

/// Deterministic CBOR encoding of the body — the COSE payload (structural move 1).
pub fn canonical_cbor(body: &EventBody) -> Result<Vec<u8>, EventError> {
    let mut buf = Vec::new();
    ciborium::into_writer(body, &mut buf).map_err(|e| EventError::Cbor(e.to_string()))?;
    Ok(buf)
}

/// Multihash(sha2-256) of the signed bytes — the event's content address (move 2).
pub fn event_address(signed_bytes: &[u8]) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    let mut out = SHA2_256_MULTIHASH_PREFIX.to_vec();
    out.extend_from_slice(&Sha256::digest(signed_bytes));
    out
}

/// Multihash(BLAKE3) of a blob's bytes — its content address (§4.4). BLAKE3's
/// tree structure is what makes chunked, resumable, swarm fetch self-verifying.
pub fn blob_address(bytes: &[u8]) -> Vec<u8> {
    let mut out = BLAKE3_MULTIHASH_PREFIX.to_vec();
    out.extend_from_slice(blake3::hash(bytes).as_bytes());
    out
}

/// Sign a body into `signed_bytes` (COSE_Sign1, Ed25519) plus its content address.
pub fn sign(body: &EventBody, signing_key: &SigningKey) -> Result<SignedEvent, EventError> {
    use coset::{iana, CborSerializable, CoseSign1Builder, HeaderBuilder};

    let payload = canonical_cbor(body)?;
    let kid = signing_key.verifying_key().to_bytes().to_vec();
    let protected = HeaderBuilder::new()
        .algorithm(iana::Algorithm::EdDSA)
        .key_id(kid)
        .build();

    let sign1 = CoseSign1Builder::new()
        .protected(protected)
        .payload(payload)
        .create_signature(b"", |tbs| signing_key.sign(tbs).to_bytes().to_vec())
        .build();

    let signed_bytes = sign1.to_vec().map_err(|e| EventError::Cose(e.to_string()))?;
    let content_address = event_address(&signed_bytes);
    Ok(SignedEvent {
        signed_bytes,
        content_address,
    })
}

/// Read the COSE key id (the claimed Ed25519 public key) without verifying.
pub fn key_id(signed_bytes: &[u8]) -> Result<Vec<u8>, EventError> {
    use coset::{CborSerializable, CoseSign1};
    let sign1 = CoseSign1::from_slice(signed_bytes).map_err(|e| EventError::Cose(e.to_string()))?;
    Ok(sign1.protected.header.key_id)
}

/// Verify `signed_bytes` against a known key and decode the body. This is the
/// safety-critical seam (§9 / ADR-0002) that moves into an in-DB pgrx gate in
/// production so no unverified row can ever enter the log.
pub fn verify_with(signed_bytes: &[u8], vk: &VerifyingKey) -> Result<EventBody, EventError> {
    use coset::{CborSerializable, CoseSign1};
    let sign1 = CoseSign1::from_slice(signed_bytes).map_err(|e| EventError::Cose(e.to_string()))?;
    sign1
        .verify_signature(b"", |sig, tbs| {
            let signature =
                ed25519_dalek::Signature::from_slice(sig).map_err(|_| EventError::BadSignature)?;
            vk.verify(tbs, &signature).map_err(|_| EventError::BadSignature)
        })
        .map_err(|_| EventError::BadSignature)?;
    let payload = sign1.payload.ok_or(EventError::NoPayload)?;
    ciborium::from_reader(&payload[..]).map_err(|e| EventError::Cbor(e.to_string()))
}

/// Verify using the self-described key id (skeleton convenience — see the note on
/// [`generate_key`]; the registry replaces "trust the embedded key" in production).
pub fn verify_self_described(signed_bytes: &[u8]) -> Result<EventBody, EventError> {
    let kid = key_id(signed_bytes)?;
    let bytes: [u8; 32] = kid.try_into().map_err(|_| EventError::BadKeyId)?;
    let vk = VerifyingKey::from_bytes(&bytes).map_err(|_| EventError::BadKeyId)?;
    verify_with(signed_bytes, &vk)
}

/// Mechanically derive the §3.13 plaintext legibility twin from a body. Crude on
/// purpose: the twin must be derivable by *any* node from the structured content,
/// so a node generations behind can still read the event as prose.
pub fn plaintext_twin(body: &EventBody) -> String {
    let when = body.t_effective.as_deref().unwrap_or("(time unknown)");
    let content = serde_json::to_string_pretty(&body.payload).unwrap_or_default();
    format!(
        "[{}] {} for patient {} (recorded {}:{} @ {}; effective {})\n{}",
        body.event_type,
        body.schema_version,
        body.patient_id,
        body.hlc.wall,
        body.hlc.counter,
        body.hlc.node_origin,
        when,
        content,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> EventBody {
        EventBody {
            event_id: uuid::Uuid::now_v7().to_string(),
            patient_id: uuid::Uuid::now_v7().to_string(),
            event_type: "patient.created".into(),
            schema_version: "patient/1".into(),
            hlc: Hlc {
                wall: 1_700_000_000_000,
                counter: 0,
                node_origin: "cape-york".into(),
            },
            t_effective: Some("2026-06-16T00:00:00Z".into()),
            signer_key_id: String::new(),
            contributors: json!([{"role": "author", "kind": "human"}]),
            payload: json!({"name": "Test Patient", "dob": "1980-01-01", "sex": "F"}),
            attachments: vec![],
        }
    }

    // Bet A2 in miniature: a signed event survives a round-trip through bytes,
    // verifies, and any tampering is detected.
    #[test]
    fn sign_roundtrip_verifies_and_detects_tampering() {
        let (sk, kid) = generate_key().unwrap();
        let mut body = sample();
        body.signer_key_id = kid;

        let signed = sign(&body, &sk).unwrap();

        // Same bytes -> same content address (idempotent set-union, §3.14).
        assert_eq!(signed.content_address, event_address(&signed.signed_bytes));
        assert_eq!(signed.content_address[0..2], SHA2_256_MULTIHASH_PREFIX);

        // Round-trip the verbatim bytes (the "wire") and verify.
        let on_wire = signed.signed_bytes.clone();
        let decoded = verify_self_described(&on_wire).unwrap();
        assert_eq!(decoded, body);

        // Flip one byte of the payload region -> verification must fail.
        let mut tampered = signed.signed_bytes.clone();
        let mid = tampered.len() / 2;
        tampered[mid] ^= 0x01;
        assert!(verify_self_described(&tampered).is_err());
    }

    #[test]
    fn blob_address_is_blake3_multihash() {
        let a = blob_address(b"DICOM bytes here");
        assert_eq!(a[0..2], BLAKE3_MULTIHASH_PREFIX);
        assert_eq!(a.len(), 34);
        assert_eq!(&a[2..], blake3::hash(b"DICOM bytes here").as_bytes());
    }
}
