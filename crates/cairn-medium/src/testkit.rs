//! Shared test fixtures for this crate's unit tests, used by `chunk`, `marker`, `container`,
//! `verify`, `record` and `segment`'s `mod tests`. Centralised rather than copied into each
//! module: the `enroll()` fixture is what every attestation test's meaning rests on, and
//! `bytes()`/`record()` are what every custody assertion in `record` and `segment` rests on —
//! independent copies would be independent places for one of them to quietly drift apart.

use crate::record::MediumRecord;
use cairn_event::{event_address, sign, EventBody, Hlc, SigningKey};

pub(crate) fn sk() -> SigningKey {
    cairn_event::generate_key().unwrap().0
}

pub(crate) fn kid(sk: &SigningKey) -> String {
    hex::encode(sk.verifying_key().to_bytes())
}

pub(crate) fn node_id(ev: &[u8]) -> String {
    hex::encode(event_address(ev))
}

/// A real, validly-signed enroll for `sk` — its content-address IS the node-id.
pub(crate) fn enroll(sk: &SigningKey, name: &str) -> Vec<u8> {
    let body = EventBody {
        event_id: uuid::Uuid::now_v7().to_string(),
        patient_id: cairn_event::NIL_PATIENT.into(),
        event_type: "node.enrolled".into(),
        schema_version: "node/1".into(),
        hlc: Hlc {
            wall: 1,
            counter: 0,
            node_origin: name.into(),
        },
        t_effective: None,
        signer_key_id: kid(sk),
        contributors: serde_json::json!([]),
        payload: serde_json::json!({ "display_name": name, "address": "10.0.0.1:7843" }),
        attachments: vec![],
        plaintext_twin: None,
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    };
    sign(&body, sk).unwrap().signed_bytes
}

/// Runtime-derived bytes for a fixture field. NEVER a literal: a byte-array literal in a
/// crypto context trips CodeQL's `rust/hard-coded-cryptographic-value` (house rule 6,
/// issue #146), and a wrapped DEK is exactly such a context.
pub(crate) fn bytes(seed: u8, len: usize) -> Vec<u8> {
    (0..len).map(|i| seed.wrapping_add(i as u8)).collect()
}

/// A `MediumRecord` fixture with the optional fields selected by `flags` (the same
/// three-bit layout `put_record`/`take_record` encode). Shared between `record`'s tests
/// (which exercise the record codec directly) and `segment`'s tests (which build `Segment`
/// fixtures out of these records) for the same reason as `bytes`: a second definition
/// would be a second place for the fixture's custody shape to drift from what the
/// record-layer tests assert against.
pub(crate) fn record(flags: u8) -> MediumRecord {
    MediumRecord {
        signed_bytes: bytes(1, 40),
        attestation: (flags & 0b001 != 0).then(|| bytes(2, 16)),
        attester_key: (flags & 0b010 != 0).then(|| bytes(3, 32)),
        dek_wrapped: (flags & 0b100 != 0).then(|| bytes(4, 48)),
        source_seq: 7,
    }
}
