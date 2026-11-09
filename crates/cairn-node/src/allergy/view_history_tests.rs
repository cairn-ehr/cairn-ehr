//! Pure tests for the words of the allergy list's struck and historical lines added by the
//! slice-2 final review: who de-labelled a line (ADR-0097 decision 18), a withdrawal's reason and a de-label's
//! note wherever the act is worded, a withdrawn de-labelled head that still says its de-label
//! (decision 19), and a "none known" line's doubt flag (C1). No database; every string is a
//! clinician's exact words.
use super::*;
use crate::allergy::read::{ChallengeRow, KeyState};
use crate::allergy::test_fixtures::*;

fn view(p: &PatientAllergies) -> AllergyView {
    build_view(p, bne())
}
fn struck(ls: &[EnteredInError]) -> Vec<&str> {
    ls.iter().map(|l| l.text.as_str()).collect()
}

/// Decision 18: a historical line names who de-labelled it and when, as paper's initialled
/// "tolerated amoxil 03/22". The key's marker is worded INSIDE the bracket (after a comma), so
/// an unconfirmed de-labeller never nests brackets; with no name, the marker alone.
#[test]
fn a_historical_line_names_its_delabeller_in_one_bracket() {
    let line = |c: ChallengeRow| {
        let v = view(&pa(vec![pen(1, "rash", 8)], vec![], vec![c], vec![]));
        v.historical[0].text.clone()
    };
    assert_eq!(
        line(challenge(5, 1, 9)),
        "Historically reported: penicillin allergy. Challenged: tolerated amoxicillin Mar 2022 \
         (de-labelled 9 Nov 2026 by Dr Ada)"
    );
    let mut c = challenge(5, 1, 9);
    c.key = KeyState::Unconfirmed;
    c.signer = Some("Dr Bea".into());
    assert_eq!(
        line(c.clone()),
        "Historically reported: penicillin allergy. Challenged: tolerated amoxicillin Mar 2022 \
         (de-labelled 9 Nov 2026 by Dr Bea, signer unconfirmed)"
    );
    c.signer = None;
    assert_eq!(
        line(c),
        "Historically reported: penicillin allergy. Challenged: tolerated amoxicillin Mar 2022 \
         (de-labelled 9 Nov 2026, signer unconfirmed)"
    );
}

/// The ledger's ruling: a withdrawal's reason is shown wherever the withdrawal is worded — on
/// its "Entered in error:" line, and on an older version's "Earlier:" tail (R-4c).
#[test]
fn a_withdrawals_reason_is_shown_where_it_is_worded() {
    let mut w = retraction(6, 1, 10);
    w.text = Some("  wrong patient ".into());
    let v = view(&pa(
        vec![pen(1, "anaphylaxis", 8)],
        vec![],
        vec![],
        vec![w.clone()],
    ));
    assert_eq!(
        struck(&v.entered_in_error),
        ["Entered in error: Allergy: penicillin → anaphylaxis (withdrawn 10 Nov 2026 by Dr Ada) \
          — wrong patient"]
    );
    // A withdrawn de-label, and a withdrawn "none known" statement.
    let mut back = retraction(6, 5, 10);
    back.text = Some("challenge was another patient's".into());
    let mut c = challenge(5, 1, 9);
    c.text = Some("oral challenge in clinic".into());
    let v = view(&pa(vec![pen(1, "rash", 8)], vec![], vec![c], vec![back]));
    assert_eq!(
        struck(&v.entered_in_error),
        [
            "Entered in error: de-label — tolerated amoxicillin — oral challenge in clinic \
          (withdrawn 10 Nov 2026 by Dr Ada) — challenge was another patient's"
        ]
    );
    let mut w = retraction(6, 1, 9);
    w.text = Some("asked the wrong patient".into());
    let v = view(&pa(vec![], vec![none(1, "drug", 8)], vec![], vec![w]));
    assert_eq!(
        struck(&v.entered_in_error),
        [
            "Entered in error: no known drug allergies (asked 8 Nov 2026 by Dr Ada) \
          — asked the wrong patient"
        ]
    );
    // On an older version's tail.
    let entries = || vec![pen(1, "rash", 8), correcting(pen(2, "anaphylaxis", 9), 1)];
    let mut w = retraction(6, 1, 10);
    w.text = Some("not this patient's".into());
    let v = view(&pa(entries(), vec![], vec![], vec![w]));
    assert_eq!(
        v.drug_allergies[0].earlier,
        [
            "Earlier: Allergy: penicillin → rash (recorded 8 Nov 2026 by Dr Ada) \
          — entered in error 10 Nov 2026 by Dr Ada — not this patient's"
        ]
    );
    // A blank reason adds nothing.
    let mut w = retraction(6, 1, 10);
    w.text = Some("   ".into());
    let v = view(&pa(entries(), vec![], vec![], vec![w]));
    assert_eq!(
        v.drug_allergies[0].earlier,
        [
            "Earlier: Allergy: penicillin → rash (recorded 8 Nov 2026 by Dr Ada) \
          — entered in error 10 Nov 2026 by Dr Ada"
        ]
    );
}

/// The ledger's ruling: a de-label's note is shown on an older version's "Earlier:" tail too.
#[test]
fn a_delabels_note_is_shown_on_an_earlier_tail() {
    let mut c = challenge(5, 1, 10);
    c.text = Some("oral challenge in clinic".into());
    let v = view(&pa(
        vec![pen(1, "rash", 8), correcting(pen(2, "anaphylaxis", 9), 1)],
        vec![],
        vec![c],
        vec![],
    ));
    assert_eq!(
        v.drug_allergies[0].earlier,
        [
            "Earlier: Allergy: penicillin → rash (recorded 8 Nov 2026 by Dr Ada) \
          — de-labelled 10 Nov 2026 by Dr Ada: tolerated amoxicillin Mar 2022 \
          — oral challenge in clinic"
        ]
    );
}

/// Decision 19: a de-labelled line struck as entered in error (the window's one act, aimed at its
/// head entry) still says the de-label it carried, so no signed act vanishes; it is off the
/// historical list and the strip.
#[test]
fn a_withdrawn_delabelled_line_still_says_its_delabel() {
    let mut c = challenge(5, 1, 9);
    c.text = Some("oral challenge in clinic".into());
    let mut w = retraction(6, 1, 10);
    w.text = Some("wrong patient".into());
    let v = view(&pa(vec![pen(1, "rash", 8)], vec![], vec![c], vec![w]));
    assert!(v.historical.is_empty() && v.drug_allergies.is_empty());
    assert_eq!(v.strip, "Allergies not yet asked");
    assert_eq!(
        struck(&v.entered_in_error),
        [
            "Entered in error: Allergy: penicillin → rash (withdrawn 10 Nov 2026 by Dr Ada) \
          — wrong patient — de-labelled 9 Nov 2026 by Dr Ada: tolerated amoxicillin Mar 2022 \
          — oral challenge in clinic"
        ]
    );
}

/// C1: a "none known" line carries whether its chart's link is in doubt, so the window draws
/// no Entered-in-error button on a possible stranger's statement.
#[test]
fn a_none_line_says_when_its_chart_is_doubted() {
    let mut theirs = none(2, "drug", 8);
    theirs.chart = id(2000);
    let mut p = pa(vec![], vec![none(1, "all", 7), theirs], vec![], vec![]);
    p.doubted_charts = vec![id(2000)];
    let v = view(&p);
    assert_eq!(v.standing_none.len(), 1);
    assert!(!v.standing_none[0].doubted_chart);
    assert_eq!(v.previously_none.len(), 1);
    assert_eq!(v.previously_none[0].event_id, id(2));
    assert!(v.previously_none[0].doubted_chart);
}
