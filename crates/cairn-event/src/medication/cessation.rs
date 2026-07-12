//! Medication *cessation* builder (the "stop" verb) — references an existing thread.
//! Pure: shapes only the payload JSON. The drug name lives on the assertion, so the
//! cessation carries only the thread id, an optional stop date, and an optional reason.
use serde_json::{json, Value};

/// A medication cessation (the "stop" verb). Carries only the thread id it ends,
/// plus an optional uncertainty-capable stop date and reason.
pub struct MedicationCessation<'a> {
    pub medication_id: &'a str,
    pub stopped: Option<&'a str>,
    pub stopped_precision: Option<&'a str>,
    pub reason: Option<&'a str>,
}

/// Build the `clinical.medication-cessation.asserted` payload.
pub fn medication_cessation_body(c: &MedicationCessation) -> Value {
    let mut p = json!({ "medication_id": c.medication_id });
    let obj = p.as_object_mut().expect("json! built an object");
    if let Some(v) = c.stopped {
        let mut stopped = json!({ "value": v });
        if let Some(pr) = c.stopped_precision {
            stopped
                .as_object_mut()
                .expect("json! built an object")
                .insert("precision".into(), json!(pr));
        }
        obj.insert("stopped".into(), stopped);
    }
    if let Some(r) = c.reason {
        obj.insert("reason".into(), json!(r));
    }
    p
}

/// The legibility twin for a cessation. The drug name lives on the assertion, not
/// the cessation (which only references the thread), so this is deliberately terse
/// but always non-empty.
pub fn render_medication_cessation_twin(c: &MedicationCessation) -> String {
    let mut s = String::from("Ceased medication");
    if let Some(v) = c.stopped {
        s.push_str(&format!(" (stopped {v})"));
    }
    if let Some(r) = c.reason {
        s.push_str(&format!(" — {r}"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cessation_body_carries_and_omits_correctly() {
        let full = MedicationCessation {
            medication_id: "11111111-1111-7111-8111-111111111111",
            stopped: Some("2025-06"),
            stopped_precision: Some("month"),
            reason: Some("switched agent"),
        };
        let v = medication_cessation_body(&full);
        assert_eq!(v["medication_id"], "11111111-1111-7111-8111-111111111111");
        assert_eq!(v["stopped"]["value"], "2025-06");
        assert_eq!(v["stopped"]["precision"], "month");
        assert_eq!(v["reason"], "switched agent");

        let bare = MedicationCessation {
            medication_id: "11111111-1111-7111-8111-111111111111",
            stopped: None,
            stopped_precision: None,
            reason: None,
        };
        let vb = medication_cessation_body(&bare);
        let obj = vb.as_object().unwrap();
        assert!(
            !obj.contains_key("stopped"),
            "absent stopped omitted, not null"
        );
        assert!(!obj.contains_key("reason"));
        assert_eq!(vb["medication_id"], "11111111-1111-7111-8111-111111111111");
    }

    #[test]
    fn cessation_twin_is_nonempty() {
        let bare = MedicationCessation {
            medication_id: "11111111-1111-7111-8111-111111111111",
            stopped: None,
            stopped_precision: None,
            reason: None,
        };
        assert!(!render_medication_cessation_twin(&bare).trim().is_empty());
    }
}
