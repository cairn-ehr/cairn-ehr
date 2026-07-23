//! Issue #216 — the born clock-confidence grade on the wire (ADR-0058).
use cairn_event::{canonical_cbor, ClockGrade, EventBody, Hlc};

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
    // A body encoded WITHOUT the `clock_grade` key must deserialize with the field
    // defaulted to Unknown (`#[serde(default)]`). This is a serde-default check via
    // the crate's real CBOR wire format (ciborium), not serde_json (Finding 2) — the
    // genuine "a pre-#216 signed blob still verifies AND reads Unknown" proof (the
    // additive-only guarantee end to end, wire-signing included) lives in
    // `cairn_event::tests::pre_216_signed_blob_without_clock_grade_verifies_and_defaults_to_unknown`
    // in lib.rs, which reaches the private `cose_sign1_in_context` to sign a struct
    // that mirrors `EventBody` exactly minus `clock_grade` — the typed `EventBody`
    // literal can't itself omit the field, since it is now mandatory.
    let b = body(ClockGrade::SelfAsserted);
    let bytes = canonical_cbor(&b).unwrap();
    let mut value: ciborium::value::Value = ciborium::from_reader(&bytes[..]).unwrap();
    match &mut value {
        ciborium::value::Value::Map(entries) => {
            entries.retain(|(k, _)| k.as_text() != Some("clock_grade"));
        }
        _ => panic!("canonical_cbor did not encode EventBody as a CBOR map"),
    }
    let mut without_grade = Vec::new();
    ciborium::into_writer(&value, &mut without_grade).unwrap();
    let legacy: EventBody = ciborium::from_reader(&without_grade[..]).unwrap();
    assert_eq!(
        legacy.clock_grade,
        ClockGrade::Unknown,
        "absent → Unknown default"
    );
}
