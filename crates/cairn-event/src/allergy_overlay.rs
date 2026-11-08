//! The allergy overlay bodies (allergy slice 2): a de-label-by-challenge and an
//! entered-in-error withdrawal. Pure: no clock, no randomness, no I/O. Keys and twin wording
//! are WIRE, frozen once merged (principle 11). A correction is not here: it is an ordinary
//! `clinical.allergy/1` entry carrying a `correction-of` ref (see `allergy::AllergyEntry`).
//!
//! A blank (whitespace-only) `text` yields no tail in the twin, as a blank reaction text does
//! in `entry_words_dated`; the payload still carries exactly what the caller gave.
use crate::allergy::{onset_wire, Onset};
use serde_json::{json, Value};

/// Event type of a de-label-by-challenge.
pub const ALLERGY_CHALLENGE_EVENT_TYPE: &str = "clinical.allergy-challenge.asserted";
/// Payload schema tag of a challenge.
pub const ALLERGY_CHALLENGE_SCHEMA_VERSION: &str = "clinical.allergy-challenge/1";
/// Event type of an entered-in-error withdrawal.
pub const ALLERGY_RETRACTION_EVENT_TYPE: &str = "clinical.allergy-retraction.asserted";
/// Payload schema tag of a withdrawal.
pub const ALLERGY_RETRACTION_SCHEMA_VERSION: &str = "clinical.allergy-retraction/1";

/// "The patient tolerated this substance on this date": de-labels the target allergy record.
pub struct AllergyChallenge<'a> {
    pub target: &'a str,
    pub tolerated: &'a str,
    pub date: Onset,
    pub text: Option<&'a str>,
}

/// "That allergy record was entered in error."
pub struct AllergyRetraction<'a> {
    pub target: &'a str,
    pub text: Option<&'a str>,
}

/// Free text with blanks treated as absent (twin only).
fn tail(text: Option<&str>) -> String {
    text.map(str::trim)
        .filter(|t| !t.is_empty())
        .map(|t| format!(" — {t}"))
        .unwrap_or_default()
}

/// Build the `clinical.allergy-challenge/1` payload. `tolerated.coding` is a reserved slot.
pub fn allergy_challenge_payload(c: &AllergyChallenge) -> Value {
    let (precision, value) = onset_wire(&c.date);
    json!({
        "target": c.target,
        "tolerated": {"term": c.tolerated, "coding": null},
        "date": {"precision": precision, "value": value},
        "text": c.text,
    })
}

/// The signed twin of a challenge; the date is ISO and omitted when not known (R17).
pub fn render_allergy_challenge_twin(c: &AllergyChallenge) -> String {
    let date = match onset_wire(&c.date).1 {
        Some(v) => format!(" · {v}"),
        None => String::new(),
    };
    format!(
        "De-labelled by challenge: allergy record {} — tolerated {}{date}{}",
        c.target,
        c.tolerated,
        tail(c.text)
    )
}

/// Build the `clinical.allergy-retraction/1` payload.
pub fn allergy_retraction_payload(r: &AllergyRetraction) -> Value {
    json!({"target": r.target, "text": r.text})
}

/// The signed twin of a withdrawal.
pub fn render_allergy_retraction_twin(r: &AllergyRetraction) -> String {
    format!(
        "Entered in error: withdraws allergy record {}{}",
        r.target,
        tail(r.text)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn challenge(date: Onset, text: Option<&'static str>) -> AllergyChallenge<'static> {
        AllergyChallenge {
            target: "E1",
            tolerated: "amoxicillin",
            date,
            text,
        }
    }

    #[test]
    fn challenge_payload_has_exactly_the_frozen_keys() {
        let p = allergy_challenge_payload(&challenge(Onset::Month("2022-03".into()), None));
        let mut keys: Vec<_> = p.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, ["date", "target", "text", "tolerated"]);
        assert_eq!(
            p["tolerated"],
            json!({"term": "amoxicillin", "coding": null})
        );
        assert_eq!(p["date"], json!({"precision": "month", "value": "2022-03"}));
        assert_eq!(p["text"], Value::Null);
        let n = allergy_challenge_payload(&challenge(Onset::NotKnown, Some("x")));
        assert_eq!(n["date"], json!({"precision": "not-known", "value": null}));
    }

    #[test]
    fn challenge_twin_words_are_exact() {
        assert_eq!(
            render_allergy_challenge_twin(&challenge(
                Onset::Month("2022-03".into()),
                Some("oral challenge")
            )),
            "De-labelled by challenge: allergy record E1 — tolerated amoxicillin · 2022-03 — oral challenge"
        );
        assert_eq!(
            render_allergy_challenge_twin(&challenge(Onset::NotKnown, None)),
            "De-labelled by challenge: allergy record E1 — tolerated amoxicillin"
        );
        assert!(
            !render_allergy_challenge_twin(&challenge(Onset::NotKnown, Some("  ")))
                .ends_with(" — ")
        );
    }

    #[test]
    fn retraction_payload_and_twin() {
        let r = AllergyRetraction {
            target: "E1",
            text: Some("wrong patient"),
        };
        assert_eq!(
            allergy_retraction_payload(&r),
            json!({"target": "E1", "text": "wrong patient"})
        );
        assert_eq!(
            render_allergy_retraction_twin(&r),
            "Entered in error: withdraws allergy record E1 — wrong patient"
        );
        let n = AllergyRetraction {
            target: "E1",
            text: None,
        };
        assert_eq!(
            render_allergy_retraction_twin(&n),
            "Entered in error: withdraws allergy record E1"
        );
    }
}
