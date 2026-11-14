//! The warning threshold (batch-charting slice 3, ADR-0104): an adverse-effect note keeps the
//! record it came from, and whether a check WARNS depends on the site's threshold. At the
//! default only an allergy mark warns; lowered to `AdverseEffect`, a previous adverse effect
//! warns too — named by its own record, after the marks. A note with no record ("the allergy
//! list could not be read") never warns. Helpers are `check_tests`'s.
use super::tests::*;
use super::*;
use crate::allergy::drug_facts::DrugFacts;
use crate::allergy::test_fixtures::{coded, entry, id};

const NOTE: &str = "Previous adverse effect: codeine → GI upset";

/// A codeine line against an adverse effect to codeine (record `id(1)`).
fn codeine_note() -> LineCheck {
    let a = only(vec![entry(
        1,
        "drug",
        "codeine",
        "adverse-effect",
        "gi-upset",
        1,
    )]);
    run(&a, &[med(900, "codeine", None)], &DrugFacts::Unavailable)[&id(900)].clone()
}

#[test]
fn an_adverse_effect_note_keeps_its_record() {
    let c = codeine_note();
    assert_eq!(c.notes, vec![NOTE.to_string()]);
    assert_eq!(c.adverse_against, vec![id(1)]);
    assert_eq!(
        c.threshold,
        WarningThreshold::AllergyHit,
        "the caller sets it"
    );
}

#[test]
fn at_the_default_an_adverse_effect_does_not_warn() {
    let c = codeine_note();
    assert!(!c.warns());
    assert!(c.warned_against().is_empty());
    assert!(c.warning_notes().is_empty());
    assert_eq!(c.quiet_notes(), [NOTE.to_string()]);
}

#[test]
fn lowered_an_adverse_effect_warns_by_its_own_record() {
    let c = LineCheck {
        threshold: WarningThreshold::AdverseEffect,
        ..codeine_note()
    };
    assert!(c.warns());
    assert_eq!(c.warned_against(), vec![id(1)]);
    assert_eq!(c.warning_notes(), [NOTE.to_string()]);
    assert!(c.quiet_notes().is_empty());
}

/// A line meeting an allergy (record 1) and an adverse effect (record 2): the override names
/// the mark's record first, then the note's.
#[test]
fn lowered_a_mark_and_a_note_are_named_mark_first() {
    let a = only(vec![
        amox("anaphylaxis"),
        coded(
            entry(2, "drug", "amoxicillin", "adverse-effect", "gi-upset", 2),
            MOIETY_SYSTEM,
            AMOX,
        ),
    ]);
    let got = run(
        &a,
        &[med(900, "amoxicillin", Some(AMOX))],
        &DrugFacts::Unavailable,
    );
    let c = LineCheck {
        threshold: WarningThreshold::AdverseEffect,
        ..got[&id(900)].clone()
    };
    assert_eq!(c.against, vec![id(1)]);
    assert_eq!(c.adverse_against, vec![id(2)]);
    assert_eq!(c.warned_against(), vec![id(1), id(2)]);
    // At the default the mark alone warns.
    let d = LineCheck {
        threshold: WarningThreshold::AllergyHit,
        ..c
    };
    assert_eq!(d.warned_against(), vec![id(1)]);
}

/// `not_read`'s shape (review focus 3): a note with no record is information, never a warning.
#[test]
fn a_note_with_no_record_never_warns() {
    for threshold in [
        WarningThreshold::AllergyHit,
        WarningThreshold::AdverseEffect,
    ] {
        let c = LineCheck {
            notes: vec!["the allergy list could not be read".into()],
            unverified: true,
            threshold,
            ..LineCheck::default()
        };
        assert!(!c.warns(), "{threshold:?}");
        assert!(c.warning_notes().is_empty(), "{threshold:?}");
        assert_eq!(c.quiet_notes().len(), 1, "{threshold:?}");
    }
}

#[test]
fn the_threshold_parses_and_an_unknown_word_over_warns() {
    assert_eq!(
        WarningThreshold::parse("allergy-hit"),
        WarningThreshold::AllergyHit
    );
    assert_eq!(
        WarningThreshold::parse("adverse-effect"),
        WarningThreshold::AdverseEffect
    );
    assert_eq!(
        WarningThreshold::parse("interaction"),
        WarningThreshold::AdverseEffect
    );
    assert_eq!(WarningThreshold::parse(""), WarningThreshold::AdverseEffect);
    assert_eq!(WarningThreshold::AllergyHit.as_word(), "allergy-hit");
    assert_eq!(WarningThreshold::AdverseEffect.as_word(), "adverse-effect");
    assert_eq!(WarningThreshold::default(), WarningThreshold::AllergyHit);
}

fn noted(notes: &[(&str, Option<u128>)]) -> LineCheck {
    // Adverse notes (with a record) first, as `check_line` builds them.
    LineCheck {
        notes: notes.iter().map(|(w, _)| w.to_string()).collect(),
        adverse_against: notes.iter().filter_map(|(_, n)| n.map(id)).collect(),
        ..LineCheck::default()
    }
}

/// Review focus 1: a combination whose parts meet two adverse-effect records keeps each note
/// beside its own record; the same words twice keep one note and its first record.
#[test]
fn merge_keeps_each_note_beside_its_record() {
    let got = merge(noted(&[("n1", Some(1))]), noted(&[("n2", Some(2))]));
    assert_eq!(got, noted(&[("n1", Some(1)), ("n2", Some(2))]));
    let got = merge(noted(&[("n1", Some(1))]), noted(&[("n1", Some(3))]));
    assert_eq!(got, noted(&[("n1", Some(1))]));
}

/// A note with no record never shifts a record onto the wrong note: the adverse notes stay first.
#[test]
fn merge_keeps_the_adverse_notes_first() {
    let got = merge(noted(&[("unread", None)]), noted(&[("n2", Some(2))]));
    assert_eq!(got.notes, vec!["n2".to_string(), "unread".to_string()]);
    assert_eq!(got.adverse_against, vec![id(2)]);
}
