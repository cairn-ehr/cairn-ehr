//! The ONE definition of what a registration attests to.
//!
//! This constructor is the whole reason the crate exists. If the surface that displays
//! candidates and the act that attests to them each built their own answer to "what was
//! shown?", a registration could swear to candidates the clerk never saw — destroying
//! exactly the forensic record the funnel is for. So the attestation is derived FROM the
//! displayed list and cannot be constructed independently of one.
use crate::candidate::CandidateList;
use crate::query::SearchQuery;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchAttestation {
    pub query: SearchQuery,
    /// The candidate ids that were on the screen, in display order.
    pub displayed: Vec<Uuid>,
    /// Carried straight through from the list — never re-decided here.
    pub incomplete: bool,
}

impl SearchAttestation {
    /// Derive the attestation from the query and the list that was actually displayed.
    pub fn from_displayed(query: &SearchQuery, list: &CandidateList) -> Self {
        Self {
            query: query.clone(),
            displayed: list.candidates.iter().map(|c| c.patient_id).collect(),
            incomplete: list.incomplete,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candidate::{Candidate, CandidateList, TrustState};
    use uuid::Uuid;

    fn candidate(n: u128) -> Candidate {
        Candidate {
            patient_id: Uuid::from_u128(n),
            display_name: format!("Patient {n}"),
            age: None,
            trust: TrustState::Confirmed,
            last_activity: None,
            locale: None,
            photo_ref: None,
        }
    }

    #[test]
    fn the_attestation_names_exactly_what_the_list_held_in_order() {
        let list = CandidateList {
            candidates: vec![candidate(1), candidate(2)],
            incomplete: false,
            incomplete_reason: None,
        };
        let q = SearchQuery::new("smith", None, &[]);
        let a = SearchAttestation::from_displayed(&q, &list);
        assert_eq!(a.displayed, vec![Uuid::from_u128(1), Uuid::from_u128(2)]);
        assert!(!a.incomplete);
    }

    #[test]
    fn incompleteness_propagates_from_the_list_it_was_built_from() {
        // The whole reason this constructor exists: the surface that DISPLAYS and the act
        // that ATTESTS must not be able to disagree. A registration must never swear to a
        // complete search over a list the node knew was partial.
        let list = CandidateList {
            candidates: vec![candidate(7)],
            incomplete: true,
            incomplete_reason: Some("one chart unreadable".into()),
        };
        let q = SearchQuery::new("smith", None, &[]);
        assert!(SearchAttestation::from_displayed(&q, &list).incomplete);
    }

    #[test]
    fn an_empty_list_attests_to_an_empty_search_not_to_no_search() {
        let list = CandidateList {
            candidates: vec![],
            incomplete: false,
            incomplete_reason: None,
        };
        let q = SearchQuery::new("nobody", None, &[]);
        let a = SearchAttestation::from_displayed(&q, &list);
        assert!(a.displayed.is_empty());
        assert_eq!(a.query.name_tokens, vec!["nobody"]);
    }
}
