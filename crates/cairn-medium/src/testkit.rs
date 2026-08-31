//! Shared test fixtures for this crate's unit tests, used by `chunk`, `marker`, `container` and
//! `verify`'s `mod tests`. Centralised rather than copied into each module: the `enroll()`
//! fixture is what every attestation test's meaning rests on, and four independent copies would
//! be four places for it to quietly drift apart.

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
