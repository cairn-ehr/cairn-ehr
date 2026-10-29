//! Tests for `change.rs`: the one sentence a re-read says when the record's chart set moved
//! while it was on screen (#797), and which seen sets it refuses to compare at all.
//!
//! Every expected sentence is BUILT from the fixture `MemberLine`s and compared whole; no
//! assert echoes the line text in its message (house rule: identity-built text never reaches
//! a log, CodeQL's rust/cleartext-logging).
use super::*;
use crate::chart_set::MemberLine;
use uuid::Uuid;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

fn set(ns: &[u128]) -> ChartSet {
    ChartSet::new(ns.iter().map(|n| id(*n))).expect("a non-empty set")
}

/// A member line as `member_line` would build it, with a recognisable, non-identifying text.
fn line(n: u128) -> MemberLine {
    MemberLine {
        patient_id: id(n).to_string(),
        name: format!("Fixture {n}"),
        born: "born 2000-01-01".into(),
        text: format!(
            "Fixture {n} · born 2000-01-01 · identity confirmed · chart {}",
            id(n)
        ),
    }
}

fn unread(n: u128) -> String {
    format!("chart {} (its name could not be read here)", id(n))
}

#[test]
fn an_unchanged_set_says_nothing() {
    let now = set(&[1, 2]);
    let said = set_change_sentence(&set(&[2, 1]), &now, &[line(1), line(2)], &[]);
    assert!(said.is_none(), "an equal set must say nothing");
}

#[test]
fn a_joined_chart_is_named_by_its_own_line() {
    // Seen {1}, read {1, 2}: chart 2 joined, and its OWN line names it — the pane's member
    // lines carry both, but only the joined one is said.
    let said = set_change_sentence(&set(&[1]), &set(&[1, 2]), &[line(1), line(2)], &[]);
    let expected = format!(
        "This record changed while it was on screen. Now part of it: {}.",
        line(2).text
    );
    assert!(
        said.as_deref() == Some(expected.as_str()),
        "joined sentence differs"
    );
}

#[test]
fn a_joined_chart_without_a_line_is_said_unread() {
    // The pane's member lines failed to read: the joined chart is still named, by id.
    let said = set_change_sentence(&set(&[1]), &set(&[1, 2]), &[], &[]);
    let expected = format!(
        "This record changed while it was on screen. Now part of it: {}.",
        unread(2)
    );
    assert!(
        said.as_deref() == Some(expected.as_str()),
        "unread joined differs"
    );
}

#[test]
fn a_departed_chart_is_named_and_an_unread_one_is_said_unread() {
    // Seen {1, 2, 3}, read {1}: chart 2's identity was read, chart 3's could not be — it is
    // still said, never dropped, and WHY it could not be read is said too (#881): an outage
    // must not read like any other unread name. ChartSet order (2 before 3).
    let why = "connection to the node was lost and could not be reopened: no answer";
    let departed = [
        DepartedLine::Unread(id(3), why.into()),
        DepartedLine::Read(line(2)),
    ];
    let said = set_change_sentence(&set(&[1, 2, 3]), &set(&[1]), &[], &departed);
    let expected = format!(
        "This record changed while it was on screen. No longer part of it: {}; {}. Why the \
         name could not be read: {why}.",
        line(2).text,
        unread(3)
    );
    assert!(
        said.as_deref() == Some(expected.as_str()),
        "departed sentence differs"
    );
}

/// #881 review: one failed read leaves every departed chart unread for ONE reason; it is said
/// once, not inside each chart's parentheses, and a reason already ending in a full stop does
/// not get a second.
#[test]
fn a_failed_read_is_said_once_for_every_unread_departed_chart() {
    let why = "the node refused it. An operator must act.";
    let departed = [
        DepartedLine::Unread(id(2), why.into()),
        DepartedLine::Unread(id(3), why.into()),
    ];
    let said = set_change_sentence(&set(&[1, 2, 3]), &set(&[1]), &[], &departed);
    let expected = format!(
        "This record changed while it was on screen. No longer part of it: {}; {}. Why the \
         names could not be read: {why}",
        unread(2),
        unread(3)
    );
    assert!(
        said.as_deref() == Some(expected.as_str()),
        "departed sentence differs"
    );
}

#[test]
fn a_departed_chart_missing_from_the_lines_is_said_unread() {
    let said = set_change_sentence(&set(&[1, 2]), &set(&[1]), &[], &[]);
    let expected = format!(
        "This record changed while it was on screen. No longer part of it: {}.",
        unread(2)
    );
    assert!(
        said.as_deref() == Some(expected.as_str()),
        "missing departed differs"
    );
}

#[test]
fn both_halves_in_one_sentence_joined_first() {
    // Seen {1, 2}, read {1, 3, 4}: 3 and 4 joined, 2 left.
    let said = set_change_sentence(
        &set(&[1, 2]),
        &set(&[1, 3, 4]),
        &[line(1), line(4), line(3)],
        &[DepartedLine::Read(line(2))],
    );
    let expected = format!(
        "This record changed while it was on screen. Now part of it: {}; {}. No longer part \
         of it: {}.",
        line(3).text,
        line(4).text,
        line(2).text
    );
    assert!(
        said.as_deref() == Some(expected.as_str()),
        "two-halves sentence differs"
    );
}

/// The sentence is made of identity lines and fixed words, nothing else: a medication name
/// (or any other pane content) can never leak into it, because the function is never handed
/// one. Pinned by building the whole expected string from the lines alone.
#[test]
fn the_sentence_names_only_identity_lines_never_medications() {
    let said = set_change_sentence(
        &set(&[1]),
        &set(&[1, 2]),
        &[line(1), line(2)],
        &[DepartedLine::Read(line(9))],
    )
    .expect("a change is said");
    let expected = format!(
        "This record changed while it was on screen. Now part of it: {}.",
        line(2).text
    );
    assert!(said == expected, "only the joined identity line is said");
    // A departed line for a chart that was never seen is not part of the change.
    assert!(
        !said.contains(&id(9).to_string()),
        "an unseen chart is not named"
    );
}

// ---- Which seen sets are compared at all ----

#[test]
fn no_seen_set_compares_nothing() {
    assert!(seen_set(None, id(1)).is_none());
}

#[test]
fn an_empty_or_unparseable_seen_set_compares_nothing() {
    assert!(seen_set(Some(&[]), id(1)).is_none());
    let garbled = vec![id(1).to_string(), "not-a-uuid".to_string()];
    assert!(seen_set(Some(&garbled), id(1)).is_none());
}

/// A seen set that does not hold the opened chart was another record's screen (the window
/// moved to a new chart): comparing it would announce a whole different patient as "changed".
#[test]
fn a_seen_set_without_the_opened_chart_compares_nothing() {
    let other = vec![id(7).to_string()];
    assert!(seen_set(Some(&other), id(1)).is_none());
}

#[test]
fn a_seen_set_holding_the_opened_chart_is_compared() {
    let seen = vec![id(2).to_string(), id(1).to_string()];
    assert_eq!(seen_set(Some(&seen), id(1)), Some(set(&[1, 2])));
}

#[test]
fn the_departed_ids_are_the_seen_charts_no_longer_read() {
    assert_eq!(
        departed_ids(&set(&[1, 2, 3]), &set(&[1, 4])),
        vec![id(2), id(3)]
    );
    assert!(departed_ids(&set(&[1]), &set(&[1, 2])).is_empty());
}
