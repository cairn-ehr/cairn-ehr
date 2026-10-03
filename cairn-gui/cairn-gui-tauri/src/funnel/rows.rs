//! A person row as the window renders it (R3, ADR-0076 decision 6).
//!
//! The clerk's unit is a PERSON: charts the identity layer has linked are one row. This module is
//! the pure translation from `cairn_patient_search::PersonRow` to the payload the webview draws,
//! plus the two sentences that talk about people versus charts, so no wording lives in JS.
//!
//! # Why every member line has its own open
//!
//! A row of linked charts does NOT pick one chart for the clerk. Each member is its own open
//! target (the maintainer's decision): the chart that gets opened decides what a doubted set lets
//! you sign and where new content will go, and that is a choice for the person at the desk, not
//! for a collapse rule. The combined record reads the same whichever member is opened.
//!
//! # Why the label is worded here
//!
//! "One person — 2 linked charts" is a sentence a clerk acts on, so it is tested in Rust like every
//! other sentence on the front door (`view.rs`); `funnel.js` only draws it.
use crate::funnel::view::{candidate_view, CandidateView};
use cairn_patient_search::PersonRow;
use serde::Serialize;

/// One person, as the list shows them.
#[derive(Debug, Clone, Serialize)]
pub struct PersonRowView {
    /// `Some` only for a LINKED row; a single chart is rendered exactly as it always was.
    pub label: Option<String>,
    /// The row's charts, in the row's own order — each one an open (or Compare) target.
    pub members: Vec<CandidateView>,
}

/// The label over a linked row. `charts` is the member count (at least 2 for a linked row).
pub fn linked_row_label(charts: usize) -> String {
    format!("One person — {charts} linked charts")
}

/// A person row as the webview draws it. Pure.
pub fn person_row_view(row: &PersonRow) -> PersonRowView {
    PersonRowView {
        label: row
            .is_linked()
            .then(|| linked_row_label(row.members().len())),
        members: row.members().iter().map(candidate_view).collect(),
    }
}

/// " ({charts} charts)" when the charts outnumber the people, else nothing. Shared by
/// [`people_phrase`] and the link panel's search line so those two name charts the same way
/// (`browse_summary` keeps its own arms, each pinned by a golden sentence).
pub fn charts_suffix(people: usize, charts: usize) -> String {
    if charts == people {
        String::new()
    } else {
        format!(" ({charts} charts)")
    }
}

/// "{n} existing patient(s)", naming the charts only when they outnumber the people. Serves the
/// step-3 announcement; the link panel's line shares [`charts_suffix`].
pub fn people_phrase(people: usize, charts: usize) -> String {
    format!(
        "{people} existing patient(s){}",
        charts_suffix(people, charts)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::funnel::view::tests::sample_candidate;
    use cairn_patient_search::TrustState;

    fn chart(n: u128) -> cairn_patient_search::Candidate {
        let mut c = sample_candidate();
        c.patient_id = uuid::Uuid::from_u128(n);
        c
    }

    #[test]
    fn a_single_chart_row_has_no_label_and_one_member() {
        let c = chart(1);
        let v = person_row_view(&PersonRow::alone(c.clone()));
        assert_eq!(v.label, None);
        assert_eq!(v.members, vec![candidate_view(&c)]);
    }

    #[test]
    fn a_linked_row_is_labelled_and_keeps_its_member_order() {
        let row = PersonRow::new(vec![chart(2), chart(1)]).unwrap();
        let v = person_row_view(&row);
        assert_eq!(v.label.as_deref(), Some("One person — 2 linked charts"));
        let ids: Vec<&str> = v.members.iter().map(|m| m.patient_id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                uuid::Uuid::from_u128(2).to_string(),
                uuid::Uuid::from_u128(1).to_string()
            ]
        );
    }

    #[test]
    fn a_chart_not_held_here_reads_unknown() {
        let mut c = chart(3);
        c.trust = TrustState::Unknown;
        assert_eq!(
            person_row_view(&PersonRow::alone(c)).members[0].trust,
            "unknown"
        );
    }

    #[test]
    fn the_phrase_names_charts_only_when_they_outnumber_people() {
        assert_eq!(people_phrase(2, 2), "2 existing patient(s)");
        assert_eq!(people_phrase(2, 3), "2 existing patient(s) (3 charts)");
    }
}
