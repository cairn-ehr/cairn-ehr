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
    /// The §5.9 precise safety claim, established PRE-SEAL by the node authoring the
    /// overlay (ADR-0063). Same seam and same reason as `MedicationAssertion::safety`: a
    /// coding OVERLAY is authored by whoever codes it — a pharmacist, a professional coder
    /// — and that node is again the one holding a coding authority. `None` when this
    /// deployment's class map has no row for the coding.
    pub safety: Option<crate::safety::PreciseSafety<'a>>,
}

/// What a correction claims: a replacement drug identity, or a retraction of the one on
/// record.
///
/// A two-field `(Option<SubstanceCoding>, bool)` could spell two shapes that mean nothing —
/// both at once, and neither — which the in-DB floor then had to refuse and this builder
/// had to silently normalize. An enum deletes both: there is no way to hand the builder a
/// correction that does not say exactly one thing. The floor keeps refusing the same two
/// wire shapes regardless (it is the unbypassable enforcement, principle 12, and a peer's
/// bytes never went through this type).
///
/// The strike stays EXPLICIT rather than inferred from an absent coding — a caller who
/// simply forgot the coding must get a refusal, not a silently un-coded medication.
///
/// `Debug` for the same reason `SubstanceCoding` derives it — `expect_err` on a
/// `Result<CodingClaim, _>` requires the Ok side to be `Debug`. `Clone`/`Copy` because it
/// holds nothing but borrowed `&str`s.
#[derive(Debug, Clone, Copy)]
pub enum CodingClaim<'a> {
    /// This is the drug, not the one on record.
    Replace(SubstanceCoding<'a>),
    /// It is NOT the drug on record, and I cannot say what it is — the acknowledged
    /// uncertainty principle 4 protects, and the alternative to inventing a substitute
    /// identity nobody can vouch for.
    Strike,
}

/// Correct a coding claim — replace it, or strike it back to not-yet-coded.
pub struct MedicationCodingCorrection<'a> {
    /// The immortal thread id whose coding is being corrected.
    pub medication_id: &'a str,
    /// The event whose coding claim this fixes — a prior coding overlay, or the assertion
    /// itself when the coding was inline. Existence is NOT required anywhere: the
    /// corrected event may replicate later, or never (offline-first; the same contract
    /// `clinical.medication-dose-correction.asserted` already carries).
    pub corrects: &'a str,
    /// What this correction claims — exactly one thing, by construction.
    pub claim: CodingClaim<'a>,
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
    let mut p = json!({
        "medication_id": c.medication_id,
        "coding": coding_object(&c.coding),
    });
    // Under the seal, never coarsened — see `medication_assertion_body`'s twin of this
    // block. Inserted only when present, so an overlay with no class is byte-identical to
    // the pre-ADR-0063 shape (principle 11: an added-later field must not change an
    // existing event's content address).
    if let Some(s) = c.safety {
        p.as_object_mut()
            .expect("json! built an object")
            .insert("safety".into(), crate::safety::precise_safety_body(&s));
    }
    p
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
    // Exhaustive over the claim, so exactly one of the two keys is ever written — the
    // enum is what makes that a fact rather than a convention.
    match c.claim {
        CodingClaim::Replace(k) => obj.insert("coding".into(), coding_object(&k)),
        CodingClaim::Strike => obj.insert("strike".into(), json!(true)),
    };
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
    let head = match c.claim {
        CodingClaim::Replace(k) => format!("coding corrected to {} [{}]", k.display, k.system),
        CodingClaim::Strike => "coding struck — no longer coded".to_string(),
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
            safety: None,
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
            claim: CodingClaim::Replace(coding()),
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
            claim: CodingClaim::Strike,
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
            claim: CodingClaim::Strike,
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
            safety: None,
        });
        assert_eq!(s, "coded as atorvastatin [drugref-moiety]");
    }

    #[test]
    fn correction_twins_distinguish_replacement_from_strike() {
        let replaced = render_medication_coding_correction_twin(&MedicationCodingCorrection {
            medication_id: MED,
            corrects: TARGET,
            claim: CodingClaim::Replace(coding()),
            note: Some("brand name was ambiguous"),
        });
        assert_eq!(
            replaced,
            "coding corrected to atorvastatin [drugref-moiety] — brand name was ambiguous"
        );

        let struck = render_medication_coding_correction_twin(&MedicationCodingCorrection {
            medication_id: MED,
            corrects: TARGET,
            claim: CodingClaim::Strike,
            note: None,
        });
        assert_eq!(struck, "coding struck — no longer coded");
    }
}
