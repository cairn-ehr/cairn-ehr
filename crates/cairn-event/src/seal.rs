//! ADR-0052 born-sealed clinical bodies: the AEAD seal core.
//!
//! WHY THIS EXISTS: every clinical JSONB body is sealed at write under a
//! per-event DEK so the ADR-0005 erasure ladder stays reachable forever
//! (erasability, not confidentiality — the node holds custody by default).
//! Seal-then-sign: the author calls seal_event_payload BEFORE cairn_event::sign,
//! so the signature covers the ciphertext and still verifies after a shred.
//! The legibility twin travels INSIDE the sealed region under the same DEK
//! (#92 collision (a)); the outer plaintext_twin is a signed mechanical stub.

use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{Key, KeyInit, XChaCha20Poly1305, XNonce};
use zeroize::Zeroizing;

use crate::EventError;

/// The one sealed-body AEAD algorithm (crypto-agile: the container names it,
/// additive evolution adds members, never reinterprets this one).
pub const SEAL_ALG: &str = "xchacha20poly1305";
/// AAD domain tag: binds a container to the seal plane AND its event id, so a
/// ciphertext cannot be transplanted between events even pre-signature-check.
const SEAL_AAD_CONTEXT: &[u8] = b"cairn-sealed-body-v1";

/// Mechanical outer twin for a sealed event (principle 11: the row stays
/// honestly self-describing as WHAT it is; the real twin is under the seal).
pub fn seal_stub_twin(event_type: &str) -> String {
    format!("sealed {event_type} event — twin under seal (ADR-0052)")
}

/// True iff a payload value is the ADR-0052 sealed container shape.
pub fn is_sealed_container(payload: &serde_json::Value) -> bool {
    payload.get("sealed").and_then(|v| v.as_bool()) == Some(true)
}

/// Builds the AEAD associated data for one event: the fixed domain tag plus the
/// event id, so ciphertext from one event can never be swapped onto another's row.
fn aad_for(event_id: &str) -> Vec<u8> {
    let mut aad = SEAL_AAD_CONTEXT.to_vec();
    aad.extend_from_slice(event_id.as_bytes());
    aad
}

/// Seal a clear payload + its clear twin under a FRESH per-event DEK.
/// Returns (container, dek). The caller places the container into
/// EventBody.payload and seal_stub_twin(..) into EventBody.plaintext_twin,
/// then signs — seal-then-sign, so the signature covers the ciphertext.
pub fn seal_event_payload(
    payload: &serde_json::Value,
    twin: &str,
    event_id: &str,
) -> Result<(serde_json::Value, Zeroizing<[u8; 32]>), EventError> {
    // Fresh DEK + nonce from the OS RNG (production key material is always
    // random — house rule 6 applies to tests only).
    let mut dek = Zeroizing::new([0u8; 32]);
    getrandom::fill(dek.as_mut()).map_err(|e| EventError::Seal(format!("entropy failure: {e}")))?;
    let mut nonce = [0u8; 24];
    getrandom::fill(&mut nonce).map_err(|e| EventError::Seal(format!("entropy failure: {e}")))?;

    // The inner (sealed) region: clear payload AND clear twin together, so the
    // twin is under the SAME DEK as its body (#92 (a), normative in ADR-0052).
    let inner = serde_json::json!({ "payload": payload, "plaintext_twin": twin });
    let inner_bytes = serde_json::to_vec(&inner)
        .map_err(|e| EventError::Seal(format!("inner serialize: {e}")))?;

    let cipher = XChaCha20Poly1305::new(Key::from_slice(dek.as_ref()));
    let ct = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &inner_bytes,
                aad: &aad_for(event_id),
            },
        )
        .map_err(|_| EventError::Seal("encrypt failure".into()))?;

    let container = serde_json::json!({
        "sealed": true,
        "alg": SEAL_ALG,
        "nonce": hex::encode(nonce),
        "ct": hex::encode(ct),
    });
    Ok((container, dek))
}

/// Open a sealed container with its DEK. Returns (clear payload, clear twin).
/// Errors on wrong DEK, wrong event id (AAD), unknown alg, or malformed shape
/// — every failure is a refusal, never a silent fallback.
pub fn unseal_event_payload(
    container: &serde_json::Value,
    dek: &[u8; 32],
    event_id: &str,
) -> Result<(serde_json::Value, String), EventError> {
    if !is_sealed_container(container) {
        return Err(EventError::Seal("not a sealed container".into()));
    }
    let alg = container.get("alg").and_then(|v| v.as_str()).unwrap_or("");
    if alg != SEAL_ALG {
        return Err(EventError::Seal(format!("unknown seal alg {alg:?}")));
    }
    let nonce = hex::decode(
        container
            .get("nonce")
            .and_then(|v| v.as_str())
            .unwrap_or(""),
    )
    .map_err(|_| EventError::Seal("malformed nonce hex".into()))?;
    let ct = hex::decode(container.get("ct").and_then(|v| v.as_str()).unwrap_or(""))
        .map_err(|_| EventError::Seal("malformed ct hex".into()))?;
    if nonce.len() != 24 {
        return Err(EventError::Seal("nonce must be 24 bytes".into()));
    }
    let cipher = XChaCha20Poly1305::new(Key::from_slice(dek));
    let inner_bytes = cipher
        .decrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: ct.as_slice(),
                aad: &aad_for(event_id),
            },
        )
        .map_err(|_| EventError::Seal("AEAD open failed (wrong DEK or tampered)".into()))?;
    let inner: serde_json::Value = serde_json::from_slice(&inner_bytes)
        .map_err(|e| EventError::Seal(format!("inner parse: {e}")))?;
    let payload = inner
        .get("payload")
        .cloned()
        .ok_or_else(|| EventError::Seal("inner missing payload".into()))?;
    let twin = inner
        .get("plaintext_twin")
        .and_then(|v| v.as_str())
        .ok_or_else(|| EventError::Seal("inner missing plaintext_twin".into()))?
        .to_string();
    Ok((payload, twin))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn dek_fixture() -> [u8; 32] {
        // House rule 6: derived, never a literal.
        std::array::from_fn(|i| (i as u8).wrapping_mul(7).wrapping_add(3))
    }

    #[test]
    fn seal_then_unseal_round_trips_payload_and_twin() {
        let payload = json!({"medication_id": "m", "substance": {"term": "amoxicillin"}});
        let twin = "amoxicillin 500 mg — patient reports taking";
        let (container, dek) = seal_event_payload(&payload, twin, "evt-1").unwrap();
        assert_eq!(container["sealed"], json!(true));
        assert_eq!(container["alg"], json!(SEAL_ALG));
        // No plaintext leaks into the container.
        let ct_json = container.to_string();
        assert!(!ct_json.contains("amoxicillin"));
        let (p2, t2) = unseal_event_payload(&container, &dek, "evt-1").unwrap();
        assert_eq!(p2, payload);
        assert_eq!(t2, twin);
    }

    #[test]
    fn unseal_fails_with_wrong_dek() {
        let (container, _dek) = seal_event_payload(&json!({"a": 1}), "t", "evt-1").unwrap();
        let wrong = dek_fixture();
        assert!(unseal_event_payload(&container, &wrong, "evt-1").is_err());
    }

    #[test]
    fn unseal_fails_when_event_id_differs_aad_binding() {
        // The AAD binds the container to its event: a ciphertext transplanted
        // onto another event id must not open (defense in depth beside the sig).
        let (container, dek) = seal_event_payload(&json!({"a": 1}), "t", "evt-1").unwrap();
        assert!(unseal_event_payload(&container, &dek, "evt-2").is_err());
    }

    #[test]
    fn two_seals_of_same_payload_differ_fresh_dek_and_nonce() {
        let p = json!({"a": 1});
        let (c1, d1) = seal_event_payload(&p, "t", "e").unwrap();
        let (c2, d2) = seal_event_payload(&p, "t", "e").unwrap();
        assert_ne!(c1["ct"], c2["ct"]);
        assert_ne!(d1.as_slice(), d2.as_slice());
    }

    #[test]
    fn is_sealed_container_detects_shape() {
        let (c, _d) = seal_event_payload(&json!({}), "t", "e").unwrap();
        assert!(is_sealed_container(&c));
        assert!(!is_sealed_container(&json!({"medication_id": "m"})));
        assert!(!is_sealed_container(&json!({"sealed": false})));
    }

    #[test]
    fn stub_twin_names_type_and_seal_state() {
        let s = seal_stub_twin("clinical.medication.asserted");
        assert!(s.contains("clinical.medication.asserted"));
        assert!(s.contains("seal"));
    }
}
