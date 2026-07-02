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

pub mod demographics;
pub mod identity;

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
    #[error("body signer_key_id does not match the key the signature verified against")]
    SignerKeyMismatch,
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
    /// The §4.5 materialised legibility twin, authored into the signed body. Absent
    /// (None) for legacy event types whose twin submit_event still derives; present
    /// for demographic assertions, where the in-DB floor (db/010) requires it.
    /// `skip_serializing_if` ⇒ a None twin is omitted from the wire, so adding this
    /// field never changes an existing event's bytes/content-address (additive-only,
    /// principle 11 / ADR-0012).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plaintext_twin: Option<String>,
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
    getrandom::fill(&mut seed).map_err(|e| EventError::Entropy(e.to_string()))?;
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
    let body = verify_with(signed_bytes, &vk)?;
    // Bind the body's claimed signer to the key the signature actually verified
    // against. The COSE header key is what the signature *proves*; body.signer_key_id
    // is what the registry resolves and what the projection records as the author.
    // If they may disagree, a holder of ANY (even unenrolled) key can author events
    // that verify yet are ATTRIBUTED to an enrolled victim — forged authorship that
    // also leaves signed_bytes (header key) inconsistent with the signer_key_id
    // column. The signature must prove the claimed origin (founding principle 2).
    if body.signer_key_id != hex::encode(bytes) {
        return Err(EventError::SignerKeyMismatch);
    }
    Ok(body)
}

/// Mechanically derive the §3.13 plaintext legibility twin from a body. This is BOTH the
/// canonical generic *authoring* renderer (a conformant author materialises this into the body
/// via `materialise_generic_twin`, then signs it in — ADR-0039) AND the crude shape the floor
/// falls back to when an event arrives without an authored twin. Crude on purpose: derivable by
/// *any* node from the structured content, so a node generations behind still reads it as prose.
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

/// True iff an Option twin is present and not just whitespace. The single blank-test
/// shared by `resolve_twin` and `materialise_generic_twin` (DRY).
fn twin_is_present(twin: &Option<String>) -> bool {
    matches!(twin.as_deref(), Some(t) if !t.trim().is_empty())
}

/// Resolve the twin to STORE for an event, following the globalised-twin rule (ADR-0039):
/// prefer the author-materialised twin (principle 11 — the author renders it faithfully and
/// signs it in, so a reader generations behind never re-derives from a schema it may not
/// understand); fall back to the mechanically-derived twin only when the author left it absent
/// or blank (an older / non-conformant peer). The in-DB floor (db/015 `cairn_event_twin`)
/// mirrors this exact rule for the validated write door — keep the two in sync.
/// Note: the derived (fallback) twin is a non-authoritative LOCAL projection — two nodes may
/// render a twin-less event's derived twin differently, but the signed body is the convergent
/// artifact, so this never breaks set-union.
pub fn resolve_twin(body: &EventBody) -> String {
    if twin_is_present(&body.plaintext_twin) {
        // Safe: twin_is_present guarantees Some(non-blank).
        body.plaintext_twin.clone().unwrap()
    } else {
        plaintext_twin(body)
    }
}

/// Materialise the generic authored twin into a body BEFORE signing, so a conformant author
/// globalises the §3.13 twin in one call (ADR-0039). Idempotent: an already-authored twin
/// (e.g. a demographic builder's tailored twin) is left untouched, so this is safe to call on
/// any body. Must run before `sign`, as the twin becomes part of the signed/content-addressed body.
pub fn materialise_generic_twin(mut body: EventBody) -> EventBody {
    if !twin_is_present(&body.plaintext_twin) {
        body.plaintext_twin = Some(plaintext_twin(&body));
    }
    body
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

/// A §3.9 contributor: who contributed, in what role, and — only when an
/// attestation token backs it — whether they bear responsibility. The agent
/// authors with role `triaged` and `responsibility = None`, so "AI-generated /
/// un-vouched" is emergent (C1): there is no `is_ai` flag anywhere.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Contributor {
    pub actor_id: String,
    // TODO: a closed ContributorRole enum (ADR-0028) — String for the spike
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub responsibility: Option<String>,
}

/// Render a contributor set as the JSON that rides in the signed body's
/// `contributors` field (and lands in `event_log.contributors`).
pub fn contributors_json(set: &[Contributor]) -> serde_json::Value {
    serde_json::to_value(set).expect("contributor set serializes")
}

/// The payload of an attestation token: a human (or attesting actor) binds their
/// key and a responsibility-bearing role to a specific event's content-address.
/// Signed as a COSE_Sign1, verified in-DB by cairn_pgx (ADR-0008: the token, never
/// the DB session, is what confers responsibility / stops a forged human author).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AttestationBody {
    pub content_address_hex: String,
    pub attester_key_id: String,
    pub role: String,
}

/// Sign an attestation token over `content_address` (a COSE_Sign1, Ed25519).
pub fn sign_attestation(
    content_address: &[u8],
    attester_key_id: &str,
    role: &str,
    sk: &SigningKey,
) -> Result<Vec<u8>, EventError> {
    use coset::{iana, CborSerializable, CoseSign1Builder, HeaderBuilder};
    let body = AttestationBody {
        content_address_hex: hex::encode(content_address),
        attester_key_id: attester_key_id.to_string(),
        role: role.to_string(),
    };
    let mut payload = Vec::new();
    ciborium::into_writer(&body, &mut payload).map_err(|e| EventError::Cbor(e.to_string()))?;
    let kid = sk.verifying_key().to_bytes().to_vec();
    let protected = HeaderBuilder::new()
        .algorithm(iana::Algorithm::EdDSA)
        .key_id(kid)
        .build();
    let sign1 = CoseSign1Builder::new()
        .protected(protected)
        .payload(payload)
        .create_signature(b"", |tbs| sk.sign(tbs).to_bytes().to_vec())
        .build();
    sign1.to_vec().map_err(|e| EventError::Cose(e.to_string()))
}

/// Verify an attestation token against `vk`, confirm it binds `content_address`, AND
/// confirm the token's CLAIMED `attester_key_id` is the key that actually signed it.
///
/// That last check mirrors the event-side `signer_key_id` gate (see `verify_self_described`
/// / `SignerKeyMismatch`): without it, the `attester_key_id` field is forgeable attribution
/// to any consumer that reads it out of a stored token (audit UI, re-verification on sync) —
/// the signature would verify while naming a different attester. Responsibility attribution
/// (ADR-0007) must not be forgeable, so the claimed key must equal the verifying key.
pub fn verify_attestation(token: &[u8], content_address: &[u8], vk: &VerifyingKey) -> bool {
    use coset::{CborSerializable, CoseSign1};
    let sign1 = match CoseSign1::from_slice(token) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let verified = sign1
        .verify_signature(b"", |sig, tbs| {
            let signature =
                ed25519_dalek::Signature::from_slice(sig).map_err(|_| EventError::BadSignature)?;
            vk.verify(tbs, &signature).map_err(|_| EventError::BadSignature)
        })
        .is_ok();
    if !verified {
        return false;
    }
    // COSE_Sign1 signs over the payload in its TBS structure, so the payload read below is exactly the bytes that were verified above.
    let payload = match sign1.payload {
        Some(p) => p,
        None => return false,
    };
    let body: AttestationBody = match ciborium::from_reader(&payload[..]) {
        Ok(b) => b,
        Err(_) => return false,
    };
    body.content_address_hex == hex::encode(content_address)
        && body.attester_key_id == hex::encode(vk.to_bytes())
}

/// Recursively sort object keys so the encoding is canonical regardless of input
/// key order, then return the value re-built with BTreeMap-ordered objects.
fn canonicalize(v: &serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match v {
        Value::Object(m) => {
            let mut sorted = serde_json::Map::new();
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            for k in keys {
                sorted.insert(k.clone(), canonicalize(&m[k]));
            }
            Value::Object(sorted)
        }
        Value::Array(a) => Value::Array(a.iter().map(canonicalize).collect()),
        other => other.clone(),
    }
}

/// Content-address of an arbitrary JSON value: the `0x1220` sha2-256 multihash of
/// its canonical CBOR encoding. Used to derive an actor's identity from its pinned
/// determinant set (Spike 0002 / ADR-0011), so identity is the *hash of what is
/// pinned* — bumping any determinant (incl. skill_epoch) yields a new identity.
/// Determinant values are expected to be strings (model/version/skill_epoch); integer
/// numbers encode deterministically, but float values are NOT guaranteed stable across
/// serialization round-trips and must not be used as determinants.
pub fn canonical_json_address(v: &serde_json::Value) -> Vec<u8> {
    let canon = canonicalize(v);
    let mut cbor = Vec::new();
    ciborium::into_writer(&canon, &mut cbor).expect("canonical json encodes to CBOR");
    use sha2::{Digest, Sha256};
    let mut out = SHA2_256_MULTIHASH_PREFIX.to_vec();
    out.extend_from_slice(&Sha256::digest(&cbor));
    out
}

/// A human-verifiable short fingerprint of an Ed25519 public key (hex): the
/// sha2-256 of the 32 key bytes, rendered as five 4-hex-digit groups. This is the
/// out-of-band code an operator reads aloud / scans to confirm a peer's identity
/// at pairing (the MITM antidote — ADR-0017 §7). Display-only; the DB pins the key.
pub fn short_fingerprint(pubkey_hex: &str) -> Result<String, EventError> {
    use sha2::{Digest, Sha256};
    let raw = hex::decode(pubkey_hex).map_err(|_| EventError::BadKeyId)?;
    if raw.len() != 32 {
        return Err(EventError::BadKeyId);
    }
    let digest = Sha256::digest(&raw);
    let groups: Vec<String> = digest[..10]
        .chunks(2)
        .map(|c| format!("{:02X}{:02X}", c[0], c[1]))
        .collect();
    Ok(groups.join("-"))
}

/// The out-of-band pairing offer (ADR-0017 §7): a signed, operator-carried bundle
/// that introduces one node to another. The fingerprint is the human check; the
/// pubkey is what the trust set pins.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PairingBundle {
    pub node_id_hex: String,
    pub pubkey_hex: String,
    pub address: String,
    pub fingerprint: String,
    pub nonce: String,
    pub hlc: Hlc,
}

/// Sign a pairing bundle as a COSE_Sign1 (Ed25519), reusing the event signing path.
pub fn sign_pairing_bundle(b: &PairingBundle, sk: &SigningKey) -> Result<Vec<u8>, EventError> {
    use coset::{iana, CborSerializable, CoseSign1Builder, HeaderBuilder};
    let mut payload = Vec::new();
    ciborium::into_writer(b, &mut payload).map_err(|e| EventError::Cbor(e.to_string()))?;
    let kid = sk.verifying_key().to_bytes().to_vec();
    let protected = HeaderBuilder::new()
        .algorithm(iana::Algorithm::EdDSA)
        .key_id(kid)
        .build();
    let sign1 = CoseSign1Builder::new()
        .protected(protected)
        .payload(payload)
        .create_signature(b"", |tbs| sk.sign(tbs).to_bytes().to_vec())
        .build();
    sign1.to_vec().map_err(|e| EventError::Cose(e.to_string()))
}

/// Verify a pairing bundle against the key it embeds, and confirm it does not lie
/// about its own fingerprint (the fingerprint must derive from the embedded key).
pub fn verify_pairing_bundle(token: &[u8]) -> Result<PairingBundle, EventError> {
    use coset::{CborSerializable, CoseSign1};
    let sign1 = CoseSign1::from_slice(token).map_err(|e| EventError::Cose(e.to_string()))?;
    let kid = sign1.protected.header.key_id.clone();
    let bytes: [u8; 32] = kid.as_slice().try_into().map_err(|_| EventError::BadKeyId)?;
    let vk = VerifyingKey::from_bytes(&bytes).map_err(|_| EventError::BadKeyId)?;
    sign1
        .verify_signature(b"", |sig, tbs| {
            let signature =
                ed25519_dalek::Signature::from_slice(sig).map_err(|_| EventError::BadSignature)?;
            vk.verify(tbs, &signature).map_err(|_| EventError::BadSignature)
        })
        .map_err(|_| EventError::BadSignature)?;
    let payload = sign1.payload.ok_or(EventError::NoPayload)?;
    let b: PairingBundle =
        ciborium::from_reader(&payload[..]).map_err(|e| EventError::Cbor(e.to_string()))?;
    // The bundle must be honest about the key it carries and that key's fingerprint.
    if b.pubkey_hex != hex::encode(bytes) || b.fingerprint != short_fingerprint(&b.pubkey_hex)? {
        return Err(EventError::SignerKeyMismatch);
    }
    Ok(b)
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
            plaintext_twin: None,
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

    // Spike 0002 review (attribution forgery): signing with one key while claiming
    // another's signer_key_id must be rejected. The registry resolves the actor and
    // the projection records the author from signer_key_id, so it has to be bound to
    // the key the signature actually verified against.
    #[test]
    fn verify_rejects_body_claiming_a_different_signer_key() {
        let (sk, _kid) = generate_key().unwrap();
        let (_victim_sk, victim_kid) = generate_key().unwrap();
        let mut body = sample();
        body.signer_key_id = victim_kid; // claim the victim's key id...
        let signed = sign(&body, &sk).unwrap(); // ...but sign with our own key
        match verify_self_described(&signed.signed_bytes) {
            Err(EventError::SignerKeyMismatch) => {}
            other => panic!("expected SignerKeyMismatch, got {other:?}"),
        }
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

    #[test]
    fn canonical_json_address_recurses_into_nested_objects_and_arrays() {
        let a = canonical_json_address(&json!({
            "outer": {"z": 1, "a": 2},
            "list": [{"y": "1", "x": "2"}]
        }));
        let b = canonical_json_address(&json!({
            "list": [{"x": "2", "y": "1"}],
            "outer": {"a": 2, "z": 1}
        }));
        assert_eq!(a, b, "nested object/array key order must not change the address");
    }

    #[test]
    fn canonical_json_address_is_stable_under_key_order() {
        let a = canonical_json_address(&json!({"model": "m", "version": "1", "skill_epoch": "e"}));
        let b = canonical_json_address(&json!({"version": "1", "skill_epoch": "e", "model": "m"}));
        assert_eq!(a, b, "address must not depend on key order");
        assert_eq!(a[0..2], SHA2_256_MULTIHASH_PREFIX);
        assert_eq!(a.len(), 34);

        // A different pinned value yields a different actor identity (the C4 supersede trigger).
        let c = canonical_json_address(&json!({"model": "m", "version": "1", "skill_epoch": "e2"}));
        assert_ne!(a, c);
    }

    #[test]
    fn attestation_binds_key_and_content_address() {
        let (sk, kid) = generate_key().unwrap();
        let vk = sk.verifying_key();
        let ca = event_address(b"some signed event bytes");

        let token = sign_attestation(&ca, &kid, "attested", &sk).unwrap();
        assert!(verify_attestation(&token, &ca, &vk), "valid token for right key + address");

        // Wrong content-address -> reject (a token cannot be replayed onto another event).
        let other = event_address(b"a different event");
        assert!(!verify_attestation(&token, &other, &vk));

        // Wrong key -> reject (a forged attester does not verify).
        let other_vk = SigningKey::from_bytes(&[5u8; 32]).verifying_key();
        assert!(!verify_attestation(&token, &ca, &other_vk));

        // Tampered token bytes -> reject.
        let mut bad = token.clone();
        let m = bad.len() / 2;
        bad[m] ^= 0x01;
        assert!(!verify_attestation(&bad, &ca, &vk));
    }

    #[test]
    fn attestation_rejects_forged_attester_key_id() {
        // Review fix M7: a token that SIGNS with sk but CLAIMS a different attester in the
        // payload must be rejected — otherwise the attester_key_id field is forgeable
        // attribution to any consumer that reads it out of a stored token.
        let (sk, _kid) = generate_key().unwrap();
        let vk = sk.verifying_key();
        let ca = event_address(b"evt");
        // Claim a victim's key id while signing with our own key.
        let victim_kid = hex::encode(SigningKey::from_bytes(&[9u8; 32]).verifying_key().to_bytes());
        let forged = sign_attestation(&ca, &victim_kid, "attested", &sk).unwrap();
        // Signature verifies against vk and the content-address matches, but the claimed
        // attester_key_id != hex(vk), so the binding check must reject it.
        assert!(
            !verify_attestation(&forged, &ca, &vk),
            "a token whose attester_key_id != signing key must be rejected"
        );
    }

    #[test]
    fn agent_contributor_is_unvouched_by_construction() {
        let set = vec![Contributor {
            actor_id: "agent-aid".into(),
            role: "triaged".into(),
            responsibility: None,
        }];
        let v = contributors_json(&set);
        // role present, NO responsibility key, NO is_ai flag anywhere (C1).
        assert_eq!(v[0]["role"], json!("triaged"));
        assert!(v[0].get("responsibility").is_none());
        assert!(v[0].get("is_ai").is_none());
    }

    #[test]
    fn attested_contributor_serializes_responsibility_key() {
        let set = vec![Contributor {
            actor_id: "clinician-aid".into(),
            role: "attested".into(),
            responsibility: Some("authored".into()),
        }];
        let v = contributors_json(&set);
        assert_eq!(v[0]["role"], json!("attested"));
        assert_eq!(v[0]["responsibility"], json!("authored"));
    }

    #[test]
    fn fingerprint_is_deterministic_and_keyed() {
        let (_sk, kid) = generate_key().unwrap();
        let fp1 = short_fingerprint(&kid).unwrap();
        let fp2 = short_fingerprint(&kid).unwrap();
        assert_eq!(fp1, fp2, "same key -> same fingerprint");
        let (_sk2, kid2) = generate_key().unwrap();
        assert_ne!(fp1, short_fingerprint(&kid2).unwrap(), "different key -> different fingerprint");
        assert!(short_fingerprint("not-hex").is_err());
    }

    // A None authored-twin must NOT change the wire bytes vs. the pre-field shape,
    // so every existing event's content-address is preserved (append-only, principle 1).
    #[test]
    fn twin_absent_is_wire_identical_to_pre_field_shape() {
        #[derive(serde::Serialize)]
        struct LegacyBody<'a> {
            event_id: &'a str, patient_id: &'a str, event_type: &'a str,
            schema_version: &'a str, hlc: &'a Hlc, t_effective: Option<String>,
            signer_key_id: &'a str, contributors: &'a serde_json::Value,
            payload: &'a serde_json::Value, attachments: &'a Vec<AttachmentRef>,
        }
        let hlc = Hlc { wall: 1, counter: 0, node_origin: "n".into() };
        let contributors = serde_json::json!([{"actor_id": "k", "role": "triaged"}]);
        let payload = serde_json::json!({"text": "hi"});
        let attachments: Vec<AttachmentRef> = vec![];
        let legacy = LegacyBody {
            event_id: "e", patient_id: "p", event_type: "note.added",
            schema_version: "advisory/1", hlc: &hlc, t_effective: None,
            signer_key_id: "k", contributors: &contributors, payload: &payload,
            attachments: &attachments,
        };
        let body = EventBody {
            event_id: "e".into(), patient_id: "p".into(), event_type: "note.added".into(),
            schema_version: "advisory/1".into(), hlc: hlc.clone(), t_effective: None,
            signer_key_id: "k".into(), contributors: contributors.clone(),
            payload: payload.clone(), attachments: vec![], plaintext_twin: None,
        };
        let mut legacy_bytes = Vec::new();
        ciborium::into_writer(&legacy, &mut legacy_bytes).unwrap();
        assert_eq!(canonical_cbor(&body).unwrap(), legacy_bytes,
                   "None twin must encode byte-identically to the pre-field shape");
    }

    // Bytes authored before the field existed must still decode (forward-compat).
    // Encode from a GENUINE pre-field struct (no `plaintext_twin` at all) so this test
    // is self-contained: it proves the decode path defaults a missing key to None on
    // its own, and would still catch a regression even if `skip_serializing_if` were
    // removed (it does not rely on the wire-identity test holding).
    #[test]
    fn legacy_bytes_decode_with_twin_none() {
        #[derive(serde::Serialize)]
        struct LegacyBody<'a> {
            event_id: &'a str, patient_id: &'a str, event_type: &'a str,
            schema_version: &'a str, hlc: &'a Hlc, t_effective: Option<String>,
            signer_key_id: &'a str, contributors: &'a serde_json::Value,
            payload: &'a serde_json::Value, attachments: &'a Vec<AttachmentRef>,
        }
        let hlc = Hlc { wall: 1, counter: 0, node_origin: "n".into() };
        let contributors = serde_json::json!([]);
        let payload = serde_json::json!({});
        let attachments: Vec<AttachmentRef> = vec![];
        let legacy = LegacyBody {
            event_id: "e", patient_id: "p", event_type: "note.added",
            schema_version: "advisory/1", hlc: &hlc, t_effective: None,
            signer_key_id: "k", contributors: &contributors, payload: &payload,
            attachments: &attachments,
        };
        let mut bytes = Vec::new();
        ciborium::into_writer(&legacy, &mut bytes).unwrap();
        let decoded: EventBody = ciborium::from_reader(&bytes[..]).unwrap();
        assert_eq!(decoded.plaintext_twin, None,
                   "a missing plaintext_twin key must decode to None (serde default)");
    }

    #[test]
    fn pairing_bundle_roundtrips_and_rejects_tampering() {
        let (sk, kid) = generate_key().unwrap();
        let b = PairingBundle {
            node_id_hex: hex::encode(event_address(b"genesis-bytes")),
            pubkey_hex: kid.clone(),
            address: "10.0.0.2:7800".into(),
            fingerprint: short_fingerprint(&kid).unwrap(),
            nonce: "abcd1234".into(),
            hlc: Hlc { wall: 1, counter: 0, node_origin: "n".into() },
        };
        let token = sign_pairing_bundle(&b, &sk).unwrap();
        assert_eq!(verify_pairing_bundle(&token).unwrap(), b);

        // A bundle that lies about its own fingerprint is rejected.
        let mut liar = b.clone();
        liar.fingerprint = "DEAD-BEEF".into();
        let bad = sign_pairing_bundle(&liar, &sk).unwrap();
        assert!(verify_pairing_bundle(&bad).is_err());

        // Tampered bytes -> reject.
        let mut t = token.clone();
        let m = t.len() / 2; t[m] ^= 0x01;
        assert!(verify_pairing_bundle(&t).is_err());
    }

    // Globalised-twin helpers (ADR-0039). A reusable note body whose payload renders into a twin.
    fn sample_note_body() -> EventBody {
        EventBody {
            event_id: "00000000-0000-7000-8000-000000000001".into(),
            patient_id: "00000000-0000-7000-8000-000000000002".into(),
            event_type: "note.added".into(),
            schema_version: "note/1".into(),
            hlc: Hlc { wall: 7, counter: 0, node_origin: "n".into() },
            t_effective: None,
            signer_key_id: "k".into(),
            contributors: serde_json::json!([{"actor_id": "k", "role": "recorded"}]),
            payload: serde_json::json!({"text": "BP 120/80, afebrile"}),
            attachments: vec![],
            plaintext_twin: None,
        }
    }

    #[test]
    fn resolve_twin_prefers_authored_else_derives() {
        let mut body = sample_note_body();
        // Absent authored twin → derive (identical to the mechanical renderer).
        assert_eq!(resolve_twin(&body), plaintext_twin(&body));
        // Whitespace-only authored twin → still derive (treated as blank).
        body.plaintext_twin = Some("   \n".into());
        assert_eq!(resolve_twin(&body), plaintext_twin(&body));
        // Non-empty authored twin → carried verbatim.
        body.plaintext_twin = Some("Progress note: BP 120/80".into());
        assert_eq!(resolve_twin(&body), "Progress note: BP 120/80");
    }

    #[test]
    fn materialise_generic_twin_fills_blank_and_is_idempotent() {
        let body = sample_note_body();
        let m = materialise_generic_twin(body.clone());
        let twin = m.plaintext_twin.as_deref().expect("twin materialised");
        assert!(!twin.trim().is_empty(), "materialised twin is non-empty");
        assert_eq!(twin, plaintext_twin(&body), "materialised == the generic rendering");
        // Idempotent: an already-authored twin is preserved unchanged.
        let mut authored = sample_note_body();
        authored.plaintext_twin = Some("kept verbatim".into());
        let m2 = materialise_generic_twin(authored);
        assert_eq!(m2.plaintext_twin.as_deref().unwrap(), "kept verbatim");
    }

    #[test]
    fn materialised_twin_roundtrips_through_sign_verify() {
        let (sk, kid) = generate_key().unwrap();
        let mut body = sample_note_body();
        body.signer_key_id = kid;
        let body = materialise_generic_twin(body);
        let signed = sign(&body, &sk).unwrap();
        let decoded = verify_self_described(&signed.signed_bytes).unwrap();
        assert_eq!(decoded.plaintext_twin, body.plaintext_twin);
        assert!(decoded.plaintext_twin.is_some(), "a materialised twin survives the wire");
    }
}
