//! The signing seam: every human signature in Cairn goes through [`EventSigner`].
//!
//! WHY A TRAIT: today the clinician's Ed25519 key is a [`SigningKey`] held in process memory.
//! Slice 5 of the sign-in design owes a signer whose key never leaves the database vault; that
//! must be a NEW IMPLEMENTATION of this trait, not a rewrite of every signing call site.
//! Callers that already hold a `&SigningKey` keep compiling unchanged (it implements the trait,
//! and `&SigningKey` coerces to `&dyn EventSigner`).
//!
//! WHY `Send + Sync`: async orchestrators hold `&dyn EventSigner` across `.await` points, which
//! requires the referent to be shareable between threads.

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};

use crate::EventError;

/// Something that can sign COSE `Sig_structure` bytes (the "to-be-signed" bytes, `tbs`) and
/// say which public key verifies them. The domain-separation context (ADR-0040) is already
/// inside `tbs`, so an implementation signs exactly what it is given and nothing else.
pub trait EventSigner: Send + Sync {
    /// The public key that verifies this signer's signatures (also the COSE `kid`).
    fn verifying_key(&self) -> VerifyingKey;
    /// Sign `tbs`. Fallible because a non-local signer (a vault) can refuse or be unreachable.
    fn sign_tbs(&self, tbs: &[u8]) -> Result<Signature, EventError>;
}

/// An in-memory key is the trivial signer: it cannot fail.
impl EventSigner for SigningKey {
    fn verifying_key(&self) -> VerifyingKey {
        SigningKey::verifying_key(self)
    }
    fn sign_tbs(&self, tbs: &[u8]) -> Result<Signature, EventError> {
        Ok(self.sign(tbs))
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        sign, sign_attestation, verify_attestation, ClockGrade, EventBody, EventSigner, Hlc,
        SigningKey,
    };
    use ed25519_dalek::{Signature, VerifyingKey};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Test-only signer that delegates to a real key and counts how often it was asked.
    struct CountingSigner {
        key: SigningKey,
        calls: AtomicUsize,
    }

    impl EventSigner for CountingSigner {
        fn verifying_key(&self) -> VerifyingKey {
            self.key.verifying_key()
        }
        fn sign_tbs(&self, tbs: &[u8]) -> Result<Signature, crate::EventError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.key.sign_tbs(tbs)
        }
    }

    fn key() -> SigningKey {
        // Runtime-derived seed, never a literal (house rule 6).
        SigningKey::from_bytes(&std::array::from_fn(|i| (i * 7 + 3) as u8))
    }

    fn counting() -> CountingSigner {
        CountingSigner {
            key: key(),
            calls: AtomicUsize::new(0),
        }
    }

    fn body() -> EventBody {
        EventBody {
            event_id: "00000000-0000-7000-8000-000000000001".into(),
            patient_id: "00000000-0000-7000-8000-000000000002".into(),
            event_type: "note.added".into(),
            schema_version: "note/1".into(),
            hlc: Hlc {
                wall: 7,
                counter: 0,
                node_origin: "n".into(),
            },
            t_effective: None,
            signer_key_id: "k".into(),
            contributors: serde_json::json!([{"actor_id": "k", "role": "recorded"}]),
            payload: serde_json::json!({"text": "BP 120/80"}),
            attachments: vec![],
            plaintext_twin: None,
            clock_grade: ClockGrade::SelfAsserted,
            safety: None,
        }
    }

    #[test]
    fn a_wrapped_key_signs_events_byte_identically() {
        let c = counting();
        let via_trait = sign(&body(), &c).unwrap();
        let direct = sign(&body(), &key()).unwrap();
        assert_eq!(via_trait.signed_bytes, direct.signed_bytes);
        assert_eq!(via_trait.content_address, direct.content_address);
        assert_eq!(c.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_wrapped_key_signs_attestations_byte_identically() {
        let c = counting();
        let ca = [9u8; 34];
        // verify_attestation requires the claimed attester id to be hex(verifying key).
        let kid = hex::encode(key().verifying_key().to_bytes());
        let via_trait = sign_attestation(&ca, &kid, "attested", &c).unwrap();
        let direct = sign_attestation(&ca, &kid, "attested", &key()).unwrap();
        assert_eq!(via_trait, direct);
        assert_eq!(c.calls.load(Ordering::SeqCst), 1);
        assert!(verify_attestation(&via_trait, &ca, &key().verifying_key()));
    }
}
