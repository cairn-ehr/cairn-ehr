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
