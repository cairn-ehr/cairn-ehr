//! Medication *withhold* and *release* builders (ADR-0081) — a time-bounded hold on an
//! existing thread, and the early end of named holds. Pure: shapes only the payload
//! JSON and the plaintext twin. Like cessation, neither carries the drug name (it lives
//! on the assertion), so the twins are terse but always non-empty.
//!
//! Optional fields are inserted only when present, never as null (principle 11).
//! Instants (`from`, `until`) are ISO-8601 with an explicit UTC offset; the database
//! floor refuses offset-less ones, so this layer passes them through verbatim.
use serde_json::{json, Value};

/// How a hold ends: at an asserted instant, or when a clinician reviews it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldUntil<'a> {
    At(&'a str),
    Reviewed,
}

impl HoldUntil<'_> {
    /// The wire / twin text: the instant itself, or the literal `reviewed`.
    fn as_str(&self) -> &str {
        match self {
            HoldUntil::At(t) => t,
            HoldUntil::Reviewed => "reviewed",
        }
    }
}

/// A medication withhold: "do not give this for now", with a reason and an end.
pub struct MedicationWithhold<'a> {
    pub medication_id: &'a str,
    pub reason: &'a str,
    /// Absent means "from now" (the event's own effective time).
    pub from: Option<&'a str>,
    pub until: HoldUntil<'a>,
}

/// Build the `clinical.medication-withhold.asserted` payload.
pub fn medication_withhold_body(w: &MedicationWithhold) -> Value {
    let mut p = json!({
        "medication_id": w.medication_id,
        "reason": w.reason,
        "until": w.until.as_str(),
    });
    if let Some(f) = w.from {
        p.as_object_mut()
            .expect("json! built an object")
            .insert("from".into(), json!(f));
    }
    p
}

/// Legibility twin: `Withheld medication from <from|now> until <until|reviewed> — <reason>`.
pub fn render_medication_withhold_twin(w: &MedicationWithhold) -> String {
    format!(
        "Withheld medication from {} until {} — {}",
        w.from.unwrap_or("now"),
        w.until.as_str(),
        w.reason
    )
}

/// Ends named withholds early. Names them by withhold *event id*, so it can only end
/// holds its author saw.
pub struct MedicationWithholdRelease<'a> {
    pub medication_id: &'a str,
    pub releases: &'a [String],
    pub reason: Option<&'a str>,
}

/// Build the `clinical.medication-withhold-release.asserted` payload.
pub fn medication_withhold_release_body(r: &MedicationWithholdRelease) -> Value {
    let mut p = json!({ "medication_id": r.medication_id, "releases": r.releases });
    if let Some(x) = r.reason {
        p.as_object_mut()
            .expect("json! built an object")
            .insert("reason".into(), json!(x));
    }
    p
}

/// Legibility twin: `Released withheld medication (<n> hold(s))[ — <reason>]`.
pub fn render_medication_withhold_release_twin(r: &MedicationWithholdRelease) -> String {
    let n = r.releases.len();
    let mut s = format!(
        "Released withheld medication ({n} hold{})",
        if n == 1 { "" } else { "s" }
    );
    if let Some(x) = r.reason {
        s.push_str(&format!(" — {x}"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    const MED: &str = "11111111-1111-7111-8111-111111111111";

    #[test]
    fn withhold_body_carries_until_reviewed_and_omits_from() {
        let w = MedicationWithhold {
            medication_id: MED,
            reason: "BP 85/50",
            from: None,
            until: HoldUntil::Reviewed,
        };
        let v = medication_withhold_body(&w);
        assert_eq!(v["until"], "reviewed");
        assert_eq!(v["reason"], "BP 85/50");
        assert_eq!(v["medication_id"], MED);
        assert!(!v.as_object().unwrap().contains_key("from"));
    }

    #[test]
    fn withhold_body_carries_an_until_instant_and_a_from() {
        let w = MedicationWithhold {
            medication_id: MED,
            reason: "pre-op",
            from: Some("2026-10-21T20:00:00+10:00"),
            until: HoldUntil::At("2026-10-22T08:00:00+10:00"),
        };
        let v = medication_withhold_body(&w);
        assert_eq!(v["from"], "2026-10-21T20:00:00+10:00");
        assert_eq!(v["until"], "2026-10-22T08:00:00+10:00");
    }

    #[test]
    fn withhold_twin_names_span_and_reason_not_the_drug() {
        let w = MedicationWithhold {
            medication_id: MED,
            reason: "discuss with GP",
            from: None,
            until: HoldUntil::Reviewed,
        };
        assert_eq!(
            render_medication_withhold_twin(&w),
            "Withheld medication from now until reviewed — discuss with GP"
        );
    }

    #[test]
    fn withhold_twin_prints_from_then_until_when_both_are_instants() {
        // from ≠ until, so a renderer that swapped them (or printed one twice) fails here.
        let w = MedicationWithhold {
            medication_id: MED,
            reason: "pre-op",
            from: Some("2026-10-21T20:00:00+10:00"),
            until: HoldUntil::At("2026-10-22T08:00:00+10:00"),
        };
        assert_eq!(
            render_medication_withhold_twin(&w),
            "Withheld medication from 2026-10-21T20:00:00+10:00 until 2026-10-22T08:00:00+10:00 — pre-op"
        );
    }

    #[test]
    fn release_body_lists_event_ids_and_omits_absent_reason() {
        let ids = vec!["a".to_string(), "b".to_string()];
        let r = MedicationWithholdRelease {
            medication_id: MED,
            releases: &ids,
            reason: None,
        };
        let v = medication_withhold_release_body(&r);
        assert_eq!(v["releases"], serde_json::json!(["a", "b"]));
        assert!(!v.as_object().unwrap().contains_key("reason"));
    }

    #[test]
    fn release_twin_counts_holds() {
        let two = vec!["a".to_string(), "b".to_string()];
        let r = MedicationWithholdRelease {
            medication_id: MED,
            releases: &two,
            reason: Some("BP recovered"),
        };
        assert_eq!(
            render_medication_withhold_release_twin(&r),
            "Released withheld medication (2 holds) — BP recovered"
        );
        let one = vec!["a".to_string()];
        let r = MedicationWithholdRelease {
            medication_id: MED,
            releases: &one,
            reason: None,
        };
        assert_eq!(
            render_medication_withhold_release_twin(&r),
            "Released withheld medication (1 hold)"
        );
        // One hold WITH a reason: the singular and the reason together.
        let r = MedicationWithholdRelease {
            medication_id: MED,
            releases: &one,
            reason: Some("HR back to 70"),
        };
        assert_eq!(
            render_medication_withhold_release_twin(&r),
            "Released withheld medication (1 hold) — HR back to 70"
        );
    }
}
