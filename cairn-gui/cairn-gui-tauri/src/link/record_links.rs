//! The pane's "How these charts are linked" list (repair path R2b-2): one line per standing
//! link, each with its own "Not the same person…" in the webview. Per LINK, not per member
//! chart (maintainer, 2026-09-30): each link appears once, so each control does too, and an
//! unlink that leaves two charts joined through another link can point at one list.
// Task 5 (the commands that call these) lands next; until then this is a binary crate with
// no caller, so dead_code would fail clippy. REMOVE this allow when Task 5 wires them in.
#![allow(dead_code)]

use crate::chart_set::MemberLine;
use cairn_node::patient::edges::RecordEdge;
use serde::Serialize;
use uuid::Uuid;

/// One link as the pane lists it. `low`/`high` travel back with "Not the same person…".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecordLinkView {
    /// The link's lower chart id.
    pub low: String,
    /// The link's higher chart id.
    pub high: String,
    /// The whole line as the clinician reads it.
    pub text: String,
}

/// A chart as a link line names it: its member line's name and its id, or the id alone when
/// the member lines could not be read (the ids still tie the line to the rows' source labels).
fn chart_label(chart: Uuid, members: &[MemberLine]) -> String {
    let id = chart.to_string();
    match members.iter().find(|m| m.patient_id == id) {
        Some(m) => format!("{} (chart {id})", m.name),
        None => format!("chart {id}"),
    }
}

/// One link's line. "Attested" is db/018's stored definition; its absence is worded as what
/// is known — no clinician's confirmation is on record HERE — never as "the matcher", which a
/// peer's human link with an attester this node has not enrolled would make untrue (principle 4).
pub fn record_link_line(edge: &RecordEdge, members: &[MemberLine]) -> RecordLinkView {
    let how = if edge.attested {
        "linked by a clinician's judgement"
    } else {
        "linked without a clinician's confirmation on record here"
    };
    RecordLinkView {
        low: edge.low.to_string(),
        high: edge.high.to_string(),
        text: format!(
            "{} and {} — {how}, recorded {}",
            chart_label(edge.low, members),
            chart_label(edge.high, members),
            edge.recorded_on
        ),
    }
}

/// The list, or — when it could not be read — no lines and a sentence saying so. An unread
/// list must never render as an empty one: that reads as "nothing joins these charts".
pub fn links_section(
    edges: Result<Vec<RecordEdge>, String>,
    members: &[MemberLine],
) -> (Vec<RecordLinkView>, Option<String>) {
    match edges {
        Ok(edges) => (
            edges.iter().map(|e| record_link_line(e, members)).collect(),
            None,
        ),
        Err(e) => (
            vec![],
            Some(format!(
                "The links joining these charts could not be read, so none can be undone from \
                 here until the chart is reloaded: {e}"
            )),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edge(attested: bool) -> RecordEdge {
        RecordEdge {
            low: Uuid::from_u128(1),
            high: Uuid::from_u128(2),
            attested,
            recorded_on: "2026-09-28".into(),
        }
    }
    fn member(n: u128, name: &str) -> MemberLine {
        MemberLine {
            patient_id: Uuid::from_u128(n).to_string(),
            name: name.into(),
            text: String::new(),
        }
    }

    #[test]
    fn a_link_line_names_both_charts_how_it_was_made_and_when() {
        let members = [member(1, "SMITH John"), member(2, "SMYTHE John")];
        let human = record_link_line(&edge(true), &members);
        assert!(human.text.contains("SMITH John") && human.text.contains("SMYTHE John"));
        assert!(human.text.contains("by a clinician's judgement"));
        assert!(human.text.contains("2026-09-28"));
        assert_eq!(human.low, Uuid::from_u128(1).to_string());
        let machine = record_link_line(&edge(false), &members);
        assert!(machine
            .text
            .contains("without a clinician's confirmation on record here"));
        assert!(
            !machine.text.contains("matcher"),
            "un-attested is not proof of the matcher"
        );
    }

    #[test]
    fn a_chart_missing_from_the_members_is_named_by_its_id_alone() {
        let line = record_link_line(&edge(true), &[]);
        assert!(line.text.contains(&Uuid::from_u128(1).to_string()));
    }

    /// Review Focus 5.
    #[test]
    fn an_unread_link_list_says_so_and_offers_no_unlink() {
        let (links, error) = links_section(Err("connection reset".into()), &[]);
        assert!(links.is_empty());
        let error = error.expect("an unread list is said, never shown empty");
        assert!(error.contains("could not be read"));
        assert!(error.contains("reload"));
    }

    #[test]
    fn a_read_list_carries_no_error() {
        let (links, error) = links_section(Ok(vec![edge(true)]), &[]);
        assert_eq!(links.len(), 1);
        assert!(error.is_none());
    }
}
