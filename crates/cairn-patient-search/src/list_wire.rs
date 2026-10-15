//! The wire shape of a [`CandidateList`], and the one place the doubt mark crosses it (#786).
//!
//! # The problem this solves
//!
//! R6 (#723) marks a person row whose links db/054 doubts ([`PersonRow::with_doubt`]), so the
//! front door never reads a disputed set as "One person — N linked charts". The mark deliberately
//! stays OUT of a row's own wire shape: a row serialises as its bare member array, the shape a
//! registration's attested list is built from, and that shape must not change.
//!
//! Left there, though, the mark was lost whenever a `CandidateList` crossed a process boundary:
//! every row read back as undoubted. Under ADR-0021 another front-end built on the native API
//! would then show a disputed set exactly like an undisputed one. That fails open on a
//! principle-4 display: an untruth stated as fact.
//!
//! # The shape
//!
//! An ADDITIVE sibling of `people`: `doubted_rows`, the indices of the doubted rows, in order.
//! - **Absent when no row is doubted**, so an undoubted list serialises byte-for-byte as before,
//!   and a reader written before #786 reads every list it could already read.
//! - **Defaulted when absent**, so a list written before #786 reads as "no row marked". That
//!   means *not marked*, never *verified undoubted* (the same meaning `PersonRow`'s flag has).
//! - **Refused when it names a row that is not there.** A malformed list is an error; guessing
//!   which row was meant would be worse.
//!
//! The in-memory [`CandidateList`] keeps ONE source of truth, each row's own flag. This module
//! only projects it to and from the wire, so the two cannot disagree. Nothing here is signed:
//! what a registration attests to is [`CandidateList::displayed_charts`], which ignores the mark.
use crate::candidate::CandidateList;
use crate::person::PersonRow;
use serde::{Deserialize, Serialize};
use std::fmt;

/// A [`CandidateList`] as it travels. Private: callers serialise `CandidateList` itself, and
/// serde routes through this (`#[serde(try_from, into)]` on the list).
#[derive(Serialize, Deserialize)]
pub(crate) struct CandidateListWire {
    people: Vec<PersonRow>,
    incomplete: bool,
    incomplete_reason: Option<String>,
    /// Indices into `people` of the rows whose links db/054 doubts. Written in row order; a
    /// reader accepts any order and repeats, since marking a row twice changes nothing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    doubted_rows: Vec<usize>,
}

/// The error for a wire list whose `doubted_rows` names a row the list does not have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DoubtedRowOutOfRange {
    /// The index named.
    pub index: usize,
    /// How many rows the list has.
    pub rows: usize,
}

impl fmt::Display for DoubtedRowOutOfRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "doubted_rows names row {} of a list with {} row(s)",
            self.index, self.rows
        )
    }
}

impl std::error::Error for DoubtedRowOutOfRange {}

impl From<CandidateList> for CandidateListWire {
    fn from(list: CandidateList) -> Self {
        let doubted_rows = list
            .people
            .iter()
            .enumerate()
            .filter(|(_, row)| row.is_doubted())
            .map(|(i, _)| i)
            .collect();
        CandidateListWire {
            people: list.people,
            incomplete: list.incomplete,
            incomplete_reason: list.incomplete_reason,
            doubted_rows,
        }
    }
}

impl TryFrom<CandidateListWire> for CandidateList {
    type Error = DoubtedRowOutOfRange;

    fn try_from(wire: CandidateListWire) -> Result<Self, DoubtedRowOutOfRange> {
        let rows = wire.people.len();
        // Check every index before marking any, so a refused list is refused whole.
        if let Some(&index) = wire.doubted_rows.iter().find(|&&i| i >= rows) {
            return Err(DoubtedRowOutOfRange { index, rows });
        }
        let mut people = wire.people;
        for i in wire.doubted_rows {
            people[i] = people[i].clone().with_doubt(true);
        }
        Ok(CandidateList {
            people,
            incomplete: wire.incomplete,
            incomplete_reason: wire.incomplete_reason,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::candidate::{Candidate, CandidateList, TrustState};
    use crate::person::PersonRow;
    use uuid::Uuid;

    fn member(n: u128) -> Candidate {
        Candidate {
            patient_id: Uuid::from_u128(n),
            display_name: format!("Person {n}"),
            age: None,
            trust: TrustState::Unconfirmed,
            last_activity: None,
            locale: None,
            photo_ref: None,
        }
    }

    /// Three rows: an undoubted single, a DOUBTED linked pair, an undoubted linked pair. The
    /// doubted row sits in the middle, so an off-by-one index or "mark the first linked row"
    /// cannot pass.
    fn list() -> CandidateList {
        CandidateList {
            people: vec![
                PersonRow::alone(member(1)),
                PersonRow::new(vec![member(2), member(3)])
                    .unwrap()
                    .with_doubt(true),
                PersonRow::new(vec![member(4), member(5)]).unwrap(),
            ],
            incomplete: false,
            incomplete_reason: None,
        }
    }

    #[test]
    fn a_doubted_row_keeps_its_mark_across_the_wire() {
        let value = serde_json::to_value(list()).unwrap();
        assert_eq!(value["doubted_rows"], serde_json::json!([1]));
        let back: CandidateList = serde_json::from_value(value).unwrap();
        let marks: Vec<bool> = back.people.iter().map(PersonRow::is_doubted).collect();
        assert_eq!(marks, vec![false, true, false]);
        assert_eq!(back, list(), "the derived PartialEq compares the mark too");
    }

    #[test]
    fn an_undoubted_list_serialises_exactly_as_before_786() {
        let mut undoubted = list();
        undoubted.people[1] = undoubted.people[1].clone().with_doubt(false);
        let value = serde_json::to_value(&undoubted).unwrap();
        let keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            vec!["incomplete", "incomplete_reason", "people"],
            "no doubted_rows key unless a row is doubted (additive: an older reader sees no change)"
        );
    }

    /// The key-set test above reads a `serde_json::Value`, a sorted map, so it cannot see field
    /// ORDER. This one pins the exact text: the pre-#786 bytes, in declaration order.
    #[test]
    fn an_undoubted_list_serialises_to_the_same_bytes_as_before_786() {
        let undoubted = CandidateList {
            people: vec![PersonRow::alone(member(1))],
            incomplete: true,
            incomplete_reason: Some("why".into()),
        };
        let member_json = serde_json::to_string(&member(1)).unwrap();
        assert_eq!(
            serde_json::to_string(&undoubted).unwrap(),
            format!(
                r#"{{"people":[[{member_json}]],"incomplete":true,"incomplete_reason":"why"}}"#
            )
        );
    }

    #[test]
    fn a_list_written_before_786_reads_as_no_row_marked() {
        let old = serde_json::json!({
            "people": [[serde_json::to_value(member(1)).unwrap()]],
            "incomplete": false,
            "incomplete_reason": null
        });
        let back: CandidateList = serde_json::from_value(old).unwrap();
        assert!(!back.people[0].is_doubted());
    }

    #[test]
    fn a_doubted_index_past_the_last_row_is_refused_whole() {
        let mut value = serde_json::to_value(list()).unwrap();
        value["doubted_rows"] = serde_json::json!([1, 3]);
        let err = serde_json::from_value::<CandidateList>(value).unwrap_err();
        assert!(
            err.to_string()
                .contains("doubted_rows names row 3 of a list with 3 row(s)"),
            "{err}"
        );
    }

    #[test]
    fn the_mark_never_changes_what_a_registration_attests() {
        let mut undoubted = list();
        undoubted.people[1] = undoubted.people[1].clone().with_doubt(false);
        assert_eq!(list().displayed_charts(), undoubted.displayed_charts());
    }
}
