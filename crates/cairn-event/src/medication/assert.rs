//! Medication *assertion* builder (the "start" verb) — mints a medication thread.
//! Pure: no clock, no randomness, no I/O. Optional fields are inserted only when
//! present (never serialized as null), so an added-later field never changes an
//! existing event's content address (principle 11).
use serde_json::{json, Value};

/// A medication statement (the "start" verb). `term` is the one mandatory
/// clinical field (may be vague, e.g. "little white pill"); every `Option`
/// field is omitted from the payload when `None`.
pub struct MedicationAssertion<'a> {
    /// Immortal thread id the caller mints; a later cessation references it.
    pub medication_id: &'a str,
    /// As-asserted substance term — mandatory, non-empty.
    pub term: &'a str,
    /// Stable INN anchor; `None` = not-yet-coded (usual in slice 1, no dictionary).
    pub inn_code: Option<&'a str>,
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
        if let Some(c) = a.inn_code {
            s.insert("inn_code".into(), json!(c));
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
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_assertion() -> MedicationAssertion<'static> {
        MedicationAssertion {
            medication_id: "11111111-1111-7111-8111-111111111111",
            term: "atorvastatin",
            inn_code: Some("INN:atorvastatin"),
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
    fn assertion_body_carries_all_present_fields() {
        let v = medication_assertion_body(&full_assertion());
        assert_eq!(v["medication_id"], "11111111-1111-7111-8111-111111111111");
        assert_eq!(v["substance"]["term"], "atorvastatin");
        assert_eq!(v["substance"]["inn_code"], "INN:atorvastatin");
        assert_eq!(v["substance"]["formulation"], "tablet");
        assert_eq!(v["dose"]["amount"], "40");
        assert_eq!(v["dose"]["unit"], "mg");
        assert_eq!(v["sig"], "one BD");
        assert_eq!(v["info_source"], "patient-reported");
        assert_eq!(v["started"]["value"], "2024");
        assert_eq!(v["started"]["precision"], "year");
    }

    #[test]
    fn assertion_body_omits_absent_optionals_never_null() {
        // The "little white pill, don't know anything else" floor case.
        let a = MedicationAssertion {
            medication_id: "22222222-2222-7222-8222-222222222222",
            term: "little white pill",
            inn_code: None,
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
            !subst.contains_key("inn_code"),
            "absent inn_code must be omitted, not null"
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
    fn assertion_twin_nonempty_for_vague_term_only() {
        let a = MedicationAssertion {
            medication_id: "22222222-2222-7222-8222-222222222222",
            term: "little white pill",
            inn_code: None,
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
