//! cairn_pgx — the in-database safety floor (Spike 0002 §4.3).
//!
//! A thin pgrx wrapper over the existing `cairn-event` crate so there is ONE
//! verify/parse implementation, not two. This is the ADR-0002 production move
//! ("the verify gate moves in-DB so no unverified row can enter the log") made
//! real for the spike. Safety-critical Rust per the §9 blast-radius rule.

use pgrx::prelude::*;

::pgrx::pg_module_magic!();

/// True iff `signed` is a valid COSE_Sign1/Ed25519 event that verifies against
/// its self-described key. The C5.1 floor: an unsigned or malformed event is
/// rejected in-DB, even for a caller with direct DB access.
#[pg_extern(immutable, parallel_safe)]
fn cairn_verify(signed: &[u8]) -> bool {
    cairn_event::verify_self_described(signed).is_ok()
}

#[cfg(any(test, feature = "pg_test"))]
#[pg_schema]
mod tests {
    use pgrx::prelude::*;

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
