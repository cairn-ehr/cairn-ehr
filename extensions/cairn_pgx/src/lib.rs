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

/// Diagnostic companion to `cairn_verify`: NULL when the bytes verify, else the
/// legible `EventError` string ("signing-context mismatch…", "signature
/// verification failed", …). The doors keep gating on the boolean; this exists so
/// an operator (or a future door version) can surface WHY a blob was rejected —
/// without it, ADR-0040's legible `ContextMismatch` dies at the SQL boundary and a
/// wire-format skew is indistinguishable from tampering.
#[pg_extern(immutable, parallel_safe)]
fn cairn_verify_error(signed: &[u8]) -> Option<String> {
    cairn_event::verify_self_described(signed)
        .err()
        .map(|e| e.to_string())
}

/// The version of the verify floor actually LOADED into this backend — the
/// compiled .so, not the extension-catalog entry (`\dx` can lie after a rebuild
/// without `ALTER EXTENSION`). Lets a daemon at startup, or an operator mid-outage,
/// detect a stale library after a wire-format change (e.g. the ADR-0040 signing
/// contexts, which this floor enforces from 0.2.0) instead of diagnosing a generic
/// "signature verification failed" write outage.
#[pg_extern(immutable, parallel_safe)]
fn cairn_pgx_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
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

/// True iff `content` BLAKE3-hashes to the multihash blob address `addr` — the
/// byte tier's self-verifying property (§3.14/ADR-0013), restated IN-DB so that
/// `present := TRUE` with wrong bytes is impossible even for a caller with raw SQL
/// access (the db/026 trigger floor; previously an L2 promise recorded as an
/// honest gap in db/003). Thin wrapper over `cairn_event::blob_address` — the
/// same implementation cairn-sync verifies with, so there is ONE hashing rule,
/// never two. A malformed address (wrong length / wrong multihash prefix) is
/// simply FALSE: fail closed, never an error path a hostile caller can steer.
#[pg_extern(immutable, parallel_safe)]
fn cairn_blob_verify(addr: &[u8], content: &[u8]) -> bool {
    cairn_event::blob_address(content) == addr
}

/// Diagnostic companion to `cairn_blob_verify`, mirroring `cairn_verify_error`:
/// NULL when the pair verifies, else a legible reason distinguishing a malformed
/// address from a genuine content mismatch. The floor gates on the boolean; this
/// exists so the db/026 guard can surface WHY as exception DETAIL.
#[pg_extern(immutable, parallel_safe)]
fn cairn_blob_verify_error(addr: &[u8], content: &[u8]) -> Option<String> {
    // Diagnose the address FIRST: both checks below are constant-time over a
    // 34-byte input, while hashing `content` is a full pass over possibly
    // multi-GB bytes — never pay the hash to reject an address that is
    // malformed for free. Two distinct messages so the operator sees the actual
    // cause: a wrong LENGTH and a wrong PREFIX (e.g. a sha2-256 EVENT address
    // passed as a blob address — right length, wrong hash family) are different
    // mistakes.
    if addr.len() != 34 {
        return Some(format!(
            "blob address is not a BLAKE3 multihash (expected 0x1e 0x20 + 32 bytes, got {} bytes)",
            addr.len()
        ));
    }
    if cairn_event::blake3_root_from_address(addr).is_err() {
        return Some(format!(
            "blob address is not a BLAKE3 multihash (wrong multihash prefix 0x{}, expected 0x1e20)",
            hex::encode(&addr[..2])
        ));
    }
    let actual = cairn_event::blob_address(content);
    if actual == addr {
        return None;
    }
    Some(format!(
        "content ({} bytes) hashes to {}, but the address names {}",
        content.len(),
        hex::encode(&actual[2..]),
        hex::encode(&addr[2..])
    ))
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
            hlc: cairn_event::Hlc {
                wall: 5,
                counter: 0,
                node_origin: "t".into(),
            },
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
        let id1 = crate::cairn_actor_id(pgrx::JsonB(
            serde_json::json!({"model": "m", "skill_epoch": "e"}),
        ));
        let id2 = crate::cairn_actor_id(pgrx::JsonB(
            serde_json::json!({"skill_epoch": "e", "model": "m"}),
        ));
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

    // The diagnostics surface: a good event yields NULL, a bad blob yields the
    // legible EventError string, and the loaded-library version is readable so a
    // daemon/operator can detect a stale .so after a wire-format change.
    #[pg_test]
    fn verify_error_and_version_are_legible() {
        let (sk, kid) = cairn_event::generate_key().unwrap();
        let body = cairn_event::EventBody {
            event_id: "00000000-0000-7000-8000-000000000020".into(),
            patient_id: "00000000-0000-7000-8000-000000000021".into(),
            event_type: "advisory.added".into(),
            schema_version: "advisory/1".into(),
            hlc: cairn_event::Hlc {
                wall: 7,
                counter: 0,
                node_origin: "t".into(),
            },
            t_effective: None,
            signer_key_id: kid,
            contributors: serde_json::json!([]),
            payload: serde_json::json!({"k": "v"}),
            attachments: vec![],
            plaintext_twin: None,
        };
        let signed = cairn_event::sign(&body, &sk).unwrap();
        assert!(crate::cairn_verify_error(&signed.signed_bytes).is_none());
        assert!(crate::cairn_verify_error(b"not an event").is_some());
        assert_eq!(crate::cairn_pgx_version(), env!("CARGO_PKG_VERSION"));
    }

    // The blob floor primitive: matching bytes verify; one flipped byte, a
    // truncated address, a wrong-prefix (sha2-256) address, and an empty address
    // all fail CLOSED — false, never a panic. The error text is legible and NULL
    // on success (the db/026 guard surfaces it as exception DETAIL).
    #[pg_test]
    fn blob_verify_accepts_matching_rejects_tampered_and_malformed() {
        let content = b"DICOM bytes here";
        let addr = cairn_event::blob_address(content);
        assert!(crate::cairn_blob_verify(&addr, content));
        assert!(crate::cairn_blob_verify_error(&addr, content).is_none());

        // One flipped content byte: refused, with both hashes named.
        let mut bad = content.to_vec();
        bad[3] ^= 0x01;
        assert!(!crate::cairn_blob_verify(&addr, &bad));
        let err = crate::cairn_blob_verify_error(&addr, &bad).expect("mismatch is diagnosed");
        assert!(
            err.contains("hashes to"),
            "mismatch names both hashes: {err}"
        );

        // Malformed addresses fail closed: truncated, wrong multihash prefix
        // (an event's sha2-256 address), and empty.
        assert!(!crate::cairn_blob_verify(&addr[..33], content));
        let sha_addr = cairn_event::event_address(content);
        assert!(!crate::cairn_blob_verify(&sha_addr, content));
        assert!(!crate::cairn_blob_verify(&[], content));
        let err = crate::cairn_blob_verify_error(&[], content).expect("malformed is diagnosed");
        assert!(
            err.contains("not a BLAKE3 multihash"),
            "malformed address is named: {err}"
        );
        assert!(
            err.contains("got 0 bytes"),
            "wrong length is named as a length problem: {err}"
        );

        // Right length, wrong hash family: diagnosed as a PREFIX problem, never
        // as the length (which is fine) — a sha2-256 event address is 34 bytes
        // like a blob address, and misnaming the cause would send the operator
        // down the wrong path.
        let err =
            crate::cairn_blob_verify_error(&sha_addr, content).expect("wrong prefix is diagnosed");
        assert!(
            err.contains("wrong multihash prefix 0x1220"),
            "prefix is the named cause: {err}"
        );
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
            hlc: cairn_event::Hlc {
                wall: 1,
                counter: 0,
                node_origin: "t".into(),
            },
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
