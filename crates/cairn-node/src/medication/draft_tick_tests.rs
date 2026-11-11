use super::*;
use crate::medication::line::read_directions;
use chrono::NaiveDate;

fn u(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

/// Builds facts for a clean line, lets the test break one thing, and returns the default tick.
fn tick(
    edit: impl FnOnce(&mut Directions, &mut LineCheck, &mut ListStatus, &mut bool, &mut bool),
) -> bool {
    let mut d = read_directions("500 mg cap tds");
    let mut c = LineCheck::default();
    let mut a = ListStatus::Entries;
    let (mut finished, mut coded) = (true, true);
    edit(&mut d, &mut c, &mut a, &mut finished, &mut coded);
    default_tick(&TickFacts {
        finished,
        all_parts_coded: coded,
        directions: &d,
        check: &c,
        drug_allergies: &a,
        duplicate: false,
        named_writer: true,
    })
}

#[test]
fn a_complete_clean_finished_line_is_ticked() {
    assert!(tick(|_, _, _, _, _| {}));
    assert!(tick(|_, _, a, _, _| *a = ListStatus::NoneKnown {
        asked: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()
    }));
}

#[test]
fn unfinished_is_never_ticked() {
    assert!(!tick(|_, _, _, f, _| *f = false));
}

#[test]
fn each_missing_part_withholds_the_tick() {
    assert!(!tick(|d, _, _, _, _| d.form = None));
    assert!(!tick(|d, _, _, _, _| d.dose_unit = None));
    assert!(!tick(|d, _, _, _, _| d.dose_amount = None));
    assert!(!tick(|d, _, _, _, _| d.frequency = None));
    assert!(tick(|d, _, _, _, _| {
        d.frequency = None;
        d.prn = true;
    }));
    assert!(tick(|d, _, _, _, _| {
        d.frequency = None;
        d.stat = true;
    }));
    assert!(!tick(|_, _, _, _, c| *c = false));
}

#[test]
fn not_checked_withholds_the_tick() {
    assert!(!tick(|_, c, _, _, _| c.unverified = true));
}

#[test]
fn a_mark_withholds_the_tick() {
    assert!(!tick(|_, c, _, _, _| {
        c.marks.push("penicillin allergy".into());
        c.against.push(u(1));
    }));
}

#[test]
fn an_adverse_effect_note_alone_does_not() {
    assert!(tick(|_, c, _, _, _| c
        .notes
        .push("rash on a past course".into())));
}

#[test]
fn allergies_not_yet_asked_withholds_every_default_tick() {
    assert!(!tick(|_, _, a, _, _| *a = ListStatus::NotYetAsked));
}

#[test]
fn a_duplicate_withholds_the_tick() {
    let (d, c) = (read_directions("500 mg cap tds"), LineCheck::default());
    let f = TickFacts {
        finished: true,
        all_parts_coded: true,
        directions: &d,
        check: &c,
        drug_allergies: &ListStatus::Entries,
        duplicate: true,
        named_writer: true,
    };
    assert!(!default_tick(&f));
    assert!(default_tick(&TickFacts {
        duplicate: false,
        ..f
    }));
}

#[test]
fn a_list_with_no_named_writer_is_never_default_ticked() {
    let (d, c) = (read_directions("500 mg cap tds"), LineCheck::default());
    let f = TickFacts {
        finished: true,
        all_parts_coded: true,
        directions: &d,
        check: &c,
        drug_allergies: &ListStatus::Entries,
        duplicate: false,
        named_writer: false,
    };
    assert!(!default_tick(&f));
}

fn marked(ids: &[Uuid]) -> LineCheck {
    LineCheck {
        marks: ids.iter().map(|_| "allergy".to_string()).collect(),
        against: ids.to_vec(),
        ..Default::default()
    }
}

fn seen(ids: &[Uuid]) -> Vec<(String, Uuid)> {
    ids.iter()
        .map(|i| (ALLERGY_SOURCE.to_string(), *i))
        .collect()
}

#[test]
fn a_hand_tick_holds_against_the_warnings_it_saw() {
    assert!(hand_tick_holds(
        ChartedAs::PreExisting,
        &marked(&[u(1)]),
        &seen(&[u(1), u(2)]),
        None
    ));
    assert!(hand_tick_holds(
        ChartedAs::StartedHere,
        &LineCheck::default(),
        &[],
        None
    ));
}

#[test]
fn a_hand_tick_fails_on_a_new_one() {
    assert!(!hand_tick_holds(
        ChartedAs::PreExisting,
        &marked(&[u(1), u(2)]),
        &seen(&[u(1)]),
        None
    ));
    // A ref from another source is not the same warning.
    let other = vec![("other".to_string(), u(1))];
    assert!(!hand_tick_holds(
        ChartedAs::PreExisting,
        &marked(&[u(1)]),
        &other,
        None
    ));
}

#[test]
fn a_hand_tick_with_a_reason_fails_when_the_check_did_not_run() {
    let unchecked = LineCheck {
        unverified: true,
        ..Default::default()
    };
    assert!(hand_tick_holds(
        ChartedAs::StartedHere,
        &unchecked,
        &[],
        None
    ));
    assert!(hand_tick_holds(
        ChartedAs::StartedHere,
        &unchecked,
        &[],
        Some("  ")
    ));
    assert!(!hand_tick_holds(
        ChartedAs::StartedHere,
        &unchecked,
        &[],
        Some("benefit outweighs")
    ));
}

#[test]
fn a_start_hand_tick_on_a_mark_needs_a_reason() {
    let m = marked(&[u(1)]);
    let s = seen(&[u(1)]);
    assert!(!hand_tick_holds(ChartedAs::StartedHere, &m, &s, None));
    assert!(!hand_tick_holds(ChartedAs::StartedHere, &m, &s, Some("  ")));
    assert!(hand_tick_holds(
        ChartedAs::StartedHere,
        &m,
        &s,
        Some("tolerated before")
    ));
    assert!(hand_tick_holds(ChartedAs::PreExisting, &m, &s, None));
}

#[test]
fn duplicates_count_the_chart_and_the_list() {
    let got = duplicate_codes(&[Some("a"), Some("a"), Some("b"), Some("c")], &["b", "z"]);
    assert_eq!(got, ["a", "b"].iter().map(|s| s.to_string()).collect());
    // Two chart lines alone are not this function's business.
    assert!(duplicate_codes(&[Some("a")], &["z", "z"]).is_empty());
}

#[test]
fn combinations_and_uncoded_never_group() {
    assert!(duplicate_codes(&[None, None, Some("a")], &["q"]).is_empty());
}
