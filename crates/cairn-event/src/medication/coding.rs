//! Slice-6b coding *overlay* builders — coding as a separately-authored act (ADR-0059
//! decision 3). Pure: shapes only payload JSON, no clock, no randomness, no I/O.
//!
//! A medication may be coded inline on the assertion (slice 6a, `assert.rs`) or later by
//! whoever codes it — a pharmacist or a professional coder, as a distinct contributor
//! whose coding claim never overwrites the clinician's clinical claim. A correction either
//! replaces the claim or STRIKES it: append-only means the correction event is the only
//! repair path, so it must be able to say *"not that, and I don't know what it is"* —
//! otherwise a reviewer who disproves a coding can only leave the wrong anchor standing
//! (it keeps feeding the dup-key and the group display) or invent a substitute identity
//! they cannot vouch for, which is the fabrication principle 4 forbids.
use super::SubstanceCoding;
use serde_json::{json, Value};

/// Code a medication thread that was not coded inline.
pub struct MedicationCoding<'a> {
    /// The immortal thread id being coded.
    pub medication_id: &'a str,
    /// The drug-identity claim.
    pub coding: SubstanceCoding<'a>,
}

/// Correct a coding claim — replace it, or strike it back to not-yet-coded.
///
/// Exactly one of `coding` / `strike` is meaningful; the in-DB floor refuses both-present
/// and neither-present (it is the unbypassable enforcement, principle 12). The strike is
/// EXPLICIT rather than inferred from an absent coding, so a caller who simply forgets the
/// coding gets a refusal instead of silently un-coding a medication.
pub struct MedicationCodingCorrection<'a> {
    /// The immortal thread id whose coding is being corrected.
    pub medication_id: &'a str,
    /// The event whose coding claim this fixes — a prior coding overlay, or the assertion
    /// itself when the coding was inline. Existence is NOT required anywhere: the
    /// corrected event may replicate later, or never (offline-first; the same contract
    /// `clinical.medication-dose-correction.asserted` already carries).
    pub corrects: &'a str,
    /// The replacement claim. `None` together with `strike` = strike to not-yet-coded.
    pub coding: Option<SubstanceCoding<'a>>,
    /// Strike the coding back to honest not-yet-coded.
    pub strike: bool,
    /// Why THIS correction was made (audit) — distinct from any clinical reason.
    /// `None` omits the key rather than writing null (principle 11).
    pub note: Option<&'a str>,
}

/// Serialize a coding triple as its wire object. One definition, so the inline and
/// overlay paths cannot drift in field naming or order.
fn coding_object(c: &SubstanceCoding) -> Value {
    json!({ "system": c.system, "code": c.code, "display": c.display })
}

/// Build the `clinical.medication-coding.asserted` payload.
pub fn medication_coding_body(c: &MedicationCoding) -> Value {
    json!({
        "medication_id": c.medication_id,
        "coding": coding_object(&c.coding),
    })
}

/// Build the `clinical.medication-coding-correction.asserted` payload. Optional keys are
/// inserted only when present — never serialized as null (principle 11: an added-later
/// field must not change an existing event's content address).
pub fn medication_coding_correction_body(c: &MedicationCodingCorrection) -> Value {
    let mut p = json!({
        "medication_id": c.medication_id,
        "corrects": c.corrects,
    });
    let obj = p.as_object_mut().expect("json! built an object");
    if let Some(coding) = c.coding {
        obj.insert("coding".into(), coding_object(&coding));
    } else if c.strike {
        obj.insert("strike".into(), json!(true));
    }
    if let Some(n) = c.note {
        obj.insert("note".into(), json!(n));
    }
    p
}

/// The §3.13 legibility twin for a coding overlay. Non-empty by construction: the
/// display and system are both floor-mandated non-empty strings.
pub fn render_medication_coding_twin(c: &MedicationCoding) -> String {
    format!("coded as {} [{}]", c.coding.display, c.coding.system)
}

/// The §3.13 legibility twin for a coding correction — a reader holding no drug database
/// at all must still be able to tell a replacement from a retraction.
pub fn render_medication_coding_correction_twin(c: &MedicationCodingCorrection) -> String {
    let head = match c.coding {
        Some(k) => format!("coding corrected to {} [{}]", k.display, k.system),
        None => "coding struck — no longer coded".to_string(),
    };
    match c.note {
        Some(n) => format!("{head} — {n}"),
        None => head,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MOIETY: &str = "0f8c4b1e-1b7a-5c2d-9a3e-2b6f7c8d9e01";
    const MED: &str = "11111111-1111-7111-8111-111111111111";
    const TARGET: &str = "22222222-2222-7222-8222-222222222222";

    fn coding() -> SubstanceCoding<'static> {
        SubstanceCoding {
            system: "drugref-moiety",
            code: MOIETY,
            display: "atorvastatin",
        }
    }

    #[test]
    fn coding_body_carries_the_thread_and_the_triple() {
        let v = medication_coding_body(&MedicationCoding {
            medication_id: MED,
            coding: coding(),
        });
        assert_eq!(v["medication_id"], MED);
        assert_eq!(v["coding"]["system"], "drugref-moiety");
        assert_eq!(v["coding"]["code"], MOIETY);
        assert_eq!(v["coding"]["display"], "atorvastatin");
    }

    #[test]
    fn correction_body_with_a_replacement_carries_no_strike_key() {
        let v = medication_coding_correction_body(&MedicationCodingCorrection {
            medication_id: MED,
            corrects: TARGET,
            coding: Some(coding()),
            strike: false,
            note: Some("brand name was ambiguous"),
        });
        assert_eq!(v["corrects"], TARGET);
        assert_eq!(v["coding"]["display"], "atorvastatin");
        assert_eq!(v["note"], "brand name was ambiguous");
        assert!(
            !v.as_object().unwrap().contains_key("strike"),
            "a replacement must not also claim a strike"
        );
    }

    #[test]
    fn correction_body_with_a_strike_carries_no_coding_key() {
        let v = medication_coding_correction_body(&MedicationCodingCorrection {
            medication_id: MED,
            corrects: TARGET,
            coding: None,
            strike: true,
            note: Some("not metformin; substance unidentified"),
        });
        assert_eq!(v["strike"], true);
        assert!(
            !v.as_object().unwrap().contains_key("coding"),
            "a strike must not also carry a coding"
        );
    }

    #[test]
    fn correction_body_omits_an_absent_note() {
        let v = medication_coding_correction_body(&MedicationCodingCorrection {
            medication_id: MED,
            corrects: TARGET,
            coding: None,
            strike: true,
            note: None,
        });
        assert!(
            !v.as_object().unwrap().contains_key("note"),
            "absent note must be omitted, not null"
        );
    }

    #[test]
    fn coding_twin_names_the_substance_and_the_system() {
        let s = render_medication_coding_twin(&MedicationCoding {
            medication_id: MED,
            coding: coding(),
        });
        assert_eq!(s, "coded as atorvastatin [drugref-moiety]");
    }

    #[test]
    fn correction_twins_distinguish_replacement_from_strike() {
        let replaced = render_medication_coding_correction_twin(&MedicationCodingCorrection {
            medication_id: MED,
            corrects: TARGET,
            coding: Some(coding()),
            strike: false,
            note: Some("brand name was ambiguous"),
        });
        assert_eq!(
            replaced,
            "coding corrected to atorvastatin [drugref-moiety] — brand name was ambiguous"
        );

        let struck = render_medication_coding_correction_twin(&MedicationCodingCorrection {
            medication_id: MED,
            corrects: TARGET,
            coding: None,
            strike: true,
            note: None,
        });
        assert_eq!(struck, "coding struck — no longer coded");
    }
}
