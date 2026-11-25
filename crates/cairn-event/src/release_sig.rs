//! Detached signatures for the release plane (#909 slice C2): trust roots and release manifests.
//!
//! A release file (a trust root, or a release manifest) is signed by SEVERAL steward keys, so
//! each signature is DETACHED (ADR-0055): the COSE_Sign1 carries no payload, and the verifier is
//! handed the payload bytes separately. That keeps one copy of the payload however many
//! signatures it gathers, and lets its content address be the SHA-256 of the payload alone.
//!
//! Apart from the detachment, these signatures are built and checked exactly like every other
//! signature in this crate (ADR-0040): the same protected header (EdDSA, key id, the context as
//! content type, via `protected_header`), the context again as `external_aad`, and the same
//! fail-closed header checks (`check_protected_header`). Each public function binds its own
//! context, so no caller can pass the wrong one: a root signature never verifies as a manifest,
//! and neither ever verifies as an event.
//!
//! This module proves only "key K signed these bytes in this context". Which keys count, and how
//! many signatures are enough, is the verifier crate's job, not this one's.

use ed25519_dalek::Verifier;

use crate::{
    check_protected_header, protected_header, EventError, EventSigner, SigningContext,
    VerifyingKey, CTX_RELEASE_MANIFEST, CTX_TRUST_ROOT,
};

/// Sign a trust-root file's bytes, detached, in [`CTX_TRUST_ROOT`].
pub fn sign_trust_root(
    payload: &[u8],
    sk: &(impl EventSigner + ?Sized),
) -> Result<Vec<u8>, EventError> {
    sign_detached(payload, sk, CTX_TRUST_ROOT)
}

/// Sign a release manifest's bytes, detached, in [`CTX_RELEASE_MANIFEST`].
pub fn sign_release_manifest(
    payload: &[u8],
    sk: &(impl EventSigner + ?Sized),
) -> Result<Vec<u8>, EventError> {
    sign_detached(payload, sk, CTX_RELEASE_MANIFEST)
}

/// Check that `sig` is `vk`'s detached signature over `payload` as a trust root.
pub fn verify_trust_root_sig(
    sig: &[u8],
    payload: &[u8],
    vk: &VerifyingKey,
) -> Result<(), EventError> {
    verify_detached(sig, payload, vk, CTX_TRUST_ROOT)
}

/// Check that `sig` is `vk`'s detached signature over `payload` as a release manifest.
pub fn verify_release_manifest_sig(
    sig: &[u8],
    payload: &[u8],
    vk: &VerifyingKey,
) -> Result<(), EventError> {
    verify_detached(sig, payload, vk, CTX_RELEASE_MANIFEST)
}

/// Read the key id (the claimed 32-byte Ed25519 public key) of a detached signature, so a
/// verifier can pick which of its pinned keys to try. Like `key_id`, this proves NOTHING: a
/// `verify_*_sig` call against the chosen key must follow.
pub fn detached_key_id(sig: &[u8]) -> Result<Vec<u8>, EventError> {
    let kid = crate::key_id(sig)?;
    if kid.len() != 32 {
        return Err(EventError::BadKeyId);
    }
    Ok(kid)
}

/// The detached twin of `cose_sign1_in_context`: same header, same `external_aad`, but the
/// payload goes into the Sig_structure only, never into the blob.
fn sign_detached(
    payload: &[u8],
    sk: &(impl EventSigner + ?Sized),
    ctx: SigningContext,
) -> Result<Vec<u8>, EventError> {
    use coset::{CborSerializable, CoseSign1Builder};
    let sign1 = CoseSign1Builder::new()
        .protected(protected_header(&sk.verifying_key(), ctx))
        // The fallible variant lets a vault-backed signer's refusal propagate as an error.
        .try_create_detached_signature(payload, ctx.as_bytes(), |tbs| {
            sk.sign_tbs(tbs).map(|sig| sig.to_bytes().to_vec())
        })?
        .build();
    sign1.to_vec().map_err(|e| EventError::Cose(e.to_string()))
}

/// The detached twin of `cose_verify1_parsed`: the same fail-closed header checks, then the
/// signature over `payload` with `ctx` as `external_aad`.
fn verify_detached(
    sig: &[u8],
    payload: &[u8],
    vk: &VerifyingKey,
    ctx: SigningContext,
) -> Result<(), EventError> {
    use coset::CborSerializable;
    let sign1 = coset::CoseSign1::from_slice(sig).map_err(|e| EventError::Cose(e.to_string()))?;
    check_protected_header(&sign1, ctx)?;
    // A blob carrying its own payload is not a detached signature; refuse it here, because
    // coset's detached verifier panics on one. This `Cose(..)` means "not detached": a verifier
    // crate maps it to Malformed.
    if sign1.payload.is_some() {
        return Err(EventError::Cose(
            "a detached signature must carry no payload".into(),
        ));
    }
    sign1.verify_detached_signature(payload, ctx.as_bytes(), |sig, tbs| {
        let signature =
            ed25519_dalek::Signature::from_slice(sig).map_err(|_| EventError::BadSignature)?;
        vk.verify(tbs, &signature)
            .map_err(|_| EventError::BadSignature)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{sign, EventBody, EventError, Hlc, SigningKey};

    // Runtime-derived key (house rule 6): never a literal in a crypto context.
    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&std::array::from_fn(|i| i as u8 + seed))
    }

    #[test]
    fn a_trust_root_signature_round_trips() {
        let sk = key(7);
        let sig = sign_trust_root(b"root v1", &sk).unwrap();
        verify_trust_root_sig(&sig, b"root v1", &sk.verifying_key()).unwrap();
    }

    #[test]
    fn a_release_manifest_signature_round_trips() {
        let sk = key(7);
        let sig = sign_release_manifest(b"manifest", &sk).unwrap();
        verify_release_manifest_sig(&sig, b"manifest", &sk.verifying_key()).unwrap();
    }

    #[test]
    fn a_tampered_payload_fails() {
        let sk = key(7);
        let sig = sign_trust_root(b"root v1", &sk).unwrap();
        let err = verify_trust_root_sig(&sig, b"root v2", &sk.verifying_key()).unwrap_err();
        assert!(matches!(err, EventError::BadSignature), "{err:?}");
    }

    #[test]
    fn another_key_fails() {
        let sig = sign_trust_root(b"root v1", &key(7)).unwrap();
        let err = verify_trust_root_sig(&sig, b"root v1", &key(9).verifying_key()).unwrap_err();
        assert!(matches!(err, EventError::BadSignature), "{err:?}");
    }

    #[test]
    fn a_root_signature_never_verifies_as_a_manifest() {
        let sk = key(7);
        let sig = sign_trust_root(b"same bytes", &sk).unwrap();
        let err =
            verify_release_manifest_sig(&sig, b"same bytes", &sk.verifying_key()).unwrap_err();
        assert!(matches!(err, EventError::ContextMismatch), "{err:?}");
    }

    #[test]
    fn a_manifest_signature_never_verifies_as_a_root() {
        let sk = key(7);
        let sig = sign_release_manifest(b"same bytes", &sk).unwrap();
        let err = verify_trust_root_sig(&sig, b"same bytes", &sk.verifying_key()).unwrap_err();
        assert!(matches!(err, EventError::ContextMismatch), "{err:?}");
    }

    #[test]
    fn an_event_signature_never_verifies_as_a_root() {
        use coset::{CborSerializable, CoseSign1};
        let sk = key(7);
        let body = EventBody {
            event_id: "e".into(),
            patient_id: crate::NIL_PATIENT.into(),
            event_type: "patient.amended".into(),
            schema_version: "patient/1".into(),
            hlc: Hlc {
                wall: 1,
                counter: 0,
                node_origin: "n".into(),
            },
            t_effective: None,
            signer_key_id: String::new(),
            contributors: serde_json::json!([]),
            payload: serde_json::json!({}),
            attachments: vec![],
            plaintext_twin: None,
            clock_grade: Default::default(),
            safety: None,
        };
        let blob = sign(&body, &sk).unwrap().signed_bytes;
        let payload = CoseSign1::from_slice(&blob).unwrap().payload.unwrap();
        let err = verify_trust_root_sig(&blob, &payload, &sk.verifying_key()).unwrap_err();
        assert!(matches!(err, EventError::ContextMismatch), "{err:?}");
    }

    // An attached blob in the right context must be refused, never panic (coset's detached
    // verifier asserts there is no payload).
    #[test]
    fn an_attached_blob_in_the_root_context_is_refused_not_a_panic() {
        use coset::{CborSerializable, CoseSign1Builder};
        let sk = key(7);
        let blob = CoseSign1Builder::new()
            .protected(protected_header(&sk.verifying_key(), crate::CTX_TRUST_ROOT))
            .payload(b"root v1".to_vec())
            .build()
            .to_vec()
            .unwrap();
        let err = verify_trust_root_sig(&blob, b"root v1", &sk.verifying_key()).unwrap_err();
        assert!(matches!(err, EventError::Cose(_)), "{err:?}");
    }

    #[test]
    fn a_short_key_id_is_refused() {
        use coset::{iana, CborSerializable, CoseSign1Builder, HeaderBuilder};
        let header = HeaderBuilder::new()
            .algorithm(iana::Algorithm::EdDSA)
            .key_id(vec![1; 31])
            .content_type(crate::CTX_TRUST_ROOT.as_str().to_string())
            .build();
        let blob = CoseSign1Builder::new()
            .protected(header)
            .build()
            .to_vec()
            .unwrap();
        let err = detached_key_id(&blob).unwrap_err();
        assert!(matches!(err, EventError::BadKeyId), "{err:?}");
    }

    #[test]
    fn a_detached_signature_carries_no_payload() {
        use coset::{CborSerializable, CoseSign1};
        let sig = sign_trust_root(b"root v1", &key(7)).unwrap();
        assert!(CoseSign1::from_slice(&sig).unwrap().payload.is_none());
    }

    #[test]
    fn the_key_id_is_read_back() {
        let sk = key(7);
        let sig = sign_release_manifest(b"manifest", &sk).unwrap();
        assert_eq!(
            detached_key_id(&sig).unwrap(),
            sk.verifying_key().to_bytes().to_vec()
        );
    }

    #[test]
    fn junk_is_malformed_not_a_panic() {
        assert!(detached_key_id(b"junk").is_err());
        assert!(verify_trust_root_sig(b"junk", b"x", &key(7).verifying_key()).is_err());
    }
}
