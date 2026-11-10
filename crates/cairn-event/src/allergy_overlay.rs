//! The allergy overlay bodies (allergy slice 2): a de-label-by-challenge and an
//! entered-in-error withdrawal. Pure: no clock, no randomness, no I/O. Keys and twin wording
//! are WIRE, frozen once merged (principle 11). A correction is not here: it is an ordinary
//! `clinical.allergy/1` entry carrying a `correction-of` ref (see `allergy::AllergyEntry`).
//!
//! A blank (whitespace-only) `text` is no text at all: it is written as JSON null in the
//! payload AND yields no tail in the twin, so a sealed body never carries a value its own twin
//! calls absent (one rule, `present`). Non-blank text is kept verbatim in the payload.
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

/// The one blank test: free text that is empty or whitespace-only is absent.
fn present(text: Option<&str>) -> Option<&str> {
    text.filter(|t| !t.trim().is_empty())
}

/// The twin's `" — {text}"` tail, empty when the text is absent.
fn tail(text: Option<&str>) -> String {
    present(text)
        .map(|t| format!(" — {}", t.trim()))
        .unwrap_or_default()
}

/// Build the `clinical.allergy-challenge/1` payload. `tolerated.coding` is a reserved slot, kept null
/// by allergy slice 3 (ADR-0098 decision 3; opening it is #1053).
pub fn allergy_challenge_payload(c: &AllergyChallenge) -> Value {
    let (precision, value) = onset_wire(&c.date);
    json!({
        "target": c.target,
        "tolerated": {"term": c.tolerated, "coding": null},
        "date": {"precision": precision, "value": value},
        "text": present(c.text),
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
    json!({"target": r.target, "text": present(r.text)})
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
        assert_eq!(
            render_allergy_challenge_twin(&challenge(Onset::NotKnown, Some("  "))),
            "De-labelled by challenge: allergy record E1 — tolerated amoxicillin"
        );
        let blank = AllergyRetraction {
            target: "E1",
            text: Some("  "),
        };
        assert_eq!(
            render_allergy_retraction_twin(&blank),
            "Entered in error: withdraws allergy record E1"
        );
    }

    #[test]
    fn blank_text_is_null_in_payload() {
        let c = allergy_challenge_payload(&challenge(Onset::NotKnown, Some("  ")));
        assert_eq!(c["text"], Value::Null);
        let r = allergy_retraction_payload(&AllergyRetraction {
            target: "E1",
            text: Some(" \t"),
        });
        assert_eq!(r["text"], Value::Null);
    }

    #[test]
    fn day_and_year_dates_and_absent_retraction_text() {
        let d = allergy_challenge_payload(&challenge(Onset::Day("2022-03-04".into()), None));
        assert_eq!(
            d["date"],
            json!({"precision": "day", "value": "2022-03-04"})
        );
        let y = allergy_challenge_payload(&challenge(Onset::Year("2022".into()), None));
        assert_eq!(y["date"], json!({"precision": "year", "value": "2022"}));
        let r = allergy_retraction_payload(&AllergyRetraction {
            target: "E1",
            text: None,
        });
        assert_eq!(r, json!({"target": "E1", "text": null}));
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
