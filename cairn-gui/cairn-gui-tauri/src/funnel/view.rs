//! Every sentence the clerk reads at the front door, and every payload the webview renders, as
//! pure functions.
//!
//! # Why the wording lives in Rust
//!
//! The webview renders and decides nothing (see `src-ui/main.js`'s header). A sentence decided
//! in JavaScript is a sentence no test pins, and on this screen the wording IS the safety
//! content: "the search failed" and "nobody matched" differ by one word and lead to opposite
//! acts — the second licenses a new chart, the first must never (principle 4). So each
//! failure the funnel can meet maps here to exactly one sentence and one piece of retry
//! advice, and the webview only shows them.
//!
//! # The three kinds of retry advice
//!
//! [`Retry`] is the §648 split carried to the screen. An outage (`Unavailable`) decided
//! nothing, so the same act may succeed now. A verdict (`Refused`) will decide the same way
//! every time, so a retry button would be a precise untruth — the clerk's way forward is to
//! change what was typed. A node-state verdict (`NotProvisioned`) is pointless to retry until
//! an operator has acted, and then succeeds.
use cairn_gui_data::port::DataError;
use cairn_gui_funnel::{MissingPart, Restored, TokenError, TriggerState, MIN_NAME_TOKENS};
use cairn_node::actor_enrolment::{
    ambiguous_actor_refusal, not_enrolled_refusal, retired_actor_refusal, ActorStanding,
};
use cairn_patient_search::Candidate;
use serde::Serialize;
use uuid::Uuid;

/// What the webview may offer after a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Retry {
    /// Nothing was decided; the same act may succeed now. Keep the button live.
    Now,
    /// This node may not write until an operator acts; then the same act succeeds.
    AfterOperator,
    /// A verdict, or nothing left to retry with. The way forward is to change the form or wait
    /// for the next search, never to press the same button again.
    Never,
}

/// A failure, as the clerk reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ErrorView {
    pub text: String,
    pub retry: Retry,
}

/// Where this node stands, as the chrome says it — `None` when there is nothing to say.
///
/// Reuses `cairn-node`'s OWN refusal sentences rather than writing new ones, so the window and
/// the CLI cannot word one node state two ways. `kid` MUST be the key the standing was probed
/// for: a standing carries no subject (#670), and pairing it with the wrong key would hand an
/// operator an identity-level remedy for somebody else's key.
pub fn standing_sentence(standing: ActorStanding, kid: &str) -> Option<String> {
    let refusal = match standing {
        ActorStanding::Enrolled => return None,
        ActorStanding::NeverEnrolled => not_enrolled_refusal(kid),
        ActorStanding::Retired => retired_actor_refusal(kid),
        ActorStanding::Ambiguous => ambiguous_actor_refusal(kid),
    };
    Some(format!(
        "Registering is unavailable at this workstation until an operator acts. {refusal:#}"
    ))
}

/// A search that failed — browse or step 3.
///
/// The outage sentence says NOT in capitals on purpose: an empty-looking list after a failure
/// is read as "nobody matched", which on this screen means "create a new chart".
pub fn search_error_view(e: &DataError) -> ErrorView {
    match e {
        DataError::Unavailable(t) => ErrorView {
            text: format!(
                "The search FAILED — this is NOT a \"no match\". Do not register on the strength \
                 of it. ({t}) Try again."
            ),
            retry: Retry::Now,
        },
        DataError::Refused(t) => ErrorView {
            text: format!("The record refused this search as typed: {t}"),
            retry: Retry::Never,
        },
        DataError::NotProvisioned(t) => ErrorView {
            text: t.clone(),
            retry: Retry::AfterOperator,
        },
        DataError::NotFound => ErrorView {
            text: "No such chart.".to_string(),
            retry: Retry::Never,
        },
    }
}

/// A registration that failed, given what happened to its search.
///
/// Two inputs because the advice depends on both: an outage whose search was KEPT can simply
/// be retried, while any failure whose search was dropped (the form changed while it was
/// saving) has nothing left to retry with — the clerk waits for the new search.
pub fn register_error_view(e: &DataError, restored: Restored) -> ErrorView {
    let (base, retry_if_kept) = match e {
        DataError::Unavailable(t) => (
            format!("Nothing was saved; the node could not be reached ({t})."),
            Retry::Now,
        ),
        DataError::Refused(t) => (
            format!(
                "The record refused this registration as typed, and will refuse it again: {t}. \
                 Change what was typed."
            ),
            Retry::Never,
        ),
        DataError::NotProvisioned(t) => (
            format!(
                "This workstation's node may not write yet: {t} Nothing was saved; an operator \
                 must act before registering."
            ),
            Retry::AfterOperator,
        ),
        DataError::NotFound => ("Nothing was saved.".to_string(), Retry::Never),
    };
    match restored {
        Restored::Kept if retry_if_kept == Retry::Now => ErrorView {
            text: format!("{base} Press Register again."),
            retry: Retry::Now,
        },
        Restored::Kept => ErrorView {
            text: base,
            retry: retry_if_kept,
        },
        Restored::SupersededAndDropped => ErrorView {
            text: format!(
                "{base} The form changed while this was saving — wait for the new search before \
                 registering."
            ),
            retry: Retry::Never,
        },
    }
}

/// A registration the token store refused before anything was sent. Its own `Display` text
/// already says what to do (`cairn_gui_funnel::token`); it is never retried as is.
pub fn token_error_view(e: TokenError) -> ErrorView {
    ErrorView {
        text: e.to_string(),
        retry: Retry::Never,
    }
}

/// What the step-3 search is waiting for, or `None` when it runs now.
///
/// Ends by saying Register still works: the trigger is ADVISORY, never a gate — a mononymous
/// patient or an unknown date of birth never trips it, and must still be registrable
/// (principle 4, the design's *Risks*).
pub fn waiting_sentence(state: &TriggerState) -> Option<String> {
    let TriggerState::Waiting(parts) = state else {
        return None;
    };
    let parts: Vec<String> = parts
        .iter()
        .map(|part| match part {
            MissingPart::NameTokens { have } => format!(
                "at least {MIN_NAME_TOKENS} words of the name ({have} of {MIN_NAME_TOKENS} typed)"
            ),
            MissingPart::BirthDate => "a date of birth".to_string(),
        })
        .collect();
    Some(format!(
        "The record will be searched for this person automatically once it has {}. Register \
         still searches on whatever is typed.",
        parts.join(" and ")
    ))
}

/// One candidate as a list row shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CandidateView {
    /// The full chart id — what the webview sends back to open this chart.
    pub patient_id: String,
    pub name: String,
    /// `"46 y"`, or `"age not recorded"` — absence named, never a blank.
    pub age: String,
    pub trust: String,
}

/// The age as a row shows it. Absence is named (principle 4: a blank reads as a value).
fn age_label(c: &Candidate) -> String {
    c.age.as_ref().map_or_else(
        || "age not recorded".to_string(),
        |a| format!("{} y", a.years),
    )
}

pub fn candidate_view(c: &Candidate) -> CandidateView {
    CandidateView {
        patient_id: c.patient_id.to_string(),
        name: c.display_name.clone(),
        age: age_label(c),
        trust: c.trust.as_str().to_string(),
    }
}

/// The persistent identity header over an open chart — the wrong-chart affordance.
///
/// `born` carries an age or a date, depending on what was known when the chart was opened: a
/// candidate carries an age and no date of birth (`Candidate` has no dob field), a
/// registration carries the date that was typed. Each says which it is in its text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChartHeaderView {
    pub patient_id: String,
    pub name: String,
    pub born: String,
    pub trust: String,
}

/// The header for a chart picked from a list: exactly what that list row showed.
pub fn header_from_candidate(c: &Candidate) -> ChartHeaderView {
    ChartHeaderView {
        patient_id: c.patient_id.to_string(),
        name: c.display_name.clone(),
        born: age_label(c),
        trust: c.trust.as_str().to_string(),
    }
}

/// The header for a chart just registered: what was typed, with absence named.
///
/// Trust is `unconfirmed` — a chart nobody has confirmed the identity of yet, the same state
/// the mock registers into. Claiming more would put a trust state on screen no act earned.
pub fn header_from_registration(
    id: Uuid,
    raw_name: &str,
    birth_date: Option<&str>,
) -> ChartHeaderView {
    let name = raw_name.trim();
    ChartHeaderView {
        patient_id: id.to_string(),
        name: if name.is_empty() {
            "(no name recorded)".to_string()
        } else {
            name.to_string()
        },
        born: match birth_date.map(str::trim).filter(|d| !d.is_empty()) {
            Some(d) => format!("born {d}"),
            None => "date of birth not recorded".to_string(),
        },
        trust: "unconfirmed".to_string(),
    }
}

/// The header for a chart opened by `--patient` at launch, where no name was read.
///
/// Says so rather than showing a bare id as if it were a name: the operator runbook and the
/// accessibility pass open charts this way, and a header that looked complete would teach them
/// the wrong thing about what the window knows.
pub fn header_opened_by_id(id: Uuid) -> ChartHeaderView {
    ChartHeaderView {
        patient_id: id.to_string(),
        name: "(opened by chart id at launch — name not read)".to_string(),
        born: "not read".to_string(),
        trust: "not read".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_gui_funnel::trigger_state;
    use cairn_patient_search::{Age, TrustState};

    #[test]
    fn every_actor_standing_but_enrolled_has_its_own_sentence() {
        assert_eq!(standing_sentence(ActorStanding::Enrolled, "k"), None);
        let never = standing_sentence(ActorStanding::NeverEnrolled, "k").unwrap();
        let retired = standing_sentence(ActorStanding::Retired, "k").unwrap();
        let ambiguous = standing_sentence(ActorStanding::Ambiguous, "k").unwrap();
        assert!(never.contains("enroll-device-actor"), "{never}");
        // Retired must NOT offer enroll-device-actor as the fix (#152): the remedy is a key.
        assert!(retired.contains("NEW signing key"), "{retired}");
        assert!(ambiguous.contains("MORE THAN ONE"), "{ambiguous}");
        assert_ne!(never, retired);
        assert_ne!(retired, ambiguous);
    }

    /// The sentence must name the key the standing was computed for (#670).
    #[test]
    fn the_standing_sentence_names_the_probed_key() {
        let s = standing_sentence(ActorStanding::NeverEnrolled, "abc123").unwrap();
        assert!(s.contains("abc123"), "{s}");
    }

    #[test]
    fn a_failed_search_never_reads_as_nothing_found() {
        let v = search_error_view(&DataError::Unavailable("connection closed".into()));
        assert!(
            v.text.contains("NOT"),
            "must say this is not a no-match: {}",
            v.text
        );
        assert!(v.text.contains("connection closed"));
        assert_eq!(v.retry, Retry::Now);
    }

    #[test]
    fn a_refusal_withholds_the_retry_and_an_outage_offers_it() {
        let r = register_error_view(&DataError::Refused("bad dob".into()), Restored::Kept);
        assert_eq!(r.retry, Retry::Never);
        assert!(r.text.contains("bad dob"));
        let u = register_error_view(&DataError::Unavailable("timeout".into()), Restored::Kept);
        assert_eq!(u.retry, Retry::Now);
        assert!(u.text.contains("Press Register again"), "{}", u.text);
        let p = register_error_view(&DataError::NotProvisioned("run x".into()), Restored::Kept);
        assert_eq!(p.retry, Retry::AfterOperator);
        assert!(p.text.contains("run x"));
    }

    #[test]
    fn a_dropped_search_says_to_wait_for_the_new_one_not_to_press_register() {
        let v = register_error_view(
            &DataError::Unavailable("timeout".into()),
            Restored::SupersededAndDropped,
        );
        assert_eq!(v.retry, Retry::Never, "there is nothing to retry WITH");
        assert!(v.text.contains("new search"), "{}", v.text);
        assert!(!v.text.contains("Press Register again"), "{}", v.text);
    }

    #[test]
    fn a_token_refusal_is_its_own_sentence_and_never_retried() {
        let v = token_error_view(TokenError::Absent);
        assert_eq!(v.text, TokenError::Absent.to_string());
        assert_eq!(v.retry, Retry::Never);
    }

    #[test]
    fn retry_advice_crosses_to_the_webview_as_snake_case() {
        let json = serde_json::to_value(Retry::AfterOperator).unwrap();
        assert_eq!(json, serde_json::json!("after_operator"));
    }

    #[test]
    fn the_waiting_sentence_names_every_missing_part() {
        let s = waiting_sentence(&trigger_state("John", "")).unwrap();
        assert!(s.contains("1 of 2"), "{s}");
        assert!(s.contains("date of birth"), "{s}");
        assert!(
            s.contains("Register still searches"),
            "advisory, never a gate: {s}"
        );
        assert_eq!(waiting_sentence(&trigger_state("John Smith", "1980")), None);
    }

    #[test]
    fn a_candidate_renders_its_age_and_trust() {
        let v = candidate_view(&sample_candidate());
        assert_eq!(v.age, "46 y");
        assert_eq!(v.trust, "confirmed");
        assert_eq!(v.patient_id, uuid::Uuid::from_u128(5).to_string());
    }

    #[test]
    fn an_unknown_age_renders_as_absence_not_a_blank() {
        let mut c = sample_candidate();
        c.age = None;
        assert_eq!(candidate_view(&c).age, "age not recorded");
    }

    #[test]
    fn a_registration_header_carries_what_was_typed_and_names_absence() {
        let id = uuid::Uuid::from_u128(9);
        let h = header_from_registration(id, " ", None);
        assert_eq!(h.name, "(no name recorded)");
        assert_eq!(h.born, "date of birth not recorded");
        let h = header_from_registration(id, "Mary Poppins", Some("1910"));
        assert_eq!(h.name, "Mary Poppins");
        assert_eq!(h.born, "born 1910");
        assert_eq!(h.trust, "unconfirmed");
        assert_eq!(h.patient_id, id.to_string());
    }

    #[test]
    fn a_picked_candidates_header_is_what_the_list_showed() {
        let h = header_from_candidate(&sample_candidate());
        assert_eq!(h.name, "Samantha Example");
        assert_eq!(h.born, "46 y");
        assert_eq!(h.trust, "confirmed");
    }

    #[test]
    fn a_chart_opened_by_id_says_its_name_was_not_read() {
        let h = header_opened_by_id(uuid::Uuid::from_u128(9));
        assert!(h.name.contains("not read"), "{}", h.name);
    }

    /// A fully populated candidate, shared with the drift guard in `commands`.
    pub(crate) fn sample_candidate() -> Candidate {
        Candidate {
            patient_id: uuid::Uuid::from_u128(5),
            display_name: "Samantha Example".into(),
            age: Some(Age {
                years: 46,
                basis: "dob".into(),
            }),
            trust: TrustState::Confirmed,
            last_activity: None,
            locale: None,
            photo_ref: None,
        }
    }
}
