//! Medication *assertion* builder (the "start" verb) — mints a medication thread.
//! Pure: no clock, no randomness, no I/O. Optional fields are inserted only when
//! present (never serialized as null), so an added-later field never changes an
//! existing event's content address (principle 11).
use serde_json::{json, Value};

/// A drug-identity coding claim, captured at coding time (ADR-0059).
///
/// The anchor is an *immortal identifier* — drugref's `moiety_uuid` — never a name:
/// keying on a label (even an INN) repeats the founding wound (principle 2). All three
/// fields travel together because `display` is the honest-degradation label: a node
/// without drugref still shows the preferred name, so it is never optional *within*
/// the object. The object as a whole stays optional — uncoded is first-class.
///
/// `Debug` is derived so callers (the Task 2 `coding_from_parts` tests, in particular)
/// can use `expect_err` on a `Result<Option<SubstanceCoding>, _>` — Rust requires the Ok
/// side to be `Debug` even though the assertion only ever inspects the `Err`.
/// `Clone`/`Copy` are derived because this holds nothing but borrowed `&str`s (like the
/// `inn_code: Option<&str>` slot it replaces, which was `Copy` too) — callers build test
/// fixtures with `..base` struct-update syntax reused across several literals, which
/// needs every field to be `Copy`, not moved-from-under the original on first use.
#[derive(Debug, Clone, Copy)]
pub struct SubstanceCoding<'a> {
    /// The drugref composition-tree level. `drugref-moiety` today; the finer
    /// `drugref-clinical-drug` / `drugref-product` levels are reserved.
    pub system: &'a str,
    /// The immortal identifier itself (a `moiety_uuid`, UUIDv5 from the UNII).
    pub code: &'a str,
    /// The INN-preferred label as it read at coding time.
    pub display: &'a str,
}

/// A medication statement (the "start" verb). `term` is the one mandatory
/// clinical field (may be vague, e.g. "little white pill"); every `Option`
/// field is omitted from the payload when `None`.
pub struct MedicationAssertion<'a> {
    /// Immortal thread id the caller mints; a later cessation references it.
    pub medication_id: &'a str,
    /// As-asserted substance term — mandatory, non-empty.
    pub term: &'a str,
    /// Drug-identity coding, when someone has coded it; `None` = not-yet-coded, which
    /// is a permanently valid state (the "little white pill" floor, principle 4).
    pub coding: Option<SubstanceCoding<'a>>,
    /// Formulation enum token (tablet, capsule, liquid, patch, …) or `None` = unknown.
    pub formulation: Option<&'a str>,
    /// Dose magnitude as a decimal string; `None` = unknown.
    pub dose_amount: Option<&'a str>,
    /// Dose unit (a small controlled token or a free-text long-tail value); `None` = unknown.
    pub dose_unit: Option<&'a str>,
    /// Free-text directions ("one BD", "PRN"); `None` = unknown.
    pub sig: Option<&'a str>,
    /// Provenance of the *claim* (who said it) — distinct from event authorship.
    /// Required-present, value-open: patient-reported|clinician-observed|external-record|unknown.
    pub info_source: &'a str,
    /// Uncertainty-capable start date value ("2024", "2024-03", "2020/2024"); `None` = unknown.
    pub started: Option<&'a str>,
    /// Precision token for `started` (year|month|day|year-range); only meaningful when `started` is Some.
    pub started_precision: Option<&'a str>,
}

/// Build the `clinical.medication.asserted` payload. Mirrors the demographics
/// `*_body` idiom: a `json!` skeleton of the always-present fields, then optional
/// keys inserted only when `Some`.
pub fn medication_assertion_body(a: &MedicationAssertion) -> Value {
    let mut substance = json!({ "term": a.term });
    {
        let s = substance.as_object_mut().expect("json! built an object");
        if let Some(c) = a.coding {
            s.insert(
                "coding".into(),
                json!({ "system": c.system, "code": c.code, "display": c.display }),
            );
        }
        if let Some(f) = a.formulation {
            s.insert("formulation".into(), json!(f));
        }
    }
    let mut p = json!({
        "medication_id": a.medication_id,
        "substance": substance,
        "info_source": a.info_source,
    });
    let obj = p.as_object_mut().expect("json! built an object");
    if a.dose_amount.is_some() || a.dose_unit.is_some() {
        let mut dose = json!({});
        let d = dose.as_object_mut().expect("json! built an object");
        if let Some(amt) = a.dose_amount {
            d.insert("amount".into(), json!(amt));
        }
        if let Some(u) = a.dose_unit {
            d.insert("unit".into(), json!(u));
        }
        obj.insert("dose".into(), dose);
    }
    if let Some(s) = a.sig {
        obj.insert("sig".into(), json!(s));
    }
    if let Some(v) = a.started {
        let mut started = json!({ "value": v });
        if let Some(pr) = a.started_precision {
            started
                .as_object_mut()
                .expect("json! built an object")
                .insert("precision".into(), json!(pr));
        }
        obj.insert("started".into(), started);
    }
    p
}

/// The §3.13/§3.3 legibility twin for a medication statement — a mechanically
/// derived, honest one-line rendering. Non-empty because `term` is non-empty.
pub fn render_medication_twin(a: &MedicationAssertion) -> String {
    let mut s = String::from(a.term);
    match (a.dose_amount, a.dose_unit) {
        (Some(amt), Some(u)) => s.push_str(&format!(" {amt} {u}")),
        (Some(amt), None) => s.push_str(&format!(" {amt}")),
        (None, Some(u)) => s.push_str(&format!(" {u}")), // unit recorded without an amount (e.g. "puffs")
        (None, None) => {}
    }
    if let Some(f) = a.formulation {
        s.push_str(&format!(" {f}"));
    }
    if let Some(sig) = a.sig {
        s.push_str(&format!(" — {sig}"));
    }
    s.push_str(&format!(" ({})", a.info_source));
    if let Some(v) = a.started {
        s.push_str(&format!(", started {v}"));
    }
    // ADR-0059 / principle 11: the captured display is what a reader without drugref
    // still has. Repeat it only when it adds something — a clinician who typed the
    // generic name already wrote it (case-folded compare, so "Atorvastatin" counts).
    // SubstanceCoding is Copy, so `a.coding` reads out a value directly through the
    // `&MedicationAssertion` — no need to borrow it (house rule 4).
    if let Some(c) = a.coding {
        if !c.display.eq_ignore_ascii_case(a.term) {
            s.push_str(&format!(" [{}]", c.display));
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_assertion() -> MedicationAssertion<'static> {
        MedicationAssertion {
            medication_id: "11111111-1111-7111-8111-111111111111",
            term: "Lipitor",
            coding: Some(SubstanceCoding {
                system: "drugref-moiety",
                code: "0f8c4b1e-1b7a-5c2d-9a3e-2b6f7c8d9e01",
                display: "atorvastatin",
            }),
            formulation: Some("tablet"),
            dose_amount: Some("40"),
            dose_unit: Some("mg"),
            sig: Some("one BD"),
            info_source: "patient-reported",
            started: Some("2024"),
            started_precision: Some("year"),
        }
    }

    #[test]
    fn assertion_body_carries_the_coding_triple() {
        let v = medication_assertion_body(&full_assertion());
        assert_eq!(v["substance"]["term"], "Lipitor");
        assert_eq!(v["substance"]["coding"]["system"], "drugref-moiety");
        assert_eq!(
            v["substance"]["coding"]["code"],
            "0f8c4b1e-1b7a-5c2d-9a3e-2b6f7c8d9e01"
        );
        assert_eq!(v["substance"]["coding"]["display"], "atorvastatin");
    }

    #[test]
    fn assertion_body_carries_all_present_fields() {
        // Restores coverage dropped by Task 1's edits (#0059 review finding): every
        // present optional field must actually reach the payload, not just the coding
        // triple (which `assertion_body_carries_the_coding_triple` already covers).
        let v = medication_assertion_body(&full_assertion());
        assert_eq!(v["medication_id"], "11111111-1111-7111-8111-111111111111");
        assert_eq!(v["substance"]["formulation"], "tablet");
        assert_eq!(v["dose"]["amount"], "40");
        assert_eq!(v["dose"]["unit"], "mg");
        assert_eq!(v["sig"], "one BD");
        assert_eq!(v["started"]["value"], "2024");
        assert_eq!(v["started"]["precision"], "year");
    }

    #[test]
    fn assertion_body_omits_absent_coding_and_never_emits_the_retired_slot() {
        let mut a = full_assertion();
        a.coding = None;
        let v = medication_assertion_body(&a);
        let subst = v["substance"].as_object().unwrap();
        assert!(
            !subst.contains_key("coding"),
            "absent coding must be omitted, not null (principle 4: uncoded is first-class)"
        );
        assert!(
            !subst.contains_key("inn_code"),
            "the reserved inn_code slot is retired (ADR-0059 decision 2)"
        );
    }

    #[test]
    fn assertion_body_omits_absent_optionals_never_null() {
        // The "little white pill, don't know anything else" floor case.
        let a = MedicationAssertion {
            medication_id: "22222222-2222-7222-8222-222222222222",
            term: "little white pill",
            coding: None,
            formulation: None,
            dose_amount: None,
            dose_unit: None,
            sig: None,
            info_source: "patient-reported",
            started: None,
            started_precision: None,
        };
        let v = medication_assertion_body(&a);
        let subst = v["substance"].as_object().unwrap();
        assert!(
            !subst.contains_key("coding"),
            "absent coding must be omitted, not null"
        );
        assert!(!subst.contains_key("formulation"));
        let obj = v.as_object().unwrap();
        assert!(
            !obj.contains_key("dose"),
            "absent dose must be omitted entirely"
        );
        assert!(!obj.contains_key("sig"));
        assert!(!obj.contains_key("started"));
        assert_eq!(v["substance"]["term"], "little white pill");
        assert_eq!(v["info_source"], "patient-reported");
    }

    #[test]
    fn assertion_body_dose_amount_only_omits_unit() {
        let mut a = full_assertion();
        a.dose_unit = None;
        let v = medication_assertion_body(&a);
        assert_eq!(v["dose"]["amount"], "40");
        assert!(!v["dose"].as_object().unwrap().contains_key("unit"));
    }

    #[test]
    fn assertion_twin_is_nonempty_and_reads_naturally() {
        let s = render_medication_twin(&full_assertion());
        assert!(s.contains("atorvastatin"));
        assert!(s.contains("40 mg"));
        assert!(s.contains("(patient-reported)"));
        assert!(s.contains("started 2024"));
        assert!(!s.trim().is_empty());
    }

    #[test]
    fn twin_appends_the_display_when_it_differs_from_the_term() {
        let s = render_medication_twin(&full_assertion());
        assert!(s.starts_with("Lipitor"));
        assert!(
            s.ends_with("[atorvastatin]"),
            "the captured display is the honest-degradation label a drugref-less \
             reader still sees, got: {s}"
        );
    }

    #[test]
    fn twin_does_not_repeat_a_display_equal_to_the_term() {
        // Case-folded compare: the clinician typed the generic name the coding resolves to.
        let mut a = full_assertion();
        a.term = "Atorvastatin";
        let s = render_medication_twin(&a);
        assert!(
            !s.contains('['),
            "a display equal to the term (case-insensitively) must add nothing, got: {s}"
        );
    }

    #[test]
    fn twin_of_an_uncoded_assertion_is_unchanged() {
        let mut a = full_assertion();
        a.coding = None;
        let s = render_medication_twin(&a);
        assert!(!s.contains('['), "no coding, no bracket: {s}");
        assert!(s.starts_with("Lipitor"));
    }

    #[test]
    fn assertion_twin_nonempty_for_vague_term_only() {
        let a = MedicationAssertion {
            medication_id: "22222222-2222-7222-8222-222222222222",
            term: "little white pill",
            coding: None,
            formulation: None,
            dose_amount: None,
            dose_unit: None,
            sig: None,
            info_source: "patient-reported",
            started: None,
            started_precision: None,
        };
        let s = render_medication_twin(&a);
        assert!(s.starts_with("little white pill"));
        assert!(!s.trim().is_empty());
    }

    #[test]
    fn assertion_twin_renders_unit_without_amount() {
        // "comes in puffs, don't know how many" — the unit must survive into the twin,
        // matching that medication_assertion_body keeps dose.unit for this case.
        let mut a = full_assertion();
        a.dose_amount = None;
        let s = render_medication_twin(&a);
        assert!(
            s.contains("mg"),
            "unit must render even without an amount, got: {}",
            s
        );
    }
}
