//! The medication check (allergy slice 3, D7–D11): which current line gets a loud mark, a quiet
//! note, or the small "not checked" tag. Built on `test_fixtures`. Codes are `id(n)`, computed:
//! 500s are moieties, 600s drug groups, 900s medication lines.
use super::*;
use crate::allergy::drug_facts::{Answer, DrugFacts, MatchPath};
use crate::allergy::read::{KeyState, PatientAllergies};
use crate::allergy::test_fixtures::{bne, challenge, coded, entry, id, pa, pen, retraction};
use crate::allergy::thread::resolve;
use crate::allergy::view::build_view;

pub(super) const AMOX: u128 = 500;
const FLUCLOX: u128 = 501;
pub(super) const AMOX_SODIUM: u128 = 502;
const PARACETAMOL: u128 = 700;
const PEN_GROUP: u128 = 600;
const CEPH_GROUP: u128 = 601;

/// A current line whose name and term are both `name`.
pub(super) fn med(n: u128, name: &str, moiety: Option<u128>) -> MedLine<'_> {
    MedLine {
        group_id: id(n),
        name,
        term: name,
        coding_display: None,
        moiety: moiety.map(id),
        current: true,
    }
}

pub(super) fn only(entries: Vec<crate::allergy::read::AllergyRow>) -> PatientAllergies {
    pa(entries, vec![], vec![], vec![])
}

pub(super) fn run(
    a: &PatientAllergies,
    meds: &[MedLine],
    facts: &DrugFacts,
) -> HashMap<Uuid, LineCheck> {
    check_medications(&resolve(a), meds, facts)
}

fn facts(pairs: Vec<(u128, u128, Answer)>) -> DrugFacts {
    DrugFacts::Available(
        pairs
            .into_iter()
            .map(|(a, m, ans)| ((id(a), id(m)), ans))
            .collect(),
    )
}

fn path(kind: &str, group: Option<&str>) -> Answer {
    Answer::Match(vec![MatchPath {
        kind: kind.into(),
        group_name: group.map(Into::into),
    }])
}

pub(super) fn marks(marks: &[&str]) -> LineCheck {
    LineCheck {
        marks: marks.iter().map(|s| s.to_string()).collect(),
        ..LineCheck::default()
    }
}

pub(super) fn not_checked() -> LineCheck {
    LineCheck {
        unverified: true,
        ..LineCheck::default()
    }
}

pub(super) fn amox(cat: &str) -> crate::allergy::read::AllergyRow {
    coded(
        entry(1, "drug", "amoxicillin", "allergy", cat, 1),
        MOIETY_SYSTEM,
        AMOX,
    )
}

#[test]
fn same_name_marks_an_uncoded_line() {
    let a = only(vec![pen(1, "rash", 1)]);
    let by_display = MedLine {
        coding_display: Some("penicillin"),
        ..med(901, "Ospen", None)
    };
    let got = run(
        &a,
        &[med(900, " Penicillin ", None), by_display],
        &DrugFacts::Unavailable,
    );
    let want = marks(&["Allergy recorded: penicillin → rash"]);
    assert_eq!(got[&id(900)], want);
    assert_eq!(got[&id(901)], want);
}

#[test]
fn a_same_drug_answer_marks_the_line() {
    let a = only(vec![amox("urticaria")]);
    let f = facts(vec![(AMOX, AMOX_SODIUM, path("same-drug", None))]);
    let got = run(&a, &[med(900, "Moxacin", Some(AMOX_SODIUM))], &f);
    assert_eq!(
        got[&id(900)],
        marks(&["Allergy recorded: amoxicillin → urticaria"])
    );
}

#[test]
fn a_group_allergen_hit_names_the_group_allergen() {
    let a = only(vec![coded(
        entry(1, "drug", "penicillin", "allergy", "anaphylaxis", 1),
        GROUP_SYSTEM,
        PEN_GROUP,
    )]);
    let f = facts(vec![(
        PEN_GROUP,
        FLUCLOX,
        path("same-group", Some("Penicillins")),
    )]);
    let line = MedLine {
        name: "Flucloxacillin 500 mg capsule",
        ..med(900, "flucloxacillin", Some(FLUCLOX))
    };
    assert_eq!(
        run(&a, &[line], &f)[&id(900)],
        marks(&["Flucloxacillin 500 mg capsule is a penicillin; allergy recorded: anaphylaxis"])
    );
}

#[test]
fn a_moiety_allergen_same_group_hit_names_the_group() {
    let a = only(vec![amox("rash")]);
    let f = facts(vec![(
        AMOX,
        FLUCLOX,
        path("same-group", Some("Penicillins")),
    )]);
    assert_eq!(
        run(&a, &[med(900, "flucloxacillin", Some(FLUCLOX))], &f)[&id(900)],
        marks(&[
            "flucloxacillin is in the same group as amoxicillin (Penicillins); allergy recorded: rash"
        ])
    );
}

/// Ruling T4-a: a path that is not exactly `same-group`, or a group path with no group name,
/// still marks, worded as the same drug.
#[test]
fn a_newer_or_unnamed_path_reads_as_the_same_drug() {
    let a = only(vec![amox("rash")]);
    let want = marks(&["Allergy recorded: amoxicillin → rash"]);
    for answer in [
        path("", None),
        path("same-salt-family", Some("Penicillins")),
        path("same-group", None),
    ] {
        let f = facts(vec![(AMOX, FLUCLOX, answer)]);
        assert_eq!(
            run(&a, &[med(900, "flucloxacillin", Some(FLUCLOX))], &f)[&id(900)],
            want
        );
    }
}

#[test]
fn an_adverse_effect_is_a_note_never_a_mark() {
    let a = only(vec![entry(
        1,
        "drug",
        "codeine",
        "adverse-effect",
        "gi-upset",
        1,
    )]);
    let got = run(&a, &[med(900, "codeine", None)], &DrugFacts::Unavailable);
    assert_eq!(
        got[&id(900)],
        LineCheck {
            notes: vec!["Previous adverse effect: codeine → GI upset".into()],
            ..LineCheck::default()
        }
    );
}

#[test]
fn a_delabelled_or_withdrawn_entry_checks_nothing() {
    let a = pa(
        vec![pen(1, "anaphylaxis", 1), pen(2, "rash", 1)],
        vec![],
        vec![challenge(10, 1, 2)],
        vec![retraction(11, 2, 2)],
    );
    assert!(run(&a, &[med(900, "penicillin", None)], &DrugFacts::Unavailable).is_empty());
}

#[test]
fn a_ceased_line_is_never_checked() {
    let a = only(vec![pen(1, "anaphylaxis", 1)]);
    let ceased = |n, name| MedLine {
        current: false,
        ..med(n, name, None)
    };
    let meds = [ceased(900, "penicillin"), ceased(901, "paracetamol")];
    assert!(run(&a, &meds, &DrugFacts::Unavailable).is_empty());
}

#[test]
fn codes_to_ask_takes_live_drug_codes_and_current_moieties() {
    let a = only(vec![
        amox("rash"),
        coded(
            entry(2, "drug", "penicillin", "allergy", "rash", 1),
            GROUP_SYSTEM,
            PEN_GROUP,
        ),
        coded(
            entry(3, "drug", "amoxil", "adverse-effect", "rash", 1),
            MOIETY_SYSTEM,
            AMOX,
        ),
        pen(4, "rash", 1),
    ]);
    let ceased = MedLine {
        current: false,
        ..med(902, "ibuprofen", Some(PARACETAMOL + 1))
    };
    let meds = [
        med(900, "flucloxacillin", Some(FLUCLOX)),
        med(901, "flucloxacillin", Some(FLUCLOX)),
        med(903, "tonic", None),
        ceased,
    ];
    let (allergens, moieties) = codes_to_ask(&resolve(&a), &meds);
    assert_eq!(allergens, vec![id(AMOX), id(PEN_GROUP)]);
    assert_eq!(moieties, vec![id(FLUCLOX)]);
}

#[test]
fn differing_accounts_mark_once_with_the_more_severe() {
    // Two accounts of one allergen by name.
    let a = only(vec![pen(1, "rash", 1), pen(2, "anaphylaxis", 2)]);
    let got = run(&a, &[med(900, "penicillin", None)], &DrugFacts::Unavailable);
    assert_eq!(
        got[&id(900)],
        marks(&["Allergy recorded: penicillin → anaphylaxis"])
    );

    // The same allergen by code; the more severe account words the mark.
    let a = only(vec![
        coded(
            entry(1, "drug", "Augmentin", "allergy", "rash", 1),
            MOIETY_SYSTEM,
            AMOX,
        ),
        coded(
            entry(2, "drug", "amoxicillin", "allergy", "angioedema", 1),
            MOIETY_SYSTEM,
            AMOX,
        ),
    ]);
    let got = run(
        &a,
        &[med(900, "Amoxil", Some(AMOX))],
        &DrugFacts::Unavailable,
    );
    assert_eq!(
        got[&id(900)],
        marks(&["Allergy recorded: amoxicillin → angioedema"])
    );

    // The more severe account need not be the one that hit: an uncoded "Augmentin" anaphylaxis
    // is the same allergen (by term) as the coded "Augmentin" rash that met the line by code.
    let a = only(vec![
        coded(
            entry(1, "drug", "Augmentin", "allergy", "rash", 1),
            MOIETY_SYSTEM,
            AMOX,
        ),
        entry(2, "drug", "Augmentin", "allergy", "anaphylaxis", 1),
    ]);
    let got = run(
        &a,
        &[med(900, "Amoxil", Some(AMOX))],
        &DrugFacts::Unavailable,
    );
    assert_eq!(
        got[&id(900)],
        marks(&["Allergy recorded: Augmentin → anaphylaxis"])
    );
}

#[test]
fn not_known_ranks_with_anaphylaxis() {
    // The not-known account is OLDER, so recency is not what picks it.
    let a = only(vec![
        pen(1, "not-known", 1),
        pen(2, "angioedema", 2),
        pen(3, "rash", 3),
    ]);
    let got = run(&a, &[med(900, "penicillin", None)], &DrugFacts::Unavailable);
    assert_eq!(
        got[&id(900)],
        marks(&["Allergy recorded: penicillin → effect not known"])
    );
}

#[test]
fn a_doubted_charts_entry_still_marks() {
    let mut stranger = pen(1, "anaphylaxis", 1);
    stranger.chart = id(2000);
    let mut a = only(vec![stranger]);
    a.doubted_charts = vec![id(2000)];
    let mut repudiated = entry(2, "drug", "sulfonamide", "allergy", "rash", 1);
    repudiated.key = KeyState::Repudiated;
    a.entries.push(repudiated);
    let got = run(
        &a,
        &[med(900, "penicillin", None), med(901, "sulfonamide", None)],
        &DrugFacts::Unavailable,
    );
    assert_eq!(
        got[&id(900)].marks,
        ["Allergy recorded: penicillin → anaphylaxis"]
    );
    assert_eq!(
        got[&id(901)].marks,
        ["Allergy recorded: sulfonamide → rash"]
    );
}

#[test]
fn latex_never_touches_the_medication_list() {
    let a = only(vec![entry(
        1,
        "other",
        "latex",
        "allergy",
        "anaphylaxis",
        1,
    )]);
    let meds = [med(900, "latex", None), med(901, "paracetamol", None)];
    assert!(run(&a, &meds, &DrugFacts::Unavailable).is_empty());
}

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
    // `assert!`, not `assert_eq!`: the clinician words name the drug list, and only a
    // diagnostic macro's arguments may (no_drugref_dependency.rs).
    assert!(
        a == format!("{b} (group from drugref, not clinically reviewed)"),
        "{a}"
    );
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
