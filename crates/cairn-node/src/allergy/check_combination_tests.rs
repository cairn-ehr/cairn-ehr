//! The medication check over a COMBINATION (charting slice 1a): the window hands one `MedLine`
//! per component, all sharing the line's `group_id`, and the line's answer is every part's
//! answer merged ([`merge`]). Review focus 3: a combination with one part unpicked must never
//! read clean. Helpers are `check_tests`'s.
use super::tests::*;
use super::*;
use crate::allergy::drug_facts::{Answer, DrugFacts};
use crate::allergy::test_fixtures::{coded, entry, id};

fn check(marks: &[(&str, u128)], notes: &[&str], unverified: bool) -> LineCheck {
    LineCheck {
        notes: notes.iter().map(|s| s.to_string()).collect(),
        unverified,
        ..super::tests::marks(marks)
    }
}

#[test]
fn merge_unions_marks_and_against_in_order_and_dedups() {
    let a = check(&[("m1", 1), ("m2", 2)], &["n1"], false);
    // m2 again (same words, same head), and m4: new words for a head already marked.
    let b = check(&[("m2", 2), ("m3", 3), ("m4", 1)], &["n1", "n2"], false);
    assert_eq!(
        merge(a, b),
        check(&[("m1", 1), ("m2", 2), ("m3", 3)], &["n1", "n2"], false)
    );
}

/// ADR-0098: `unverified` is false whenever `marks` is non-empty, whichever side had the mark.
#[test]
fn merge_keeps_unverified_false_when_either_has_a_mark() {
    let marked = || check(&[("m1", 1)], &[], false);
    assert!(!merge(marked(), not_checked()).unverified);
    assert!(!merge(not_checked(), marked()).unverified);
    assert!(merge(not_checked(), LineCheck::default()).unverified);
    assert!(merge(LineCheck::default(), not_checked()).unverified);
    assert_eq!(
        merge(LineCheck::default(), LineCheck::default()),
        LineCheck::default()
    );
}

/// Augmentin charted as two parts: amoxicillin picked, clavulanic acid not. The penicillin-group
/// allergy meets the amoxicillin part, so the LINE carries the mark — the uncoded part cannot
/// overwrite it.
#[test]
fn two_lines_one_group_both_count() {
    let a = only(vec![coded(
        entry(1, "drug", "penicillin", "allergy", "anaphylaxis", 1),
        GROUP_SYSTEM,
        PEN_GROUP,
    )]);
    let f = facts(vec![(
        PEN_GROUP,
        AMOX,
        path("same-group", Some("Penicillins")),
    )]);
    let part = |term, moiety| MedLine {
        name: "Augmentin",
        term,
        ..med(900, term, moiety)
    };
    let lines = [
        part("amoxicillin", Some(AMOX)),
        part("clavulanic acid", None),
    ];
    assert_eq!(
        run(&a, &lines, &f)[&id(900)],
        marks(&[(
            "Augmentin is a penicillin; allergy recorded: anaphylaxis",
            1
        )])
    );
}

/// Panadeine as "paracetamol + codeine" with only paracetamol picked, against an amoxicillin
/// allergy: the picked part is a verified no-match, the unpicked part is unverified, so the line
/// is "not checked", never clean — in either order: a clean part never clears the other's tag.
#[test]
fn a_combination_with_an_unpicked_part_is_not_checked_clean() {
    let a = only(vec![amox("anaphylaxis")]);
    let f = facts(vec![(AMOX, PARACETAMOL, Answer::NoMatch)]);
    let part = |term, moiety| MedLine {
        name: "Panadeine",
        ..med(900, term, moiety)
    };
    let lines = [
        part("codeine", None),
        part("paracetamol", Some(PARACETAMOL)),
    ];
    let reversed = [lines[1], lines[0]];
    for f in [&f, &DrugFacts::Unavailable] {
        assert_eq!(run(&a, &lines, f)[&id(900)], not_checked());
        assert_eq!(run(&a, &reversed, f)[&id(900)], not_checked());
    }
}

/// A combination whose two components each meet a DIFFERENT allergy (amoxicillin, paracetamol):
/// the line keeps both marks, and `against` names both records, in part order — the merge never
/// drops a mark, so an override names every allergy it was charted despite.
#[test]
fn two_components_two_allergens_keep_both_marks() {
    let a = only(vec![
        amox("anaphylaxis"),
        coded(
            entry(2, "drug", "paracetamol", "allergy", "rash", 1),
            MOIETY_SYSTEM,
            PARACETAMOL,
        ),
    ]);
    let part = |term, moiety| MedLine {
        name: "Combination",
        ..med(900, term, moiety)
    };
    let lines = [
        part("amoxicillin", Some(AMOX)),
        part("paracetamol", Some(PARACETAMOL)),
    ];
    assert_eq!(
        run(&a, &lines, &DrugFacts::Unavailable)[&id(900)],
        marks(&[
            ("Allergy recorded: amoxicillin → anaphylaxis", 1),
            ("Allergy recorded: paracetamol → rash", 2),
        ])
    );
}
