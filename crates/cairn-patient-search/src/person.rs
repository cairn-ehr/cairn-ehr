//! A *person row*: the unit the patient-search front door shows, one per PERSON rather than
//! one per chart (ADR-0076 decision 6).
//!
//! # What a row is
//!
//! Charts that have been linked (§5.7 "never merge, always link") belong to one *link
//! component* - one person. Two folders clipped together sit in one slot of the card index:
//! the clerk sees the person once, with every chart that person has listed inside the row,
//! instead of seeing the same human twice and registering a third chart.
//!
//! # Why this is pure and lives in the shared crate
//!
//! The node (which reads link components from Postgres) and any future picker window must
//! agree on what a row is, exactly as they must agree on what was displayed: a registration
//! attests to the candidates it showed, and a divergence in grouping would let it swear to
//! charts the clerk never saw. So the grouping rule is a pure function of two inputs the
//! caller has already read - the ranking and each chart's component - with no database here.
use crate::candidate::Candidate;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fmt;
use uuid::Uuid;

/// One row of the front door: every chart of one person that is worth showing, best match
/// first. Never empty (the constructors and `Deserialize` both refuse an empty row), so
/// consumers may take the first member as the row's headline without a check.
///
/// On the wire a row is just a JSON array of candidates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Vec<Candidate>", into = "Vec<Candidate>")]
pub struct PersonRow {
    members: Vec<Candidate>,
}

/// The error for building a row from nothing: a person row needs at least one chart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmptyRow;

impl fmt::Display for EmptyRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "a person row needs at least one chart")
    }
}

impl std::error::Error for EmptyRow {}

impl PersonRow {
    /// A row of the given charts, or `None` if there are none.
    pub fn new(members: Vec<Candidate>) -> Option<PersonRow> {
        if members.is_empty() {
            None
        } else {
            Some(PersonRow { members })
        }
    }

    /// A row holding exactly one chart (a person never linked to anything).
    pub fn alone(candidate: Candidate) -> PersonRow {
        PersonRow {
            members: vec![candidate],
        }
    }

    /// Every candidate as a row of one, preserving order.
    pub fn each_alone(candidates: Vec<Candidate>) -> Vec<PersonRow> {
        candidates.into_iter().map(PersonRow::alone).collect()
    }

    /// The charts of this person, best match first.
    pub fn members(&self) -> &[Candidate] {
        &self.members
    }

    /// True when more than one chart is clipped together in this row.
    pub fn is_linked(&self) -> bool {
        self.members.len() > 1
    }
}

impl TryFrom<Vec<Candidate>> for PersonRow {
    type Error = EmptyRow;

    fn try_from(members: Vec<Candidate>) -> Result<Self, EmptyRow> {
        PersonRow::new(members).ok_or(EmptyRow)
    }
}

impl From<PersonRow> for Vec<Candidate> {
    fn from(row: PersonRow) -> Vec<Candidate> {
        row.members
    }
}

/// A ranked chart had no link component in the map the caller supplied.
///
/// This is an error rather than a quiet row of one: falling back would put one person in two
/// places in the prompt with nothing said, which is the duplicate-creating failure the front
/// door exists to prevent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MissingComponent(pub Uuid);

impl fmt::Display for MissingComponent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "no link component was read for chart {}", self.0)
    }
}

impl std::error::Error for MissingComponent {}

/// Group a ranked list of matched charts into person rows of chart ids.
///
/// * `ranked` - the matched charts, best first (the unchanged ADR-0075 ranking).
/// * `components` - for each chart, its whole link component (every chart of that person),
///   sorted ascending (UUIDv7, so oldest first). A component is trusted to contain the chart
///   it is keyed by (db/054 always unions the chart itself in).
///
/// Rules: rows appear in the order of their best-ranked member; within a row the members the
/// search matched come first in rank order, then the members it did not match, oldest first
/// (an unmatched member has no rank keys, so it is never ranked, only listed). Every chart
/// of every component appears exactly once.
pub fn group_by_person(
    ranked: &[Uuid],
    components: &HashMap<Uuid, Vec<Uuid>>,
) -> Result<Vec<Vec<Uuid>>, MissingComponent> {
    // Where each matched chart sits in the ranking; a member absent here was not matched.
    let position: HashMap<Uuid, usize> =
        ranked.iter().enumerate().map(|(i, id)| (*id, i)).collect();
    let mut placed: HashSet<Uuid> = HashSet::new();
    let mut rows = Vec::new();
    for id in ranked {
        if placed.contains(id) {
            continue; // already in the row of a better-ranked member of the same person
        }
        let component = components.get(id).ok_or(MissingComponent(*id))?;
        let (mut matched, mut unmatched): (Vec<Uuid>, Vec<Uuid>) =
            component.iter().partition(|m| position.contains_key(*m));
        matched.sort_by_key(|m| position[m]);
        unmatched.sort(); // UUIDv7 ascending: oldest chart first
        let row: Vec<Uuid> = matched.into_iter().chain(unmatched).collect();
        placed.extend(row.iter().copied());
        rows.push(row);
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use uuid::Uuid;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    /// Each chart's component, as `read_components` returns it: sorted ascending.
    fn components(sets: &[&[u128]]) -> HashMap<Uuid, Vec<Uuid>> {
        let mut out = HashMap::new();
        for set in sets {
            let mut members: Vec<Uuid> = set.iter().map(|n| id(*n)).collect();
            members.sort();
            for m in &members {
                out.insert(*m, members.clone());
            }
        }
        out
    }

    #[test]
    fn never_linked_charts_are_one_row_each_in_rank_order() {
        let ranked = [id(3), id(1), id(2)];
        let got = group_by_person(&ranked, &components(&[&[1], &[2], &[3]])).unwrap();
        assert_eq!(got, vec![vec![id(3)], vec![id(1)], vec![id(2)]]);
    }

    #[test]
    fn a_row_is_placed_by_its_best_ranked_member_and_lists_both_matched_members_in_rank_order() {
        // Review Focus 1: 2 and 5 are one person; 5 ranked first, 2 ranked third.
        let ranked = [id(5), id(9), id(2)];
        let got = group_by_person(&ranked, &components(&[&[2, 5], &[9]])).unwrap();
        assert_eq!(got, vec![vec![id(5), id(2)], vec![id(9)]]);
    }

    #[test]
    fn members_the_search_did_not_match_follow_the_matched_ones_oldest_first() {
        // Review Focus 2: A(1)-B(2)-X(3) is one person; only X matched.
        let ranked = [id(3)];
        let got = group_by_person(&ranked, &components(&[&[1, 2, 3]])).unwrap();
        assert_eq!(got, vec![vec![id(3), id(1), id(2)]]);
    }

    #[test]
    fn every_chart_appears_exactly_once() {
        let ranked = [id(4), id(1), id(7), id(2)];
        let got = group_by_person(&ranked, &components(&[&[1, 2, 6], &[4], &[7, 8]])).unwrap();
        let mut flat: Vec<Uuid> = got.concat();
        flat.sort();
        assert_eq!(flat, vec![id(1), id(2), id(4), id(6), id(7), id(8)]);
    }

    #[test]
    fn a_chart_with_no_component_read_is_an_error_never_a_silent_row_of_one() {
        // Falling back to a row of one would put one person in two prompt places with nothing said.
        let err = group_by_person(&[id(1)], &HashMap::new()).unwrap_err();
        assert_eq!(err, MissingComponent(id(1)));
    }

    #[test]
    fn a_row_is_never_empty() {
        assert!(PersonRow::new(vec![]).is_none());
        let empty: Result<PersonRow, _> = serde_json::from_str("[]");
        assert!(empty.is_err(), "an empty row must not deserialize");
    }
}
