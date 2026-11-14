use super::*;
use crate::allergy::check::WarningThreshold;
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
        set_aside: false,
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
        set_aside: false,
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
        set_aside: false,
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
    assert!(hand_tick_holds(&marked(&[u(1)]), &seen(&[u(1), u(2)])));
    assert!(hand_tick_holds(&LineCheck::default(), &[]));
}

#[test]
fn a_hand_tick_fails_on_a_new_one() {
    assert!(!hand_tick_holds(&marked(&[u(1), u(2)]), &seen(&[u(1)])));
    // A ref from another source is not the same warning.
    let other = vec![("other".to_string(), u(1))];
    assert!(!hand_tick_holds(&marked(&[u(1)]), &other));
}

/// Ruling R3: the reason rules are per-line refusals at Sign, never a tick that fails. A check
/// that could not run finds no new warning; a seen mark keeps the tick with or without a reason.
#[test]
fn the_reason_rules_do_not_judge_the_tick() {
    let unchecked = LineCheck {
        unverified: true,
        ..Default::default()
    };
    assert!(hand_tick_holds(&unchecked, &[]));
    assert!(hand_tick_holds(&marked(&[u(1)]), &seen(&[u(1)])));
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

#[test]
fn a_set_aside_line_is_never_default_ticked() {
    let (d, c) = (read_directions("500 mg cap tds"), LineCheck::default());
    let f = TickFacts {
        finished: true,
        all_parts_coded: true,
        directions: &d,
        check: &c,
        drug_allergies: &ListStatus::Entries,
        duplicate: false,
        named_writer: true,
        set_aside: true,
    };
    assert!(!default_tick(&f));
    assert!(default_tick(&TickFacts {
        set_aside: false,
        ..f
    }));
}

/// A check carrying one adverse-effect note from record `u(9)`, at `threshold`.
fn adverse(threshold: WarningThreshold) -> LineCheck {
    LineCheck {
        notes: vec!["Previous adverse effect: codeine → GI upset".into()],
        adverse_against: vec![u(9)],
        threshold,
        ..Default::default()
    }
}

/// Slice 3 (ADR-0104): at a lowered threshold an adverse effect is a warning, so it withholds
/// the default tick as a mark does; at the default it still does not.
#[test]
fn a_lowered_threshold_makes_an_adverse_effect_withhold_the_tick() {
    assert!(tick(
        |_, c, _, _, _| *c = adverse(WarningThreshold::AllergyHit)
    ));
    assert!(!tick(
        |_, c, _, _, _| *c = adverse(WarningThreshold::AdverseEffect)
    ));
}

/// Review focus 4: a hand tick made before the administrator lowered the threshold saw no
/// adverse-effect warning, so at Sign the world has moved under it; one that saw it holds.
#[test]
fn a_hand_tick_must_have_seen_a_lowered_adverse_effect() {
    assert!(hand_tick_holds(&adverse(WarningThreshold::AllergyHit), &[]));
    assert!(!hand_tick_holds(
        &adverse(WarningThreshold::AdverseEffect),
        &[]
    ));
    assert!(hand_tick_holds(
        &adverse(WarningThreshold::AdverseEffect),
        &seen(&[u(9)])
    ));
}
