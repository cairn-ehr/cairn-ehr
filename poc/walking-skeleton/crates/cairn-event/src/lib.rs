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

use std::io::{Cursor, Read};

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
    #[error("malformed blob address (expected blake3 multihash)")]
    BadAddress,
    #[error("blob slice extraction: {0}")]
    BlobSlice(String),
    #[error("blob slice failed verification against the content address")]
    BlobVerify,
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

/// Compute the BLAKE3 verified-streaming **outboard** tree for a blob's bytes.
/// Stored alongside the bytes on a node that holds them; needed only to *serve*
/// slices. The bao root of this encoding equals `blake3::hash(bytes)` — i.e. the
/// `blob_address` payload — so it binds to the existing content address (§4.4).
pub fn blob_outboard(bytes: &[u8]) -> Vec<u8> {
    let (outboard, hash) = bao::encode::outboard(bytes);
    debug_assert_eq!(hash.as_bytes(), &blob_address(bytes)[2..]);
    outboard
}

/// Recover the 32-byte BLAKE3 root from a multihash blob address (`0x1e 0x20` + 32).
pub fn blake3_root_from_address(addr: &[u8]) -> Result<blake3::Hash, EventError> {
    if addr.len() != 34 || addr[0..2] != BLAKE3_MULTIHASH_PREFIX {
        return Err(EventError::BadAddress);
    }
    let bytes: [u8; 32] = addr[2..].try_into().map_err(|_| EventError::BadAddress)?;
    Ok(blake3::Hash::from(bytes))
}

/// Server side: extract a verified bao slice covering `[start, start+len)` from a
/// blob's `content` and precomputed `outboard` tree. The returned bytes are the
/// verified-streaming slice (interleaved tree nodes + data) the client decodes.
pub fn extract_slice(
    content: &[u8],
    outboard: &[u8],
    start: u64,
    len: u64,
) -> Result<Vec<u8>, EventError> {
    let mut ex = bao::encode::SliceExtractor::new_outboard(
        Cursor::new(content),
        Cursor::new(outboard),
        start,
        len,
    );
    let mut out = Vec::new();
    ex.read_to_end(&mut out).map_err(|e| EventError::BlobSlice(e.to_string()))?;
    Ok(out)
}

/// Client side — THE safety seam (§4.4): decode and verify a slice against the
/// known root, returning the verified content bytes. A tampered slice, a slice
/// claimed at the wrong offset, or verification against the wrong root all error,
/// so a lying source can never have its bytes accepted.
pub fn verify_slice(
    slice: &[u8],
    root: &blake3::Hash,
    start: u64,
    len: u64,
) -> Result<Vec<u8>, EventError> {
    let mut dec = bao::decode::SliceDecoder::new(Cursor::new(slice), root, start, len);
    let mut out = Vec::new();
    dec.read_to_end(&mut out).map_err(|_| EventError::BlobVerify)?;
    Ok(out)
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

/// Bet B (B4) — Ed25519 sign/verify throughput, ops/s. Pure CPU; the number that
/// matters on ARM (a Pi), where the safety-critical verify gate must keep up with
/// sync + chart reads.
pub fn bench_sign_verify(iters: u32) -> (f64, f64) {
    use ed25519_dalek::{Signer, Verifier};
    let sk = SigningKey::from_bytes(&[7u8; 32]);
    let vk = sk.verifying_key();
    let msg = vec![0xABu8; 512]; // a representative signed-event size (~A5: ~500 B)

    let t = std::time::Instant::now();
    for _ in 0..iters {
        std::hint::black_box(sk.sign(&msg));
    }
    let sign_per_s = iters as f64 / t.elapsed().as_secs_f64();

    let sig = sk.sign(&msg);
    let t = std::time::Instant::now();
    for _ in 0..iters {
        vk.verify(&msg, &sig).unwrap();
    }
    let verify_per_s = iters as f64 / t.elapsed().as_secs_f64();
    (sign_per_s, verify_per_s)
}

/// Bet B (B4) — SHA-256 vs BLAKE3 hashing throughput, MB/s each. This is the one
/// input that could revisit ADR-0015's *provisional* blob-digest default: if BLAKE3
/// is not faster than SHA-256 on ARM and offers no offsetting benefit, revisit.
pub fn bench_hash_mbps(total_mb: usize) -> (f64, f64) {
    use sha2::{Digest, Sha256};
    let buf = vec![0x5Au8; 1 << 20]; // 1 MiB

    let t = std::time::Instant::now();
    for _ in 0..total_mb {
        std::hint::black_box(Sha256::digest(&buf));
    }
    let sha = total_mb as f64 / t.elapsed().as_secs_f64();

    let t = std::time::Instant::now();
    for _ in 0..total_mb {
        std::hint::black_box(blake3::hash(&buf));
    }
    let blake = total_mb as f64 / t.elapsed().as_secs_f64();
    (sha, blake)
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

    // Smoke tests for the Bet B microbenchmarks: a tiny iteration count proves the
    // crypto path runs end-to-end (sign/verify succeeds, both hashes produce a rate),
    // independent of the production numbers a release build on a Pi would yield.
    #[test]
    fn bench_sign_verify_runs() {
        let (sign_per_s, verify_per_s) = bench_sign_verify(4);
        assert!(sign_per_s > 0.0, "sign throughput should be positive");
        assert!(verify_per_s > 0.0, "verify throughput should be positive");
    }

    #[test]
    fn bench_hash_mbps_runs() {
        let (sha, blake) = bench_hash_mbps(2);
        assert!(sha > 0.0, "SHA-256 throughput should be positive");
        assert!(blake > 0.0, "BLAKE3 throughput should be positive");
    }

    #[test]
    fn outboard_root_equals_blob_address() {
        let data = vec![0x33u8; 700_000];
        let ob = blob_outboard(&data);
        // The bao root must equal the BLAKE3 root we content-address by.
        let addr = blob_address(&data);
        let root = blake3_root_from_address(&addr).unwrap();
        // Ground truth: the bao root (checked inside blob_outboard) and the recovered
        // address root must both equal the plain BLAKE3 hash of the content.
        assert_eq!(root, blake3::hash(&data));
        let slice = extract_slice(&data, &ob, 0, data.len() as u64).unwrap();
        let got = verify_slice(&slice, &root, 0, data.len() as u64).unwrap();
        assert_eq!(got, data);
    }

    #[test]
    fn verify_slice_accepts_good_and_rejects_bad() {
        let data: Vec<u8> = (0..600_000u32).map(|i| (i % 251) as u8).collect();
        let ob = blob_outboard(&data);
        let addr = blob_address(&data);
        let root = blake3_root_from_address(&addr).unwrap();

        let (start, len) = (256u64 * 1024, 256u64 * 1024);
        let slice = extract_slice(&data, &ob, start, len).unwrap();

        // Good slice verifies and returns the right bytes.
        let got = verify_slice(&slice, &root, start, len).unwrap();
        assert_eq!(got, data[start as usize..(start + len) as usize]);

        // Tampered slice bytes -> reject.
        let mut bad = slice.clone();
        let m = bad.len() / 2;
        bad[m] ^= 0x01;
        assert!(verify_slice(&bad, &root, start, len).is_err());

        // Right slice, wrong claimed offset -> reject.
        assert!(verify_slice(&slice, &root, 0, len).is_err());

        // Right slice, wrong claimed length -> reject (a source can't relabel a
        // slice's span any more than it can its offset or bytes).
        assert!(verify_slice(&slice, &root, start, len * 2).is_err());

        // Right slice, wrong root -> reject.
        let other = blake3_root_from_address(&blob_address(b"different")).unwrap();
        assert!(verify_slice(&slice, &other, start, len).is_err());
    }
}
