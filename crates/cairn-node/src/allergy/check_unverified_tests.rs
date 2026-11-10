//! The medication check's "not checked" tag (allergy slice 3, D8, rulings R1–R3) and the drug
//! group review status on the expanded list (D11, M7). Helpers are `check_tests`'s.
use super::tests::*;
use super::*;
use crate::allergy::drug_facts::{Answer, DrugFacts};
use crate::allergy::test_fixtures::{bne, coded, entry, id, pen};
use crate::allergy::thread::resolve;
use crate::allergy::view::build_view;

#[test]
fn no_match_is_verified() {
    let a = only(vec![amox("anaphylaxis")]);
    let f = facts(vec![(AMOX, PARACETAMOL, Answer::NoMatch)]);
    assert!(run(&a, &[med(900, "paracetamol", Some(PARACETAMOL))], &f).is_empty());
}

#[test]
fn unplaced_is_not_checked() {
    let a = only(vec![amox("anaphylaxis")]);
    let line = [med(900, "paracetamol", Some(PARACETAMOL))];
    // Unplaced (any reason, ruling R1), no row for the pair, or no drug list at all.
    for f in [
        facts(vec![(AMOX, PARACETAMOL, Answer::Unplaced)]),
        facts(vec![]),
        DrugFacts::Unavailable,
    ] {
        assert_eq!(run(&a, &line, &f)[&id(900)], not_checked());
    }
}

#[test]
fn an_uncoded_allergy_leaves_every_line_not_checked() {
    let a = only(vec![amox("rash"), pen(2, "rash", 1)]);
    let f = facts(vec![
        (AMOX, PARACETAMOL, Answer::NoMatch),
        (AMOX, FLUCLOX + 10, Answer::NoMatch),
    ]);
    let meds = [
        med(900, "paracetamol", Some(PARACETAMOL)),
        med(901, "metformin", Some(FLUCLOX + 10)),
    ];
    let got = run(&a, &meds, &f);
    assert_eq!(got[&id(900)], not_checked());
    assert_eq!(got[&id(901)], not_checked());
}

/// An allergy coded under a drug-list system whose code does not read as a uuid (a peer's
/// malformed coding: the remote door checks structure only) is asked nothing and checks nothing.
#[test]
fn an_allergy_code_that_is_not_a_uuid_is_not_checked() {
    let mut bad = amox("anaphylaxis");
    bad.term = "Augmentin".into();
    bad.coding.as_mut().unwrap().code = "not-a-uuid".into();
    let f = facts(vec![(AMOX, PARACETAMOL, Answer::NoMatch)]);
    let got = run(
        &only(vec![bad]),
        &[med(900, "paracetamol", Some(PARACETAMOL))],
        &f,
    );
    assert_eq!(got[&id(900)], not_checked());
}

#[test]
fn an_uncoded_line_is_not_checked() {
    let a = only(vec![amox("rash")]);
    let got = run(&a, &[med(900, "Grandma's tonic", None)], &facts(vec![]));
    assert_eq!(got[&id(900)], not_checked());
}

#[test]
fn a_marked_line_is_never_also_unverified() {
    let a = only(vec![
        pen(1, "rash", 1),
        entry(2, "drug", "sulfonamide", "allergy", "rash", 1),
    ]);
    let got = run(&a, &[med(900, "penicillin", None)], &DrugFacts::Unavailable);
    assert_eq!(
        got[&id(900)],
        marks(&["Allergy recorded: penicillin → rash"])
    );
}

#[test]
fn an_adverse_effect_never_makes_a_line_unverified() {
    let a = only(vec![entry(
        1,
        "drug",
        "codeine",
        "adverse-effect",
        "rash",
        1,
    )]);
    assert!(run(
        &a,
        &[med(900, "paracetamol", None)],
        &DrugFacts::Unavailable
    )
    .is_empty());
}

#[test]
fn a_group_not_reviewed_is_marked_in_the_expanded_list_only() {
    let a = only(vec![
        coded(
            entry(1, "drug", "penicillin", "allergy", "anaphylaxis", 1),
            GROUP_SYSTEM,
            PEN_GROUP,
        ),
        coded(
            entry(2, "drug", "cephalosporin", "allergy", "rash", 1),
            GROUP_SYSTEM,
            CEPH_GROUP,
        ),
        coded(
            entry(3, "drug", "codeine", "adverse-effect", "gi-upset", 1),
            MOIETY_SYSTEM,
            AMOX,
        ),
    ]);
    let status = HashMap::from([
        (id(PEN_GROUP), "structural-seed".to_string()),
        (id(CEPH_GROUP), "reviewed".to_string()),
    ]);
    let unreviewed = unreviewed_groups(&resolve(&a), &status);
    assert_eq!(unreviewed, HashSet::from([id(PEN_GROUP)]));

    let before = build_view(&a, bne());
    let after = note_unreviewed_groups(before.clone(), &a, &unreviewed);
    assert_eq!(after.strip, before.strip, "the strip is untouched (M7)");
    let changed: Vec<(&str, &str)> = before
        .drug_allergies
        .iter()
        .chain(&before.adverse_effects)
        .zip(after.drug_allergies.iter().chain(&after.adverse_effects))
        .filter(|(b, a)| b.text != a.text)
        .map(|(b, a)| (b.text.as_str(), a.text.as_str()))
        .collect();
    assert_eq!(changed.len(), 1, "{changed:?}");
    let (b, a) = changed[0];
    assert!(b.contains("penicillin"));
    assert_eq!(a, format!("{b} (drug group not clinically reviewed)"));
}

#[test]
fn an_unknown_review_word_reads_as_not_reviewed() {
    let a = only(vec![coded(
        entry(1, "drug", "penicillin", "allergy", "rash", 1),
        GROUP_SYSTEM,
        PEN_GROUP,
    )]);
    let r = resolve(&a);
    let with = |word: &str| HashMap::from([(id(PEN_GROUP), word.to_string())]);
    let not_reviewed = HashSet::from([id(PEN_GROUP)]);
    assert_eq!(
        unreviewed_groups(&r, &with("peer-reviewed-v2")),
        not_reviewed
    );
    // A failed status read is an empty map (D11).
    assert_eq!(unreviewed_groups(&r, &HashMap::new()), not_reviewed);
    assert!(unreviewed_groups(&r, &with("reviewed")).is_empty());
}

/// Review Focus 5: a salt of the allergen is a different code; with no drug list nothing can
/// say they are one drug, so the line is never silently clear.
#[test]
fn a_salt_without_the_drug_list_is_not_checked() {
    let a = only(vec![amox("anaphylaxis")]);
    let line = [med(900, "amoxicillin sodium", Some(AMOX_SODIUM))];
    assert_eq!(
        run(&a, &line, &DrugFacts::Unavailable)[&id(900)],
        not_checked()
    );
}

/// Ruling T4-g: a code under a system this build cannot ask about is as good as uncoded, even
/// when an answer row happens to carry the same uuid.
#[test]
fn an_allergen_coded_under_another_system_is_not_checked() {
    let a = only(vec![coded(
        entry(1, "drug", "amoxicillin", "allergy", "rash", 1),
        "a-peers-newer-system",
        AMOX,
    )]);
    let f = facts(vec![(AMOX, PARACETAMOL, Answer::NoMatch)]);
    let meds = [med(900, "paracetamol", Some(PARACETAMOL))];
    assert_eq!(run(&a, &meds, &f)[&id(900)], not_checked());
    assert!(codes_to_ask(&resolve(&a), &meds).0.is_empty());
}

/// #1059: an allergy record this terminal cannot read (a body it cannot decrypt, a peer's newer
/// type) has no head, so nothing would pair with a line and every line would read clean while the
/// strip says a record cannot be read. The unread record may be the allergen: every current line
/// with no mark reads "not checked"; a ceased line is still never checked, and a marked line
/// keeps its mark and no tag.
#[test]
fn an_unreadable_record_leaves_every_current_line_not_checked() {
    let mut a = only(vec![]);
    a.unreadable = 1;
    let ceased = MedLine {
        current: false,
        ..med(901, "amoxicillin", Some(AMOX))
    };
    let meds = [med(900, "paracetamol", Some(PARACETAMOL)), ceased];
    let got = run(&a, &meds, &facts(vec![]));
    assert_eq!(got[&id(900)], not_checked());
    assert!(
        !got.contains_key(&id(901)),
        "a ceased line is never checked"
    );

    // A readable allergy that marks the line still marks it, with no tag beside it.
    let mut a = only(vec![pen(1, "rash", 1)]);
    a.unreadable = 1;
    let got = run(&a, &[med(900, "penicillin", None)], &DrugFacts::Unavailable);
    assert_eq!(
        got[&id(900)],
        marks(&["Allergy recorded: penicillin → rash"])
    );
}
