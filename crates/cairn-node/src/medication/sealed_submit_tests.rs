//! `sealed_submit`'s unit tests, moved out of the module unchanged to keep it under 500 lines.

use super::*;

/// A minimal, never-submitted `EventBody` — every field is a throwaway placeholder,
/// because `apply_safety_rung`'s two early-return branches never read past
/// `body.payload` before returning. Kept local to this test module rather than a
/// shared fixture, since nothing outside this file needs a bare body like this.
fn bare_body(payload: serde_json::Value) -> EventBody {
    EventBody {
        event_id: uuid::Uuid::now_v7().to_string(),
        patient_id: uuid::Uuid::now_v7().to_string(),
        event_type: "clinical.medication.assert".into(),
        schema_version: "1".into(),
        hlc: cairn_event::Hlc {
            wall: 0,
            counter: 0,
            node_origin: "test".into(),
        },
        t_effective: None,
        signer_key_id: "test".into(),
        contributors: serde_json::json!([]),
        payload,
        attachments: vec![],
        plaintext_twin: None,
        clock_grade: Default::default(),
        // Set directly, bypassing every verb builder in this repo (which always
        // initialise this to `None`) — exactly the "future caller reaching in
        // directly" scenario the add-on guards against.
        safety: Some(serde_json::json!({"rung": "precise", "class": "should-not-survive"})),
    }
}

/// Add-on 1 (task-6 review of #375 Task 5): the two early returns in
/// `apply_safety_rung` must clear a caller-supplied `body.safety` rather than leave it
/// standing, so "one seam decides the clear field" is a STRUCTURAL guarantee rather
/// than a convention every builder happens to follow today.
///
/// Branch 1: no `payload.safety` at all — the ordinary case for every non-medication
/// clinical verb.
#[tokio::test]
async fn a_body_with_no_precise_claim_has_its_safety_field_cleared() {
    let Some(base) = std::env::var("CAIRN_TEST_PG").ok() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    // A bare connect, not `test_serial_guard` / `connect_and_load_schema`: this test
    // never touches a table (both early-return branches return before any query), so
    // it needs a live `Client` value only because `apply_safety_rung`'s signature
    // requires one — there is nothing here to serialize against another suite.
    let client = crate::db::connect(&base).await.unwrap();
    let mut body = bare_body(serde_json::json!({}));
    assert!(body.safety.is_some(), "fixture sets it up wrong otherwise");

    apply_safety_rung(&client, &mut body).await.unwrap();

    assert!(
        body.safety.is_none(),
        "no payload.safety in ⇒ the seam must clear a pre-set body.safety, not just \
         leave it alone"
    );
}

/// Branch 2: a `payload.safety` claim that IS present but half-formed (blank class) —
/// `usable_precise_claim` refuses it (ADR-0060: never cancel the clinical write over a
/// misconfigured class-map row), and that refusal must ALSO clear a pre-set
/// `body.safety`.
#[tokio::test]
async fn a_body_with_a_half_formed_claim_has_its_safety_field_cleared() {
    let Some(base) = std::env::var("CAIRN_TEST_PG").ok() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let client = crate::db::connect(&base).await.unwrap();
    let mut body = bare_body(serde_json::json!({"safety": {"class": "", "severity": "high"}}));
    assert!(body.safety.is_some(), "fixture sets it up wrong otherwise");

    apply_safety_rung(&client, &mut body).await.unwrap();

    assert!(
        body.safety.is_none(),
        "a half-formed payload.safety claim ⇒ the seam must clear a pre-set \
         body.safety, not just leave it alone"
    );
}

/// A human signer that refuses, as a vault-backed one will when it cannot be reached.
struct RefusingSigner(SigningKey);

impl EventSigner for RefusingSigner {
    fn verifying_key(&self) -> cairn_event::VerifyingKey {
        self.0.verifying_key()
    }
    fn sign_tbs(&self, _tbs: &[u8]) -> Result<cairn_event::Signature, cairn_event::EventError> {
        Err(cairn_event::EventError::Seal("the signer refused".into()))
    }
}

/// The swap seam, pinned at compile time: `AuthorParams` takes ANY `EventSigner`, not a
/// `&SigningKey`, so slice 5's in-database signer drops in without touching a call site. And
/// its refusal stops the seal-and-sign: no medication event is built around a missing
/// signature, and the node's key does not quietly sign in the human's place.
#[test]
fn a_refusing_human_signer_seals_and_signs_nothing() {
    let human = RefusingSigner(SigningKey::from_bytes(&std::array::from_fn(|i| {
        i as u8 ^ 0x5a
    })));
    let node_sk = SigningKey::from_bytes(&std::array::from_fn(|i| (i as u8).wrapping_mul(3)));
    let human_kid = hex::encode(human.verifying_key().to_bytes());
    let author = AuthorParams {
        human_sk: &human,
        human_kid: &human_kid,
    };
    let mut body = bare_body(serde_json::json!({"drug": "test"}));
    body.plaintext_twin = Some("a test twin".into());
    let (body, signer) = apply_author(body, Some(&author), &node_sk);
    assert!(seal_and_sign(body, signer).is_err());
}
