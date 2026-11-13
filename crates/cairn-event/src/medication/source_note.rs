//! The two optional "where from / why" keys on a medication assertion.
//!
//! Pure: no clock, no I/O. Keys are inserted only when present, never as null, so an
//! assertion without them serialises byte-identically to one built before they existed
//! (principle 11).
//!
//! Why TWO keys and never one free-text "note": `source_detail` says WHERE the claim came
//! from ("GP letter", "daughter, from the packet"), while `why` says why the clinician
//! charted it this way. Fused into one box, a reader (or a later matcher) cannot tell a
//! provenance from a rationale, and a clinician with only one of the two would be pushed to
//! invent the other (spec batch-charting slice 2 §2, anti-goal 3).
use serde_json::{json, Map, Value};

/// The optional provenance detail and rationale of one medication line.
#[derive(Debug, Clone, Copy, Default)]
pub struct SourceNote<'a> {
    /// Free text: where the claim came from, finer than the `info_source` category.
    pub source_detail: Option<&'a str>,
    /// Free text: why the clinician charted the line this way.
    pub why: Option<&'a str>,
}

/// Write the present keys into an assertion's top-level `payload` (never as null).
pub fn insert_source_note(payload: &mut Map<String, Value>, n: &SourceNote) {
    if let Some(d) = n.source_detail {
        payload.insert("source_detail".into(), json!(d));
    }
    if let Some(w) = n.why {
        payload.insert("why".into(), json!(w));
    }
}

/// The twin suffix: `""` when both are absent, else ` — from {detail}` and/or ` — why: {why}`.
/// Word order is frozen (twin = wire).
pub fn source_note_twin(n: &SourceNote) -> String {
    let mut s = String::new();
    if let Some(d) = n.source_detail {
        s.push_str(&format!(" — from {d}"));
    }
    if let Some(w) = n.why {
        s.push_str(&format!(" — why: {w}"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::medication::assert::{
        medication_assertion_body, render_medication_twin, MedicationAssertion,
    };

    fn bare() -> MedicationAssertion<'static> {
        MedicationAssertion {
            medication_id: "11111111-1111-7111-8111-111111111111",
            term: "Lipitor",
            coding: None,
            formulation: Some("tablet"),
            dose_amount: Some("40"),
            dose_unit: Some("mg"),
            sig: Some("one BD"),
            info_source: "patient-reported",
            started: Some("2024"),
            started_precision: Some("year"),
            safety: None,
            charting: None,
            source_note: None,
        }
    }

    #[test]
    fn an_assertion_without_a_source_note_is_byte_identical() {
        // Written out by hand so a change to the old shape cannot hide behind the builder.
        let expected = json!({
            "medication_id": "11111111-1111-7111-8111-111111111111",
            "substance": { "term": "Lipitor", "formulation": "tablet" },
            "info_source": "patient-reported",
            "dose": { "amount": "40", "unit": "mg" },
            "sig": "one BD",
            "started": { "value": "2024", "precision": "year" },
        });
        let got = medication_assertion_body(&bare());
        assert_eq!(
            serde_json::to_vec(&got).unwrap(),
            serde_json::to_vec(&expected).unwrap()
        );
        // An all-None note writes nothing either.
        let mut a = bare();
        a.source_note = Some(SourceNote::default());
        assert_eq!(medication_assertion_body(&a), expected);
    }

    #[test]
    fn source_note_keys_are_exactly_source_detail_and_why() {
        let mut a = bare();
        a.source_note = Some(SourceNote {
            source_detail: Some("GP letter"),
            why: Some("on the packet"),
        });
        let v = medication_assertion_body(&a);
        assert_eq!(v["source_detail"], "GP letter");
        assert_eq!(v["why"], "on the packet");
    }

    #[test]
    fn a_none_key_is_absent_not_null() {
        let mut a = bare();
        a.source_note = Some(SourceNote {
            source_detail: Some("GP letter"),
            why: None,
        });
        let v = medication_assertion_body(&a);
        assert!(v.as_object().unwrap().contains_key("source_detail"));
        assert!(!v.as_object().unwrap().contains_key("why"));
    }

    #[test]
    fn the_assertion_twin_names_the_source_and_the_why() {
        let plain = render_medication_twin(&bare());
        let mut a = bare();
        a.source_note = Some(SourceNote {
            source_detail: Some("GP letter"),
            why: Some("on the packet"),
        });
        assert_eq!(
            render_medication_twin(&a),
            format!("{plain} — from GP letter — why: on the packet")
        );
    }
}
