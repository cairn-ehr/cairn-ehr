//! Issue #216 — the born clock-confidence grade on the wire (ADR-0058).
use cairn_event::{
    canonical_cbor, generate_key, sign, verify_self_described, ClockGrade, EventBody, Hlc,
};

/// A minimal body helper — derives its own key material (house rule 6: no literals).
fn body(grade: ClockGrade) -> EventBody {
    EventBody {
        event_id: "018f00000000000000000000000000aa".into(),
        patient_id: "018f00000000000000000000000000bb".into(),
        event_type: "patient.created".into(),
        schema_version: "1".into(),
        hlc: Hlc {
            wall: 1_700_000_000_000,
            counter: 0,
            node_origin: "n1".into(),
        },
        t_effective: None,
        signer_key_id: String::new(),
        contributors: serde_json::json!([{"actor_id":"k","role":"recorded"}]),
        payload: serde_json::json!({}),
        attachments: vec![],
        plaintext_twin: None,
        clock_grade: grade,
    }
}

#[test]
fn clock_grade_round_trips_through_cbor() {
    let b = body(ClockGrade::SelfAsserted);
    let bytes = canonical_cbor(&b).unwrap();
    let back: EventBody = ciborium::from_reader(&bytes[..]).unwrap();
    assert_eq!(back.clock_grade, ClockGrade::SelfAsserted);
}

#[test]
fn clock_grade_serializes_as_kebab_string() {
    let v = serde_json::to_value(ClockGrade::MultiAnchorCorroborated).unwrap();
    assert_eq!(v, serde_json::json!("multi-anchor-corroborated"));
}

#[test]
fn absent_grade_deserializes_to_unknown() {
    // A legacy/foreign body encoded WITHOUT clock_grade must read as Unknown, and
    // must still verify (additive-only: existing signed bytes are never re-encoded).
    let (sk, kid) = generate_key().unwrap();
    // Build a legacy body by round-tripping through a map that omits clock_grade.
    // signer_key_id is set to the actual signing key's hex (must match what
    // verify_self_described derives from the COSE header — its SignerKeyMismatch
    // gate — see lib.rs's `body.signer_key_id != hex::encode(key_bytes)` check;
    // the brief's verbatim helper leaves it as an empty string, which fails that
    // gate regardless of clock_grade, so it is set here to make the body signable).
    let mut b = body(ClockGrade::SelfAsserted);
    b.signer_key_id = kid;
    let mut map: serde_json::Value = serde_json::to_value(&b).unwrap();
    map.as_object_mut().unwrap().remove("clock_grade");
    let legacy: EventBody = serde_json::from_value(map).unwrap();
    assert_eq!(
        legacy.clock_grade,
        ClockGrade::Unknown,
        "absent → Unknown default"
    );
    // And a real signed legacy blob still verifies after the field is added.
    let signed = sign(&b, &sk).unwrap();
    assert!(verify_self_described(&signed.signed_bytes).is_ok());
}
