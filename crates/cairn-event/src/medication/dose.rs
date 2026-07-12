//! Slice-2 dose *change* / *correction* builders. Pure: shapes only payload JSON.
//! A change is additive (both doses true over effective time); a correction says a
//! recorded dose was wrong and references (`corrects`) the dose event it fixes.
//! Dose fields are honest-unknown (principle 4): a change with no amount ("upped it,
//! dunno to what") and a correction to unknown ("that 40 was a guess, strike it")
//! are both first-class.
use serde_json::{json, Value};

/// Build the `dose` sub-object from optional amount/unit; None when both absent
/// (so the key is omitted entirely — honest-unknown, never serialized as null).
fn dose_object(amount: Option<&str>, unit: Option<&str>) -> Option<Value> {
    if amount.is_none() && unit.is_none() {
        return None;
    }
    let mut d = serde_json::Map::new();
    if let Some(a) = amount {
        d.insert("amount".into(), json!(a));
    }
    if let Some(u) = unit {
        d.insert("unit".into(), json!(u));
    }
    Some(Value::Object(d))
}

pub struct DoseChange<'a> {
    pub medication_id: &'a str,
    pub dose_amount: Option<&'a str>,
    pub dose_unit: Option<&'a str>,
    pub effective: Option<&'a str>,
    pub effective_precision: Option<&'a str>,
    pub info_source: &'a str,
    pub reason: Option<&'a str>,
}

pub struct DoseCorrection<'a> {
    pub medication_id: &'a str,
    pub corrects: &'a str,
    pub dose_amount: Option<&'a str>,
    pub dose_unit: Option<&'a str>,
    pub info_source: Option<&'a str>,
    pub reason: Option<&'a str>,
}

/// Build the `clinical.medication-dose-change.asserted` payload. `dose` and
/// `effective` are inserted only when present (honest-unknown).
pub fn dose_change_body(d: &DoseChange) -> Value {
    let mut p = json!({
        "medication_id": d.medication_id,
        "info_source": d.info_source,
    });
    let obj = p.as_object_mut().expect("json! built an object");
    if let Some(dose) = dose_object(d.dose_amount, d.dose_unit) {
        obj.insert("dose".into(), dose);
    }
    if let Some(v) = d.effective {
        let mut eff = json!({ "value": v });
        if let Some(pr) = d.effective_precision {
            eff.as_object_mut()
                .expect("json! built an object")
                .insert("precision".into(), json!(pr));
        }
        obj.insert("effective".into(), eff);
    }
    if let Some(r) = d.reason {
        obj.insert("reason".into(), json!(r));
    }
    p
}

/// The §3.13 legibility twin for a dose change. Always non-empty.
pub fn render_dose_change_twin(d: &DoseChange) -> String {
    let mut s = String::from("Dose changed");
    match (d.dose_amount, d.dose_unit) {
        (Some(a), Some(u)) => s.push_str(&format!(" to {a} {u}")),
        (Some(a), None) => s.push_str(&format!(" to {a}")),
        (None, Some(u)) => s.push_str(&format!(" to {u}")),
        (None, None) => {}
    }
    if let Some(v) = d.effective {
        s.push_str(&format!(" (effective {v})"));
    }
    if let Some(r) = d.reason {
        s.push_str(&format!(" — {r}"));
    }
    s.push_str(&format!(" ({})", d.info_source));
    s
}

/// Build the `clinical.medication-dose-correction.asserted` payload. `dose` omitted
/// entirely when both parts are absent (correct-to-unknown).
pub fn dose_correction_body(d: &DoseCorrection) -> Value {
    let mut p = json!({
        "medication_id": d.medication_id,
        "corrects": d.corrects,
    });
    let obj = p.as_object_mut().expect("json! built an object");
    if let Some(dose) = dose_object(d.dose_amount, d.dose_unit) {
        obj.insert("dose".into(), dose);
    }
    if let Some(src) = d.info_source {
        obj.insert("info_source".into(), json!(src));
    }
    if let Some(r) = d.reason {
        obj.insert("reason".into(), json!(r));
    }
    p
}

/// The §3.13 legibility twin for a dose correction. States the corrected value (the
/// old value lives on the corrected event). Always non-empty.
pub fn render_dose_correction_twin(d: &DoseCorrection) -> String {
    let mut s = String::from("Dose corrected");
    match (d.dose_amount, d.dose_unit) {
        (Some(a), Some(u)) => s.push_str(&format!(" to {a} {u}")),
        (Some(a), None) => s.push_str(&format!(" to {a}")),
        (None, Some(u)) => s.push_str(&format!(" to {u}")),
        (None, None) => s.push_str(" to unknown"),
    }
    if let Some(r) = d.reason {
        s.push_str(&format!(" — {r}"));
    }
    if let Some(src) = d.info_source {
        s.push_str(&format!(" ({src})"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_change() -> DoseChange<'static> {
        DoseChange {
            medication_id: "11111111-1111-7111-8111-111111111111",
            dose_amount: Some("80"),
            dose_unit: Some("mg"),
            effective: Some("2025-06"),
            effective_precision: Some("month"),
            info_source: "clinician-observed",
            reason: Some("titration"),
        }
    }

    #[test]
    fn change_body_carries_all_present_fields() {
        let v = dose_change_body(&full_change());
        assert_eq!(v["medication_id"], "11111111-1111-7111-8111-111111111111");
        assert_eq!(v["dose"]["amount"], "80");
        assert_eq!(v["dose"]["unit"], "mg");
        assert_eq!(v["effective"]["value"], "2025-06");
        assert_eq!(v["effective"]["precision"], "month");
        assert_eq!(v["info_source"], "clinician-observed");
        assert_eq!(v["reason"], "titration");
    }

    #[test]
    fn change_body_unknown_amount_is_first_class() {
        // "they upped my metformin, don't know to what" — a known change, unknown target.
        let c = DoseChange {
            medication_id: "22222222-2222-7222-8222-222222222222",
            dose_amount: None,
            dose_unit: None,
            effective: Some("2025"),
            effective_precision: Some("year"),
            info_source: "patient-reported",
            reason: None,
        };
        let v = dose_change_body(&c);
        assert!(
            !v.as_object().unwrap().contains_key("dose"),
            "absent dose omitted entirely, not null"
        );
        assert_eq!(v["effective"]["value"], "2025");
        assert_eq!(v["info_source"], "patient-reported");
    }

    #[test]
    fn change_twin_is_nonempty_and_reads_naturally() {
        let s = render_dose_change_twin(&full_change());
        assert!(s.contains("80 mg"));
        assert!(s.contains("2025-06"));
        assert!(s.contains("titration"));
        assert!(s.contains("clinician-observed"));
        assert!(!s.trim().is_empty());
    }

    #[test]
    fn change_twin_nonempty_when_amount_unknown() {
        let c = DoseChange {
            medication_id: "22222222-2222-7222-8222-222222222222",
            dose_amount: None,
            dose_unit: None,
            effective: Some("2025"),
            effective_precision: Some("year"),
            info_source: "patient-reported",
            reason: None,
        };
        assert!(!render_dose_change_twin(&c).trim().is_empty());
    }

    #[test]
    fn correction_body_carries_and_omits_correctly() {
        let full = DoseCorrection {
            medication_id: "11111111-1111-7111-8111-111111111111",
            corrects: "33333333-3333-7333-8333-333333333333",
            dose_amount: Some("20"),
            dose_unit: Some("mg"),
            info_source: Some("clinician-observed"),
            reason: Some("mis-keyed"),
        };
        let v = dose_correction_body(&full);
        assert_eq!(v["corrects"], "33333333-3333-7333-8333-333333333333");
        assert_eq!(v["dose"]["amount"], "20");
        assert_eq!(v["reason"], "mis-keyed");
        assert_eq!(v["info_source"], "clinician-observed");
    }

    #[test]
    fn correction_to_unknown_omits_dose() {
        // "the 40 I typed was a guess — strike it, dose unknown" (principle 4).
        let c = DoseCorrection {
            medication_id: "11111111-1111-7111-8111-111111111111",
            corrects: "33333333-3333-7333-8333-333333333333",
            dose_amount: None,
            dose_unit: None,
            info_source: None,
            reason: Some("was a guess"),
        };
        let v = dose_correction_body(&c);
        assert!(
            !v.as_object().unwrap().contains_key("dose"),
            "correct-to-unknown carries no dose key"
        );
        assert!(!v.as_object().unwrap().contains_key("info_source"));
        assert_eq!(v["corrects"], "33333333-3333-7333-8333-333333333333");
    }

    #[test]
    fn correction_twin_nonempty_including_to_unknown() {
        let full = DoseCorrection {
            medication_id: "11111111-1111-7111-8111-111111111111",
            corrects: "33333333-3333-7333-8333-333333333333",
            dose_amount: Some("20"),
            dose_unit: Some("mg"),
            info_source: None,
            reason: Some("mis-keyed"),
        };
        let s = render_dose_correction_twin(&full);
        assert!(s.contains("20 mg"));
        assert!(s.contains("mis-keyed"));

        let unknown = DoseCorrection {
            medication_id: "11111111-1111-7111-8111-111111111111",
            corrects: "33333333-3333-7333-8333-333333333333",
            dose_amount: None,
            dose_unit: None,
            info_source: None,
            reason: None,
        };
        assert!(!render_dose_correction_twin(&unknown).trim().is_empty());
    }

    #[test]
    fn correction_twin_surfaces_info_source_when_present() {
        let c = DoseCorrection {
            medication_id: "11111111-1111-7111-8111-111111111111",
            corrects: "33333333-3333-7333-8333-333333333333",
            dose_amount: Some("20"),
            dose_unit: Some("mg"),
            info_source: Some("clinician-observed"),
            reason: None,
        };
        let s = render_dose_correction_twin(&c);
        assert!(s.contains("20 mg"));
        assert!(
            s.contains("clinician-observed"),
            "correction twin must surface info_source, got: {s}"
        );
    }
}
