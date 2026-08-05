//! §5.3/§5.8 patient registration — the wire shape of the act that brings a chart into
//! being, and of the search that preceded it.
//!
//! # Why registration is an event at all
//!
//! Before this type a standard chart came into being as a *side effect* of whatever event
//! happened to carry its `patient_id` first. §5.8 requires the create act to record that N
//! near-matches were displayed, and a side effect has nowhere to record anything. So
//! registration becomes an act, with §5.3's three classes as one discriminant so the
//! floor's precedence rule never needs an exception (ADR-0061 decision 1 — an "unless" in a
//! safety floor is where the next defect lives).
//!
//! # Why the attestation NAMES candidates rather than counting them
//!
//! A duplicate found six months later poses one question: was it on the screen when the
//! clerk clicked create? "Yes" means human judgement failed (fix the UI); "no" means the
//! search failed (fix the comparator). Those have opposite fixes, and a bare `N = 3`
//! cannot tell them apart. So `displayed` carries the candidate ids themselves.
//!
//! The displayed-and-not-chosen set is WEAK evidence — the clerk may never have read it.
//! It is not an `unlink` and must never be projected as a judgement that the charts differ.
//!
//! # Layering
//!
//! The builders take plain primitives rather than `cairn_patient_search::SearchAttestation`
//! because `cairn-event` is the wire core: depending on a read-model crate would invert the
//! layering. `cairn-node`'s `patient::register` wires the two, and carries the round-trip
//! test that keeps the seam honest.
use serde_json::{json, Value};
use uuid::Uuid;

/// The event type registered in `event_type_class` and the twin-check registry (db/045).
pub const REGISTRATION_EVENT_TYPE: &str = "identity.registration.asserted";
/// Wire schema version. Bumping this is an ADDITIVE act (ADR-0012): add fields, never
/// remove or repurpose one.
pub const REGISTRATION_SCHEMA_VERSION: &str = "identity.registration.asserted/1";

/// §5.3's three registration classes. Closed set — the db/045 floor refuses anything else,
/// so adding a member here means adding it there in the same commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistrationClass {
    /// Normal registration. The only class that carries — and must carry — a search.
    Standard,
    /// §5.4 John Doe. Search-AFTER-create by necessity: there is nothing to search with.
    Unidentified,
    /// §5.6 legally sanctioned anonymous/protective care.
    Pseudonymous,
}

impl RegistrationClass {
    /// The wire token. These strings are the floor's closed set — do not reword them.
    pub fn as_str(self) -> &'static str {
        match self {
            RegistrationClass::Standard => "standard",
            RegistrationClass::Unidentified => "unidentified",
            RegistrationClass::Pseudonymous => "pseudonymous",
        }
    }
}

/// What the clerk actually typed. Borrowed so the caller keeps ownership of its buffers.
#[derive(Debug, Clone, Copy)]
pub struct SearchTerms<'a> {
    /// Lower-cased name tokens. May be empty if the clerk searched by identifier alone.
    pub name_tokens: &'a [String],
    /// ISO `YYYY-MM-DD`, or `None` when not asked/not known.
    pub birth_date: Option<&'a str>,
    /// `(system, value)` pairs, e.g. `("MRN", "12345")`.
    pub identifiers: &'a [(String, String)],
}

/// The search a standard registration attests to.
#[derive(Debug, Clone, Copy)]
pub struct SearchAttestationInput<'a> {
    pub terms: SearchTerms<'a>,
    /// The candidates ACTUALLY on screen. May be empty — that is the normal case for a
    /// genuinely new patient, and it must never be tightened into a non-empty requirement.
    pub displayed: &'a [Uuid],
    /// True when the node knows it could not show everything it found or could not read
    /// some candidate. ADR-0060 decision 2: partial completion is reported, never implied.
    pub incomplete: bool,
}

/// One registration act.
#[derive(Debug, Clone, Copy)]
pub struct RegistrationAssertion<'a> {
    pub class: RegistrationClass,
    /// Why this class. Carried for the non-standard classes, where it is genuinely
    /// informative ("unconscious ED arrival, no ID"). Omitted for `Standard`: there the
    /// class IS the explanation, and a mandatory free-text box would be a required field
    /// satisfiable only by fabrication (principle 4).
    pub basis: Option<&'a str>,
    /// Present iff `class == Standard`. The db/045 floor enforces both directions.
    pub search: Option<SearchAttestationInput<'a>>,
}

/// Build the event payload. Pure — every input is supplied by the caller, so the whole
/// wire shape is unit-testable with no clock, no database and no key.
pub fn registration_assertion_body(a: &RegistrationAssertion) -> Value {
    let mut body = json!({ "class": a.class.as_str() });
    if let Some(basis) = a.basis {
        body["basis"] = json!(basis);
    }
    if let Some(s) = &a.search {
        let identifiers: Vec<Value> = s
            .terms
            .identifiers
            .iter()
            .map(|(system, value)| json!({ "system": system, "value": value }))
            .collect();
        // `displayed` serialises as an array of canonical UUID strings even when empty —
        // an empty ARRAY means "the search ran and found nothing", which is entirely
        // different from an absent `search` key ("no search ran").
        let displayed: Vec<Value> = s.displayed.iter().map(|u| json!(u.to_string())).collect();
        body["search"] = json!({
            "query": {
                "name_tokens": s.terms.name_tokens,
                "birth_date": s.terms.birth_date,
                "identifiers": identifiers,
            },
            "displayed": displayed,
            "incomplete": s.incomplete,
        });
    }
    body
}

/// The mandatory §3.13 legibility twin: this act in plain language, for a reader with no
/// schema at all (principle 11). Mechanically derived from the same inputs as the payload.
pub fn render_registration_twin(a: &RegistrationAssertion) -> String {
    let mut out = format!("Patient registered ({} registration)", a.class.as_str());
    if let Some(basis) = a.basis {
        out.push_str(&format!("; basis: {basis}"));
    }
    if let Some(s) = &a.search {
        out.push_str(&format!(
            "; searched before creating, {} near-match(es) displayed",
            s.displayed.len()
        ));
        if s.incomplete {
            // Never let a reader of the twin alone believe the search was exhaustive.
            out.push_str(" (search incomplete — not everything found could be shown)");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn displayed() -> Vec<Uuid> {
        vec![
            Uuid::from_u128(0x1111_1111_1111_1111_1111_1111_1111_1111),
            Uuid::from_u128(0x2222_2222_2222_2222_2222_2222_2222_2222),
        ]
    }

    fn tokens() -> Vec<String> {
        vec!["smith".to_string(), "john".to_string()]
    }

    #[test]
    fn standard_registration_carries_its_search() {
        let ids = displayed();
        let toks = tokens();
        let sys: Vec<(String, String)> = vec![("MRN".into(), "12345".into())];
        let a = RegistrationAssertion {
            class: RegistrationClass::Standard,
            basis: None,
            search: Some(SearchAttestationInput {
                terms: SearchTerms {
                    name_tokens: &toks,
                    birth_date: Some("1980-01-01"),
                    identifiers: &sys,
                },
                displayed: &ids,
                incomplete: false,
            }),
        };
        let b = registration_assertion_body(&a);
        assert_eq!(b["class"], "standard");
        assert_eq!(b["search"]["query"]["birth_date"], "1980-01-01");
        assert_eq!(b["search"]["query"]["name_tokens"][0], "smith");
        assert_eq!(b["search"]["query"]["identifiers"][0]["system"], "MRN");
        assert_eq!(b["search"]["query"]["identifiers"][0]["value"], "12345");
        assert_eq!(b["search"]["displayed"][0], ids[0].to_string());
        assert_eq!(b["search"]["incomplete"], false);
        // No count field: length(displayed) IS the count. Two representations of one
        // number is a lie waiting to happen (ADR-0061 decision 2).
        assert!(b["search"].get("displayed_count").is_none());
        // basis is omitted entirely for a standard registration (principle 4: a
        // mandatory free-text box here would be satisfiable only by fabrication).
        assert!(b.get("basis").is_none());
    }

    #[test]
    fn an_empty_candidate_list_is_a_valid_search() {
        // The NORMAL case for a genuinely new patient: the search ran and correctly
        // found nothing. `[]` must survive as an empty ARRAY, never become null or
        // vanish — a missing key would read as "no search ran".
        let toks = tokens();
        let a = RegistrationAssertion {
            class: RegistrationClass::Standard,
            basis: None,
            search: Some(SearchAttestationInput {
                terms: SearchTerms {
                    name_tokens: &toks,
                    birth_date: None,
                    identifiers: &[],
                },
                displayed: &[],
                incomplete: false,
            }),
        };
        let b = registration_assertion_body(&a);
        assert!(b["search"]["displayed"].is_array());
        assert_eq!(b["search"]["displayed"].as_array().unwrap().len(), 0);
        assert!(b["search"]["query"]["birth_date"].is_null());
    }

    #[test]
    fn non_standard_classes_carry_no_search_key_at_all() {
        // Structural absence, not an empty object: a search attestation on an
        // unconscious patient would be a precise untruth (principle 4).
        for class in [
            RegistrationClass::Unidentified,
            RegistrationClass::Pseudonymous,
        ] {
            let a = RegistrationAssertion {
                class,
                basis: Some("unidentified patient, no ID"),
                search: None,
            };
            let b = registration_assertion_body(&a);
            assert!(
                b.get("search").is_none(),
                "{} must carry no search key",
                class.as_str()
            );
            assert_eq!(b["basis"], "unidentified patient, no ID");
        }
    }

    #[test]
    fn twin_is_non_empty_and_states_the_class_and_how_many_were_seen() {
        let ids = displayed();
        let toks = tokens();
        let a = RegistrationAssertion {
            class: RegistrationClass::Standard,
            basis: None,
            search: Some(SearchAttestationInput {
                terms: SearchTerms {
                    name_tokens: &toks,
                    birth_date: None,
                    identifiers: &[],
                },
                displayed: &ids,
                incomplete: false,
            }),
        };
        let twin = render_registration_twin(&a);
        assert!(
            !twin.trim().is_empty(),
            "the floor requires a non-empty twin"
        );
        assert!(twin.contains("standard"));
        // Anchor the count to its OWN phrase, and derive it from the fixture rather than
        // writing a literal: `contains('2')` would have passed on any stray '2' anywhere in
        // the sentence (a date, a version, a reworded basis), so it did not actually test
        // that the twin states the candidate count at all.
        let expected_count = format!("{} near-match", ids.len());
        assert!(
            twin.contains(&expected_count),
            "the twin must state how many candidates were displayed; \
             expected {expected_count:?} in {twin:?}"
        );
    }

    #[test]
    fn twin_says_so_when_the_search_was_incomplete() {
        let toks = tokens();
        let a = RegistrationAssertion {
            class: RegistrationClass::Standard,
            basis: None,
            search: Some(SearchAttestationInput {
                terms: SearchTerms {
                    name_tokens: &toks,
                    birth_date: None,
                    identifiers: &[],
                },
                displayed: &[],
                incomplete: true,
            }),
        };
        // ADR-0060 decision 2: partial completion is REPORTED, never implied. A reader
        // of the twin alone must not believe the search was exhaustive.
        assert!(render_registration_twin(&a).contains("incomplete"));
    }
}
