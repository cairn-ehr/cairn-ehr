//! `clinical.medication-set-aside.asserted` — a *version* of a drug the clinician set aside.
//!
//! Pure: no clock, no I/O. A set-aside version is what a source reported (a GP letter, a
//! daughter's recollection) that the clinician did NOT chart as a current medication,
//! because the facts disagree (e.g. "packet says 25 mg"). It is kept — never erased — as
//! an alternative to a real thread (`alternative_to`), and it is never itself a thread.
//!
//! Why it reuses [`medication_assertion_body`]: the drug fields must be spelled exactly as
//! a charted line spells them, so there is ONE spelling of a drug on the wire. We take that
//! body, drop the thread id (a version is not a thread) and add the version's own ids.
use serde_json::{json, Value};

use super::assert::{drug_line_twin, medication_assertion_body, MedicationAssertion};

pub const SET_ASIDE_EVENT_TYPE: &str = "clinical.medication-set-aside.asserted";
pub const SET_ASIDE_SCHEMA: &str = "clinical.medication-set-aside/1";

/// A version of a drug set aside as an alternative to the thread `alternative_to`.
pub struct MedicationSetAside<'a> {
    pub version_id: &'a str,
    pub alternative_to: &'a str,
    /// The drug fields, spelled exactly as a charted line. `safety` and `charting.charted_as` /
    /// `warning_override` must be None (the builder debug_asserts and strips them).
    pub drug: &'a MedicationAssertion<'a>,
}

/// The payload: the assertion's body minus `medication_id`, plus `version_id` and
/// `alternative_to`. `charted_as` / `warning_override` / `safety` are stripped: a set-aside
/// version was never charted, never overridden a warning, and asserts no safety claim.
pub fn set_aside_body(s: &MedicationSetAside) -> Value {
    debug_assert!(
        s.drug.safety.is_none(),
        "a set-aside version carries no safety claim"
    );
    debug_assert!(
        s.drug
            .charting
            .as_ref()
            .is_none_or(|c| c.charted_as.is_none() && c.warning_override.is_none()),
        "a set-aside version was never charted"
    );
    stripped_body(s)
}

/// The strip itself, without the debug assertions, so a test can prove the keys really are
/// removed (not merely absent from a well-behaved caller's input).
fn stripped_body(s: &MedicationSetAside) -> Value {
    let mut v = medication_assertion_body(s.drug);
    let o = v.as_object_mut().expect("assertion body is an object");
    for k in ["medication_id", "charted_as", "warning_override", "safety"] {
        o.remove(k);
    }
    o.insert("version_id".into(), json!(s.version_id));
    o.insert("alternative_to".into(), json!(s.alternative_to));
    v
}

/// `{from} reported {drug line}. Set aside, not current{ — {why}}`. `{from}` is the
/// `source_detail`, else the `info_source` word. Never contains the assertion's own
/// source note (it is spelled in the sentence instead).
pub fn render_set_aside_twin(s: &MedicationSetAside) -> String {
    let note = s.drug.source_note.unwrap_or_default();
    let from = note.source_detail.unwrap_or(s.drug.info_source);
    let mut t = format!(
        "{from} reported {}. Set aside, not current",
        drug_line_twin(s.drug)
    );
    if let Some(w) = note.why {
        t.push_str(&format!(" — {w}"));
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::medication::charting::{ChartedAs, Charting, OverrideRef, WarningOverride};
    use crate::medication::source_note::SourceNote;

    fn drug(note: Option<SourceNote<'static>>) -> MedicationAssertion<'static> {
        MedicationAssertion {
            medication_id: "11111111-1111-7111-8111-111111111111",
            term: "Lipitor",
            coding: None,
            formulation: Some("tablet"),
            dose_amount: Some("40"),
            dose_unit: Some("mg"),
            sig: Some("one BD"),
            info_source: "external-record",
            started: None,
            started_precision: None,
            safety: None,
            charting: None,
            source_note: note,
        }
    }

    fn set_aside<'a>(d: &'a MedicationAssertion<'a>) -> MedicationSetAside<'a> {
        MedicationSetAside {
            version_id: "22222222-2222-7222-8222-222222222222",
            alternative_to: "33333333-3333-7333-8333-333333333333",
            drug: d,
        }
    }

    #[test]
    fn a_set_aside_body_has_the_assertions_drug_fields_and_no_medication_id() {
        let d = drug(None);
        let v = set_aside_body(&set_aside(&d));
        let mut expected = medication_assertion_body(&d);
        expected.as_object_mut().unwrap().remove("medication_id");
        let o = v.as_object().unwrap();
        for (k, val) in expected.as_object().unwrap() {
            assert_eq!(&o[k], val, "drug field {k}");
        }
        assert!(!o.contains_key("medication_id"));
        assert_eq!(v["version_id"], "22222222-2222-7222-8222-222222222222");
        assert_eq!(v["alternative_to"], "33333333-3333-7333-8333-333333333333");
    }

    #[test]
    fn a_set_aside_body_carries_the_source_note_keys() {
        let d = drug(Some(SourceNote {
            source_detail: Some("GP letter"),
            why: Some("w"),
        }));
        let v = set_aside_body(&set_aside(&d));
        assert_eq!(v["source_detail"], "GP letter");
        assert_eq!(v["why"], "w");
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "never charted")]
    fn a_charted_drug_trips_the_debug_assert() {
        let d = charted_drug();
        set_aside_body(&set_aside(&d));
    }

    #[test]
    fn a_set_aside_body_never_carries_charted_as_or_warning_override() {
        let d = charted_drug();
        let v = stripped_body(&set_aside(&d));
        assert!(!v.as_object().unwrap().contains_key("charted_as"));
        assert!(!v.as_object().unwrap().contains_key("warning_override"));
        assert_eq!(v["route"], "po");
    }

    fn charted_drug() -> MedicationAssertion<'static> {
        static AGAINST: [OverrideRef<'static>; 1] = [OverrideRef {
            source: "allergy",
            reference: "E1",
        }];
        let mut d = drug(None);
        d.charting = Some(Charting {
            charted_as: Some(ChartedAs::StartedHere),
            route: Some("po"),
            warning_override: Some(WarningOverride {
                reason: "r",
                against: &AGAINST,
            }),
            ..Default::default()
        });
        d
    }

    #[test]
    fn the_set_aside_twin_reads_source_reported_set_aside_not_current() {
        let d = drug(Some(SourceNote {
            source_detail: Some("GP letter"),
            why: Some("packet 25 mg, daughter confirms"),
        }));
        assert_eq!(
            render_set_aside_twin(&set_aside(&d)),
            format!(
                "GP letter reported {}. Set aside, not current — packet 25 mg, daughter confirms",
                drug_line_twin(&d)
            )
        );
    }

    #[test]
    fn falls_back_to_the_info_source_word() {
        let d = drug(None);
        let t = render_set_aside_twin(&set_aside(&d));
        assert_eq!(
            t,
            format!(
                "external-record reported {}. Set aside, not current",
                drug_line_twin(&d)
            )
        );
    }
}
