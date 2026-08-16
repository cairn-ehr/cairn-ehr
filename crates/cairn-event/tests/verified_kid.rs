//! #412 — the authorship grader's proof-carrying inputs.
//!
//! # What was wrong
//!
//! `classify_authorship_confidence` took its two proof-carrying arguments as bare `&str`,
//! and `EventBody` carries `pub signer_key_id: String` — the body's **self-asserted**
//! claim — right beside `pub contributors`. So
//!
//! ```ignore
//! classify_authorship_confidence(&body.contributors, &body.signer_key_id, None)
//! ```
//!
//! compiled, read naturally, was the obvious line to write, and returned `Attested` for
//! any body whose forger set `contributors[0].actor_id` equal to `signer_key_id`. No
//! signature was ever checked. The word "verified" in `verified_attester` was the entire
//! security property, carried by a parameter name.
//!
//! The SQL side never had this problem for a structural reason worth copying:
//! `cairn_claim_authority` reads `event_log.attester_key`, a column that **cannot be
//! written** until `cairn_verify` and `cairn_attestation_ok` have both passed. *Reading
//! that column is the proof.* In Rust the verified read and the forged read had the
//! identical type.
//!
//! # What these tests pin
//!
//! [`VerifiedKid`] can be minted two ways, and this file pins the honest one end to end:
//! a kid minted from [`verify_self_described_event`] is the key the signature **actually**
//! verified against, and bytes that do not verify mint nothing at all. The compile-time
//! half — that `&body.signer_key_id` no longer type-checks as a signer — is a
//! `compile_fail` doctest on the classifier itself, because a runtime test cannot express
//! "this line does not build".
use cairn_event::contributor::{classify_authorship_confidence, AuthorshipConfidence, VerifiedKid};
use cairn_event::{verify_self_described_event, EventBody, EventError};

/// A minimal device-shaped body signed by `kid`, with one bearing human author claim.
///
/// `claimed_signer` is separate from the key that will actually sign, so a test can build
/// the exact forgery #412 describes: a body whose `signer_key_id` names someone else.
fn body_claiming(claimed_signer: &str, bearing_author: &str) -> EventBody {
    EventBody {
        event_id: uuid::Uuid::now_v7().to_string(),
        patient_id: uuid::Uuid::now_v7().to_string(),
        event_type: "note.added".into(),
        schema_version: "note/1".into(),
        hlc: cairn_event::Hlc {
            wall: 1,
            counter: 0,
            node_origin: "n1".into(),
        },
        t_effective: None,
        signer_key_id: claimed_signer.to_string(),
        contributors: serde_json::json!([{"actor_id": bearing_author, "role": "authored"}]),
        payload: serde_json::json!({"text": "a note"}),
        attachments: vec![],
        plaintext_twin: Some("a note".into()),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    }
}

/// The honest path: verification mints the kid, and the kid is the key that signed.
///
/// Grading through the minted value returns `Attested` here because the bearing author IS
/// the signer — the same answer the old API gave, but now reachable only after a signature
/// check rather than by reading two fields out of the same untrusted blob.
#[test]
fn a_kid_minted_by_verification_is_the_key_that_actually_signed() {
    let (sk, kid) = cairn_event::generate_key().expect("keygen");
    let body = body_claiming(&kid, &kid);
    let signed = cairn_event::sign(&body, &sk).expect("signs");

    let verified = verify_self_described_event(&signed.signed_bytes).expect("verifies");
    assert_eq!(
        verified.signer().as_str(),
        kid,
        "the mint is the signing key"
    );
    assert_eq!(
        classify_authorship_confidence(&verified.body().contributors, verified.signer(), None),
        AuthorshipConfidence::Attested
    );
}

/// The forgery #412 names, run end to end: a body that CLAIMS another signer never
/// reaches the grader at all, because it never verifies.
///
/// This is the property that makes [`VerifiedKid`]'s mint sound rather than decorative.
/// `verify_self_described` binds the body's claimed `signer_key_id` to the COSE header key
/// the signature verified against (`EventError::SignerKeyMismatch`); if that bind were
/// ever removed, the mint would start handing out a forgeable value and this test — not a
/// reviewer — is what notices.
#[test]
fn a_body_claiming_a_signer_it_did_not_use_mints_nothing() {
    let (sk, kid) = cairn_event::generate_key().expect("keygen");
    let (_victim_sk, victim_kid) = cairn_event::generate_key().expect("keygen");

    // Signed with OUR key, but the body names the victim as both signer and author —
    // exactly the shape that used to grade `Attested` when read straight off the body.
    let forged = body_claiming(&victim_kid, &victim_kid);
    assert_ne!(kid, victim_kid);
    let signed = cairn_event::sign(&forged, &sk).expect("signs");

    match verify_self_described_event(&signed.signed_bytes) {
        Err(EventError::SignerKeyMismatch) => {}
        other => panic!("a mismatched signer claim must not verify, got {other:?}"),
    }
}

/// Tampered bytes mint nothing: the mint is downstream of the signature, not beside it.
#[test]
fn tampered_bytes_mint_nothing() {
    let (sk, kid) = cairn_event::generate_key().expect("keygen");
    let body = body_claiming(&kid, &kid);
    let mut signed = cairn_event::sign(&body, &sk).expect("signs").signed_bytes;
    // Flip a bit deep inside the payload. Which byte does not matter — any change
    // invalidates the COSE_Sign1 signature over the whole TBS structure.
    let last = signed.len() - 1;
    signed[last] ^= 0x01;

    assert!(
        verify_self_described_event(&signed).is_err(),
        "tampered bytes must not yield a verified event, and so must not yield a kid"
    );
}

/// The DB-provenance mint, and the reason it is a separate named constructor.
///
/// `event_log.signer_key_id` and `event_log.attester_key` are proof-carrying columns: the
/// in-DB floor (db/005 step 1) runs `cairn_verify`, which IS `verify_self_described`, so a
/// row exists only if the signature verified against the key the row names. A caller
/// reading those columns holds the same proof this crate's verifier produces, arrived at
/// by a different route — and there is no `&[u8]` around to re-verify.
///
/// The constructor is deliberately named after that provenance rather than something
/// neutral like `new`: the guarantee cannot be checked by the compiler at THIS boundary,
/// so what is left is making a wrong call site conspicuous in review and greppable in the
/// tree.
#[test]
fn the_db_column_mint_carries_the_value_through_unchanged() {
    let kid = "a".repeat(64);
    let v = VerifiedKid::from_event_log_column(&kid);
    assert_eq!(v.as_str(), kid);

    // And it grades exactly as the verification-minted kid would: the newtype changes the
    // TYPE of the argument, never the classification law (pinned by the property suite).
    let contributors = serde_json::json!([{"actor_id": kid, "role": "authored"}]);
    assert_eq!(
        classify_authorship_confidence(&contributors, v, None),
        AuthorshipConfidence::Attested
    );
}
