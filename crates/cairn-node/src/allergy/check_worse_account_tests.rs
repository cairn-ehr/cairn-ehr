//! #1131: the accounts a mark's words show beyond its hitting record ([`LineCheck::worse_accounts`])
//! count as warnings, so a hand tick made before a worse account arrived no longer holds.
//! Helpers are `check_tests`'s.
use super::tests::*;
use super::*;
use crate::allergy::drug_facts::DrugFacts;
use crate::allergy::test_fixtures::{coded, entry, id};

/// #1131: a hand tick binds to the records the clinician SAW. A more severe account of the same
/// allergen that arrives later changes the mark's words without hitting the line itself, so it
/// must still be a new warning, or a reason typed for a rash signs past an anaphylaxis.
#[test]
fn a_worse_account_arriving_later_unsettles_a_hand_tick() {
    use crate::medication::draft_tick::hand_tick_holds;
    use cairn_event::medication::ALLERGY_SOURCE;
    let rash = || {
        coded(
            entry(1, "drug", "Amoxil", "allergy", "rash", 1),
            MOIETY_SYSTEM,
            AMOX,
        )
    };
    let line = [med(900, "amoxicillin", Some(AMOX))];
    let before = run(&only(vec![rash()]), &line, &DrugFacts::Unavailable);
    let seen: Vec<(String, Uuid)> = before[&id(900)]
        .warned_against()
        .into_iter()
        .map(|r| (ALLERGY_SOURCE.to_string(), r))
        .collect();
    // An uncoded "Amoxil" anaphylaxis (a GP letter): same allergen by term, no hit of its own.
    let worse = entry(2, "drug", "Amoxil", "allergy", "anaphylaxis", 2);
    let after = run(&only(vec![rash(), worse]), &line, &DrugFacts::Unavailable);
    assert_eq!(
        after[&id(900)].marks,
        ["Allergy recorded: Amoxil → anaphylaxis"]
    );
    assert!(!hand_tick_holds(&after[&id(900)], &seen));
    // And the override names the account whose reaction the mark shows.
    assert!(after[&id(900)].warned_against().contains(&id(2)));
}

/// What a tick made on `check` saw, as `hand_tick_holds` reads it.
fn seen_at(check: &LineCheck) -> Vec<(String, Uuid)> {
    use cairn_event::medication::ALLERGY_SOURCE;
    check
        .warned_against()
        .into_iter()
        .map(|r| (ALLERGY_SOURCE.to_string(), r))
        .collect()
}

/// `check` judged at `threshold`, as the window's draft path stamps it.
fn at_threshold(mut check: LineCheck, threshold: WarningThreshold) -> LineCheck {
    check.threshold = threshold;
    check
}

/// #1131 review: the same rule for the adverse-effect notes, where a site lowered the threshold
/// to make them warn (ADR-0104). At the default threshold an adverse effect is no warning, so the
/// worse account changes nothing a tick saw.
#[test]
fn a_worse_adverse_effect_account_unsettles_a_tick_only_where_adverse_effects_warn() {
    use crate::medication::draft_tick::hand_tick_holds;
    let rash = || {
        coded(
            entry(1, "drug", "Amoxil", "adverse-effect", "rash", 1),
            MOIETY_SYSTEM,
            AMOX,
        )
    };
    let worse = entry(2, "drug", "Amoxil", "adverse-effect", "anaphylaxis", 2);
    let line = [med(900, "amoxicillin", Some(AMOX))];
    let before = run(&only(vec![rash()]), &line, &DrugFacts::Unavailable)[&id(900)].clone();
    let after = run(&only(vec![rash(), worse]), &line, &DrugFacts::Unavailable)[&id(900)].clone();
    assert_eq!(
        after.notes,
        ["Previous adverse effect: Amoxil → anaphylaxis"]
    );
    assert_eq!(after.adverse_worse_accounts, [id(2)]);

    let lowered = WarningThreshold::AdverseEffect;
    let seen = seen_at(&at_threshold(before.clone(), lowered));
    assert!(!hand_tick_holds(
        &at_threshold(after.clone(), lowered),
        &seen
    ));

    let default = WarningThreshold::AllergyHit;
    let seen = seen_at(&at_threshold(before, default));
    assert!(hand_tick_holds(&at_threshold(after, default), &seen));
}

/// #1131 review: a MILDER account arriving later changes no mark's words, so it is no new
/// warning: the tick stands (re-reasoning a rash beside a known anaphylaxis would cost the
/// clinician a step paper never asks for).
#[test]
fn a_milder_account_arriving_later_leaves_a_hand_tick_standing() {
    use crate::medication::draft_tick::hand_tick_holds;
    let anaphylaxis = || {
        coded(
            entry(1, "drug", "Amoxil", "allergy", "anaphylaxis", 1),
            MOIETY_SYSTEM,
            AMOX,
        )
    };
    let line = [med(900, "amoxicillin", Some(AMOX))];
    let before = run(&only(vec![anaphylaxis()]), &line, &DrugFacts::Unavailable);
    let milder = entry(2, "drug", "Amoxil", "allergy", "rash", 2);
    let after = run(
        &only(vec![anaphylaxis(), milder]),
        &line,
        &DrugFacts::Unavailable,
    );
    assert!(after[&id(900)].worse_accounts.is_empty());
    assert!(hand_tick_holds(
        &after[&id(900)],
        &seen_at(&before[&id(900)])
    ));
}

/// #1131 review: a combination's merge keeps the second component's worse accounts, even when its
/// mark is dropped as already said by the first (Augmentin's amoxicillin part can be `b`).
#[test]
fn a_merge_keeps_the_second_parts_worse_accounts() {
    let mark = |worse: Vec<Uuid>, adverse_worse: Vec<Uuid>| LineCheck {
        marks: vec!["Allergy recorded: Amoxil → anaphylaxis".into()],
        against: vec![id(1)],
        worse_accounts: worse,
        adverse_worse_accounts: adverse_worse,
        ..LineCheck::default()
    };
    let merged = merge(mark(vec![], vec![]), mark(vec![id(2)], vec![id(3)]));
    assert_eq!(merged.marks.len(), 1, "the same words, said once");
    assert_eq!(merged.warned_against(), [id(1), id(2)]);
    assert_eq!(
        at_threshold(merged, WarningThreshold::AdverseEffect).warned_against(),
        [id(1), id(2), id(3)]
    );
}
