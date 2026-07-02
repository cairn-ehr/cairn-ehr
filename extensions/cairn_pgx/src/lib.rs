//! cairn_pgx — the in-database safety floor (Spike 0002 §4.3).
//!
//! A thin pgrx wrapper over the existing `cairn-event` crate so there is ONE
//! verify/parse implementation, not two. This is the ADR-0002 production move
//! ("the verify gate moves in-DB so no unverified row can enter the log") made
//! real for the spike. Safety-critical Rust per the §9 blast-radius rule.

use pgrx::prelude::*;
use pgrx::JsonB;

::pgrx::pg_module_magic!();

/// True iff `signed` is a valid COSE_Sign1/Ed25519 event that verifies against
/// its self-described key. The C5.1 floor: an unsigned or malformed event is
/// rejected in-DB, even for a caller with direct DB access.
#[pg_extern(immutable, parallel_safe)]
fn cairn_verify(signed: &[u8]) -> bool {
    cairn_event::verify_self_described(signed).is_ok()
}

/// Verify and parse an event's signed bytes into its EventBody as JSONB. Returns
/// NULL when the bytes do not verify — submit_event calls cairn_verify first for a
/// legible rejection, then this to read the body PL/pgSQL cannot parse (COSE/CBOR).
#[pg_extern(immutable, parallel_safe)]
fn cairn_body(signed: &[u8]) -> Option<JsonB> {
    let body = cairn_event::verify_self_described(signed).ok()?;
    let value = serde_json::to_value(&body).ok()?; // fail closed: a non-serializable body returns SQL NULL, which submit_event rejects
    Some(JsonB(value))
}

/// Content-address (0x1220 sha2-256 multihash) of a pinned-determinant set. An
/// actor's identity IS this hash, so bumping any pinned field mints a new actor (C4).
#[pg_extern(immutable, parallel_safe)]
fn cairn_actor_id(pinned: JsonB) -> Vec<u8> {
    cairn_event::canonical_json_address(&pinned.0)
}

/// True iff `token` is a valid attestation by `attester_key` bound to `content_address`.
#[pg_extern(immutable, parallel_safe)]
fn cairn_attestation_ok(token: &[u8], content_address: &[u8], attester_key: &[u8]) -> bool {
    let bytes: [u8; 32] = match attester_key.try_into() {
        Ok(b) => b,
        Err(_) => return false,
    };
    let vk = match cairn_event::VerifyingKey::from_bytes(&bytes) {
        Ok(v) => v,
        Err(_) => return false,
    };
    cairn_event::verify_attestation(token, content_address, &vk)
}

#[cfg(any(test, feature = "pg_test"))]
#[pg_schema]
mod tests {
    use pgrx::prelude::*;

    #[pg_test]
    fn body_returns_parsed_event_and_actor_id_is_stable() {
        let (sk, kid) = cairn_event::generate_key().unwrap();
        let body = cairn_event::EventBody {
            event_id: "00000000-0000-7000-8000-000000000010".into(),
            patient_id: "00000000-0000-7000-8000-000000000011".into(),
            event_type: "advisory.added".into(),
            schema_version: "advisory/1".into(),
            hlc: cairn_event::Hlc { wall: 5, counter: 0, node_origin: "t".into() },
            t_effective: None,
            signer_key_id: kid.clone(),
            contributors: serde_json::json!([{"actor_id": "x", "role": "triaged"}]),
            payload: serde_json::json!({"urgency": 3}),
            attachments: vec![],
            plaintext_twin: None,
        };
        let signed = cairn_event::sign(&body, &sk).unwrap();
        let parsed = crate::cairn_body(&signed.signed_bytes).expect("verifies");
        assert_eq!(parsed.0["event_type"], serde_json::json!("advisory.added"));

        // Invalid bytes -> NULL.
        assert!(crate::cairn_body(b"not an event").is_none());

        // actor_id is stable under key reorder (C4).
        let id1 = crate::cairn_actor_id(pgrx::JsonB(serde_json::json!({"model": "m", "skill_epoch": "e"})));
        let id2 = crate::cairn_actor_id(pgrx::JsonB(serde_json::json!({"skill_epoch": "e", "model": "m"})));
        assert_eq!(id1, id2);
    }

    #[pg_test]
    fn attestation_ok_checks_key_and_address() {
        let (sk, kid) = cairn_event::generate_key().unwrap();
        let ca = cairn_event::event_address(b"evt");
        let token = cairn_event::sign_attestation(&ca, &kid, "attested", &sk).unwrap();
        let pubkey = hex::decode(&kid).unwrap();
        assert!(crate::cairn_attestation_ok(&token, &ca, &pubkey));
        let other = cairn_event::event_address(b"other");
        assert!(!crate::cairn_attestation_ok(&token, &other, &pubkey));

        // Fail closed on a malformed (wrong-length) key — never panic.
        assert!(!crate::cairn_attestation_ok(&token, &ca, &[]));
        assert!(!crate::cairn_attestation_ok(&token, &ca, &[0u8; 33]));
    }

    // A signed event verifies; one flipped byte does not — the Bet A2 invariant,
    // now checked from inside PostgreSQL.
    #[pg_test]
    fn verify_accepts_good_rejects_tampered() {
        let (sk, kid) = cairn_event::generate_key().unwrap();
        let body = cairn_event::EventBody {
            event_id: "00000000-0000-7000-8000-000000000001".into(),
            patient_id: "00000000-0000-7000-8000-000000000002".into(),
            event_type: "advisory.added".into(),
            schema_version: "advisory/1".into(),
            hlc: cairn_event::Hlc { wall: 1, counter: 0, node_origin: "t".into() },
            t_effective: None,
            signer_key_id: kid,
            contributors: serde_json::json!([]),
            payload: serde_json::json!({"k": "v"}),
            attachments: vec![],
            plaintext_twin: None,
        };
        let signed = cairn_event::sign(&body, &sk).unwrap();
        assert!(crate::cairn_verify(&signed.signed_bytes));

        let mut bad = signed.signed_bytes.clone();
        let m = bad.len() / 2;
        bad[m] ^= 0x01;
        assert!(!crate::cairn_verify(&bad));
    }
}

#[cfg(test)]
pub mod pg_test {
    pub fn setup(_options: Vec<&str>) {}
    pub fn postgresql_conf_options() -> Vec<&'static str> {
        vec![]
    }
}
