//! Tests for `link/unlink_view.rs`, in a sibling file like `view_tests.rs`.
use super::*;
use cairn_node::patient::compare::{FieldFact, NameFact};

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

fn held(n: u128) -> ChartFacts {
    ChartFacts {
        patient_id: id(n),
        held: true,
        trust: "confirmed".into(),
        names: vec![NameFact {
            value: "N".into(),
            use_: Some("legal".into()),
            provenance: "patient-stated".into(),
        }],
        dob: Some(FieldFact {
            value: "1950-07-01".into(),
            provenance: "document-verified".into(),
            precision: Some("day".into()),
        }),
        ..ChartFacts::default()
    }
}

fn set(v: &[u128]) -> ChartSet {
    ChartSet::new(v.iter().map(|n| Uuid::from_u128(*n))).unwrap()
}

fn parts() -> UnlinkParts {
    UnlinkParts {
        low: Ok(vec![held(1)]),
        high: Ok(vec![held(2)]),
        findings: Ok(vec![]),
    }
}

#[test]
fn an_unlink_comparison_has_the_two_charts_and_sends_back_the_record() {
    let v = unlink_comparison_view(parts(), &set(&[1, 2, 3]), id(1), id(2));
    assert_eq!(v.columns.len(), 2);
    assert_eq!(
        v.charts.len(),
        3,
        "the record compared FROM, sent back with the unlink"
    );
    assert_eq!(
        (v.low.clone(), v.high.clone()),
        (id(1).to_string(), id(2).to_string())
    );
    assert!(v.can_unlink);
}

#[test]
fn a_partial_unlink_comparison_names_what_is_missing_and_cannot_unlink() {
    let mut p = parts();
    p.findings = Err("timeout".into());
    let v = unlink_comparison_view(p, &set(&[1, 2]), id(1), id(2));
    assert!(!v.can_unlink);
    assert!(v.problems.iter().any(|m| m.contains("could not be run")));
}

#[test]
fn an_unread_chart_is_named_and_cannot_unlink() {
    let mut p = parts();
    p.high = Err("gone".into());
    let v = unlink_comparison_view(p, &set(&[1, 2]), id(1), id(2));
    assert!(!v.can_unlink);
    assert!(v.problems.iter().any(|m| m.contains(&id(2).to_string())));
}

#[test]
fn a_split_names_the_charts_that_left_and_reloads() {
    let r = unlink_report(
        LinkEffect::TookEffect,
        id(2),
        id(3),
        &set(&[1, 2, 3]),
        &set(&[1, 2]),
    );
    assert!(r.sentence.starts_with("Unlinked"));
    assert!(r.sentence.contains(&id(3).to_string()));
    assert!(r.reload);
}

/// Review Focus 2.
#[test]
fn still_joined_never_reads_as_done_and_points_at_the_links_list() {
    let r = unlink_report(
        LinkEffect::StillJoined,
        id(2),
        id(3),
        &set(&[1, 2, 3]),
        &set(&[1, 2, 3]),
    );
    assert!(!r.sentence.starts_with("Unlinked"));
    assert!(r.sentence.contains("still"));
    assert!(r.sentence.contains("How these charts are linked"));
    assert!(
        r.reload,
        "the list must re-read: this link is gone from it, the others remain"
    );
}

#[test]
fn outranked_is_a_disagreement_to_settle_not_retry() {
    let r = unlink_report(
        LinkEffect::Outranked,
        id(2),
        id(3),
        &set(&[1, 2, 3]),
        &set(&[1, 2, 3]),
    );
    assert!(r.sentence.contains("NOT in effect"));
    assert!(r.sentence.contains("the same person"));
    assert!(!r.reload);
}

#[test]
fn an_unlink_error_is_worded_as_an_unlink() {
    let v = crate::link::view::judgement_error_from(
        "unlink",
        cairn_gui_data::port::DataError::Refused("x".into()),
    );
    assert!(v.text.starts_with("The unlink was refused"));
}
