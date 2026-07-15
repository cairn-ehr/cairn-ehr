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

/// Clinician-supplied fields of a dose correction — a PER-FIELD PATCH of a targeted
/// dose point. Each group (dose / effective / reason) is independently *set* (a Some /
/// non-empty value), *struck* (named in `strike` → set-unknown), or *kept* (absent).
/// `reason` is the point's clinical reason (why the dose is what it is); `note` is why
/// THIS correction was made (audit). See ADR-0050.
pub struct DoseCorrection<'a> {
    pub medication_id: &'a str,
    /// The dose event (`dose_event_id`) this correction targets.
    pub corrects: &'a str,
    /// Set the dose amount; paired with `dose_unit`. None = don't set via a value.
    pub dose_amount: Option<&'a str>,
    /// Set the dose unit. None = don't set via a value.
    pub dose_unit: Option<&'a str>,
    /// Set the point's effective date (value). None = don't touch effective.
    pub effective: Option<&'a str>,
    /// Precision token for `effective` (year|month|day|year-range); only with `effective`.
    pub effective_precision: Option<&'a str>,
    /// Set the point's clinical reason. None = don't touch reason.
    pub reason: Option<&'a str>,
    /// Groups to set-unknown — subset of {"dose","effective","reason"}. Empty = none.
    pub strike: &'a [&'a str],
    /// Why THIS correction was made (audit; always additive). None = omit.
    pub note: Option<&'a str>,
    /// Provenance of the correction claim. None = omit.
    pub info_source: Option<&'a str>,
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

/// Build the `clinical.medication-dose-correction.asserted` payload. Each optional
/// group is emitted only when set; `strike` only when non-empty; `note`/`info_source`
/// only when present (honest-omit, never null — principle 11).
pub fn dose_correction_body(d: &DoseCorrection) -> Value {
    let mut p = json!({
        "medication_id": d.medication_id,
        "corrects": d.corrects,
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
    if !d.strike.is_empty() {
        obj.insert("strike".into(), json!(d.strike));
    }
    if let Some(n) = d.note {
        obj.insert("note".into(), json!(n));
    }
    if let Some(src) = d.info_source {
        obj.insert("info_source".into(), json!(src));
    }
    p
}

/// The §3.13 legibility twin for a dose correction. Lists set fields, struck groups,
/// the clinical reason and the audit note. Always non-empty ("Dose correction" prefix).
pub fn render_dose_correction_twin(d: &DoseCorrection) -> String {
    let mut parts: Vec<String> = Vec::new();
    match (d.dose_amount, d.dose_unit) {
        (Some(a), Some(u)) => parts.push(format!("dose {a} {u}")),
        (Some(a), None) => parts.push(format!("dose {a}")),
        (None, Some(u)) => parts.push(format!("dose {u}")),
        (None, None) => {}
    }
    if let Some(v) = d.effective {
        parts.push(format!("effective {v}"));
    }
    if let Some(r) = d.reason {
        parts.push(format!("reason \"{r}\""));
    }
    for g in d.strike {
        parts.push(format!("{g} struck (unknown)"));
    }
    let mut s = String::from("Dose correction");
    if !parts.is_empty() {
        s.push_str(": ");
        s.push_str(&parts.join(", "));
    }
    if let Some(n) = d.note {
        s.push_str(&format!(" — {n}"));
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

    fn full_correction() -> DoseCorrection<'static> {
        DoseCorrection {
            medication_id: "11111111-1111-7111-8111-111111111111",
            corrects: "33333333-3333-7333-8333-333333333333",
            dose_amount: Some("20"),
            dose_unit: Some("mg"),
            effective: Some("2024-01"),
            effective_precision: Some("month"),
            reason: Some("titration"),
            strike: &[],
            note: Some("mis-keyed the date"),
            info_source: Some("clinician-observed"),
        }
    }

    #[test]
    fn correction_body_carries_all_patch_fields() {
        let v = dose_correction_body(&full_correction());
        assert_eq!(v["corrects"], "33333333-3333-7333-8333-333333333333");
        assert_eq!(v["dose"]["amount"], "20");
        assert_eq!(v["dose"]["unit"], "mg");
        assert_eq!(v["effective"]["value"], "2024-01");
        assert_eq!(v["effective"]["precision"], "month");
        assert_eq!(v["reason"], "titration");
        assert_eq!(v["note"], "mis-keyed the date");
        assert_eq!(v["info_source"], "clinician-observed");
        assert!(
            !v.as_object().unwrap().contains_key("strike"),
            "empty strike omitted"
        );
    }

    #[test]
    fn correction_body_effective_only_omits_dose_and_reason() {
        // Fix ONLY the date — dose/reason keys absent (patch: untouched, not struck).
        let c = DoseCorrection {
            medication_id: "11111111-1111-7111-8111-111111111111",
            corrects: "33333333-3333-7333-8333-333333333333",
            dose_amount: None,
            dose_unit: None,
            effective: Some("2024-01"),
            effective_precision: None,
            reason: None,
            strike: &[],
            note: None,
            info_source: None,
        };
        let v = dose_correction_body(&c);
        let obj = v.as_object().unwrap();
        assert!(!obj.contains_key("dose"), "untouched dose omitted");
        assert!(!obj.contains_key("reason"), "untouched reason omitted");
        assert!(!obj.contains_key("strike"));
        assert_eq!(v["effective"]["value"], "2024-01");
        assert!(!v["effective"]
            .as_object()
            .unwrap()
            .contains_key("precision"));
    }

    #[test]
    fn correction_body_strike_emits_group_list() {
        // "the 40 was a guess — strike the dose to unknown" (principle 4, now explicit).
        let c = DoseCorrection {
            medication_id: "11111111-1111-7111-8111-111111111111",
            corrects: "33333333-3333-7333-8333-333333333333",
            dose_amount: None,
            dose_unit: None,
            effective: None,
            effective_precision: None,
            reason: None,
            strike: &["dose"],
            note: Some("was a guess"),
            info_source: None,
        };
        let v = dose_correction_body(&c);
        assert_eq!(v["strike"][0], "dose");
        assert!(
            !v.as_object().unwrap().contains_key("dose"),
            "struck group carries no set value"
        );
        assert_eq!(v["note"], "was a guess");
    }

    #[test]
    fn correction_twin_is_nonempty_for_set_strike_and_note() {
        let s = render_dose_correction_twin(&full_correction());
        assert!(s.contains("20 mg"));
        assert!(s.contains("2024-01"));
        assert!(s.contains("titration"));
        assert!(s.contains("mis-keyed the date"));
        assert!(s.contains("clinician-observed"));

        let struck = DoseCorrection {
            medication_id: "11111111-1111-7111-8111-111111111111",
            corrects: "33333333-3333-7333-8333-333333333333",
            dose_amount: None,
            dose_unit: None,
            effective: None,
            effective_precision: None,
            reason: None,
            strike: &["effective"],
            note: None,
            info_source: None,
        };
        let t = render_dose_correction_twin(&struck);
        assert!(
            t.contains("effective"),
            "twin names the struck group, got: {t}"
        );
        assert!(!t.trim().is_empty());
    }
}
