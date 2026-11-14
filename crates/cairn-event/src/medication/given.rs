//! The given dose and its "entered in error" retraction (charting slice 2, a stat dose
//! ordered then given).
//!
//! Pure: no clock, no I/O. Two new event types, both `additive` and born-sealed.
//!
//! Why the given type is GENERAL: `clinical.medication-given.asserted` says only "a dose of
//! this medication thread was given" (the envelope's `t_effective` is the time given). A future
//! medication-administration record (MAR) reuses it unchanged. `completes` is the one slice-2
//! tie: it names the stop (cessation) that completes a stat order, so a reader can pair the give
//! with its stat line without guessing; it is null for an ordinary give.
//!
//! Why the retraction carries `voids` ITSELF: a struck give must make the stat line current
//! again, but only if the stop it caused is the one that goes. Putting the uuid of that paired
//! stop on the retraction (rather than deriving it from the given, which may be shredded) means
//! the void keys on the retraction alone, so shredding the given cannot resurrect the stop.
//!
//! Blank free text is written as JSON null and left out of the twin (ADR-0097 decision 3),
//! exactly as the allergy overlay does. Every key and twin word is wire, frozen once merged.
use serde_json::{json, Value};

/// Event type of a given dose.
pub const GIVEN_EVENT_TYPE: &str = "clinical.medication-given.asserted";
/// Payload schema tag of a given dose, class `additive`.
pub const GIVEN_SCHEMA: &str = "clinical.medication-given/1";
/// Event type of a given dose's "entered in error" retraction.
pub const GIVEN_RETRACTION_EVENT_TYPE: &str = "clinical.medication-given-retraction.asserted";
/// Payload schema tag of the retraction, class `additive`.
pub const GIVEN_RETRACTION_SCHEMA: &str = "clinical.medication-given-retraction/1";
/// The `reason` of the cessation that pairs with a stat give.
pub const STAT_GIVEN_REASON: &str = "stat dose given";

/// One given dose of the thread `medication_id`.
#[derive(Debug, Clone, Copy)]
pub struct Given<'a> {
    pub medication_id: &'a str,
    /// The stop (cessation) event this give completes; `None` for an ordinary give.
    pub completes: Option<&'a str>,
}

/// The payload: exactly `{medication_id, completes}`; `completes` is null when `None`.
pub fn given_body(g: &Given) -> Value {
    json!({ "medication_id": g.medication_id, "completes": g.completes })
}

/// `Dose given: medication {id}{ — completes its stat order}`. Word order is frozen.
pub fn render_given_twin(g: &Given) -> String {
    let mut t = format!("Dose given: medication {}", g.medication_id);
    if g.completes.is_some() {
        t.push_str(" — completes its stat order");
    }
    t
}

/// One "entered in error" retraction of a given dose.
#[derive(Debug, Clone, Copy)]
pub struct GivenRetraction<'a> {
    pub medication_id: &'a str,
    /// The `event_id` of the given being struck.
    pub target: &'a str,
    /// The paired stop this strike voids; `None` when the given stopped nothing.
    pub voids: Option<&'a str>,
    /// Optional free-text why; blank counts as none.
    pub text: Option<&'a str>,
}

/// The one blank test: text that is empty or whitespace-only is absent.
fn present(text: Option<&str>) -> Option<&str> {
    text.filter(|t| !t.trim().is_empty())
}

/// The payload: exactly `{medication_id, target, voids, text}`; blank text and absent voids are null.
pub fn given_retraction_body(r: &GivenRetraction) -> Value {
    json!({
        "medication_id": r.medication_id,
        "target": r.target,
        "voids": r.voids,
        "text": present(r.text),
    })
}

/// `Entered in error: withdraws given dose {target}{ — {text}}`. Word order is frozen.
pub fn render_given_retraction_twin(r: &GivenRetraction) -> String {
    let mut t = format!("Entered in error: withdraws given dose {}", r.target);
    if let Some(w) = present(r.text) {
        t.push_str(&format!(" — {}", w.trim()));
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    const M: &str = "11111111-1111-7111-8111-111111111111";
    const STOP: &str = "22222222-2222-7222-8222-222222222222";
    const G: &str = "33333333-3333-7333-8333-333333333333";

    fn retraction(
        voids: Option<&'static str>,
        text: Option<&'static str>,
    ) -> GivenRetraction<'static> {
        GivenRetraction {
            medication_id: M,
            target: G,
            voids,
            text,
        }
    }

    #[test]
    fn the_given_body_is_exactly_medication_id_and_completes() {
        let v = given_body(&Given {
            medication_id: M,
            completes: Some(STOP),
        });
        assert_eq!(v, json!({"medication_id": M, "completes": STOP}));
        assert_eq!(v.as_object().unwrap().len(), 2);
    }

    #[test]
    fn a_none_completes_is_json_null() {
        let v = given_body(&Given {
            medication_id: M,
            completes: None,
        });
        assert_eq!(v, json!({"medication_id": M, "completes": null}));
    }

    #[test]
    fn the_given_twin_says_it_completes_its_stat_order() {
        let plain = Given {
            medication_id: M,
            completes: None,
        };
        assert_eq!(
            render_given_twin(&plain),
            format!("Dose given: medication {M}")
        );
        let stat = Given {
            completes: Some(STOP),
            ..plain
        };
        assert_eq!(
            render_given_twin(&stat),
            format!("Dose given: medication {M} — completes its stat order")
        );
    }

    #[test]
    fn the_retraction_body_is_exactly_four_keys() {
        let v = given_retraction_body(&retraction(Some(STOP), Some("wrong patient")));
        assert_eq!(
            v,
            json!({"medication_id": M, "target": G, "voids": STOP, "text": "wrong patient"})
        );
        assert_eq!(v.as_object().unwrap().len(), 4);
        let bare = given_retraction_body(&retraction(None, None));
        assert_eq!(
            bare,
            json!({"medication_id": M, "target": G, "voids": null, "text": null})
        );
    }

    #[test]
    fn blank_retraction_text_is_null_and_absent_from_the_twin() {
        for blank in [Some(""), Some("  \t"), None] {
            let r = retraction(None, blank);
            assert_eq!(given_retraction_body(&r)["text"], Value::Null);
            assert_eq!(
                render_given_retraction_twin(&r),
                format!("Entered in error: withdraws given dose {G}")
            );
        }
    }

    #[test]
    fn the_retraction_twin_names_its_target() {
        assert_eq!(
            render_given_retraction_twin(&retraction(Some(STOP), Some("wrong patient"))),
            format!("Entered in error: withdraws given dose {G} — wrong patient")
        );
    }
}
