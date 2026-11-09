//! Pure tests for the view over corrections, de-labels, withdrawals and differing accounts
//! (allergy slice 2). No database. Every string here is a clinician's exact words.
use super::*;
use crate::allergy::read::KeyState;
use crate::allergy::test_fixtures::*;

fn view(p: &PatientAllergies) -> AllergyView {
    build_view(p, bne())
}
fn texts(ls: &[EntryLine]) -> Vec<&str> {
    ls.iter().map(|l| l.text.as_str()).collect()
}
fn struck(ls: &[EnteredInError]) -> Vec<&str> {
    ls.iter().map(|l| l.text.as_str()).collect()
}

#[test]
fn a_correction_replaces_its_line_and_keeps_the_older_as_earlier() {
    let v = view(&pa(
        vec![pen(1, "rash", 8), correcting(pen(2, "anaphylaxis", 9), 1)],
        vec![],
        vec![],
        vec![],
    ));
    assert_eq!(
        texts(&v.drug_allergies),
        ["Allergy: penicillin → anaphylaxis"]
    );
    assert_eq!(
        v.drug_allergies[0].earlier,
        ["Earlier: Allergy: penicillin → rash (recorded 8 Nov 2026 by Dr Ada)"]
    );
    assert_eq!(v.drug_allergies[0].challenge, None);
    assert_eq!(v.strip, "Allergy: penicillin → anaphylaxis");
}

/// The metformin case: an intolerance signed as an allergy, corrected to an adverse effect. It
/// leaves the strip, and the NKDA it had overridden stands again.
#[test]
fn correcting_an_allergy_to_an_adverse_effect_moves_it_off_the_strip() {
    let mut a = entry(1, "drug", "metformin", "allergy", "other", 8);
    a.reaction_text = Some("loose bowel motions".into());
    let mut b = correcting(
        entry(2, "drug", "metformin", "adverse-effect", "other", 9),
        1,
    );
    b.reaction_text = Some("loose bowel motions".into());
    let v = view(&pa(vec![a, b], vec![none(5, "drug", 7)], vec![], vec![]));
    assert!(v.drug_allergies.is_empty());
    assert_eq!(
        texts(&v.adverse_effects),
        ["Previous adverse effect: metformin → loose bowel motions"]
    );
    assert_eq!(
        v.adverse_effects[0].earlier,
        ["Earlier: Allergy: metformin → loose bowel motions (recorded 8 Nov 2026 by Dr Ada)"]
    );
    assert_eq!(
        v.strip,
        "NKDA (asked 7 Nov 2026) · other allergies: not yet asked · +1 adverse effect"
    );
}

#[test]
fn a_delabelled_line_reads_historically_reported() {
    let mut c = challenge(5, 1, 9);
    c.text = Some("oral challenge in clinic".into());
    let v = view(&pa(vec![pen(1, "rash", 8)], vec![], vec![c], vec![]));
    assert!(v.drug_allergies.is_empty());
    assert_eq!(
        texts(&v.historical),
        ["Historically reported: penicillin allergy. Challenged: tolerated amoxicillin Mar 2022 — oral challenge in clinic"]
    );
    assert!(!v.historical[0].prominent);
    assert_eq!(v.historical[0].challenge, Some(id(5)));
    assert_eq!(v.strip, "Allergies not yet asked · +1 historical");

    // A day, an unknown date, an adverse effect, and a newer peer's kind.
    let mut c = challenge(5, 1, 9);
    (c.date_precision, c.date_value) = ("day".into(), Some("2022-03-14".into()));
    let v = view(&pa(vec![pen(1, "rash", 8)], vec![], vec![c], vec![]));
    assert_eq!(
        texts(&v.historical),
        ["Historically reported: penicillin allergy. Challenged: tolerated amoxicillin 14 Mar 2022"]
    );
    let mut c = challenge(5, 1, 9);
    (c.date_precision, c.date_value) = ("not-known".into(), None);
    c.tolerated = "metformin".into();
    let ae = entry(1, "drug", "metformin", "adverse-effect", "gi-upset", 8);
    let v = view(&pa(vec![ae], vec![], vec![c.clone()], vec![]));
    assert_eq!(
        texts(&v.historical),
        ["Historically reported: metformin adverse effect. Challenged: tolerated metformin"]
    );
    assert_eq!(v.strip, "Allergies not yet asked · +1 historical");
    let odd = entry(1, "drug", "metformin", "intolerance", "gi-upset", 8);
    let v = view(&pa(vec![odd], vec![], vec![c], vec![]));
    assert_eq!(
        texts(&v.historical),
        ["Historically reported: metformin intolerance. Challenged: tolerated metformin"]
    );
}

/// Review Focus 1: a de-labelled allergy overrides nothing, so "No known allergies" stands.
#[test]
fn a_delabelled_allergy_overrides_no_none_statement() {
    let v = view(&pa(
        vec![pen(1, "anaphylaxis", 8)],
        vec![none(2, "all", 7)],
        vec![challenge(5, 1, 9)],
        vec![],
    ));
    assert_eq!(
        v.drug,
        ListStatus::NoneKnown {
            asked: at(7).date_naive()
        }
    );
    assert_eq!(
        v.strip,
        "No known allergies (asked 7 Nov 2026) · +1 historical"
    );
    assert_eq!(
        v.standing_none,
        ["No known allergies — asked 7 Nov 2026 by Dr Ada"]
    );
    assert_eq!(v.standing_none[0].event_id, id(2));
    assert!(v.previously_none.is_empty());
}

/// M3: withdrawing the de-label puts the allergy back in force.
#[test]
fn withdrawing_a_delabel_puts_the_allergy_back() {
    let v = view(&pa(
        vec![pen(1, "anaphylaxis", 8)],
        vec![],
        vec![challenge(5, 1, 9)],
        vec![retraction(6, 5, 10)],
    ));
    assert!(v.historical.is_empty());
    assert_eq!(
        texts(&v.drug_allergies),
        ["Allergy: penicillin → anaphylaxis"]
    );
    assert!(v.drug_allergies[0].prominent);
    assert_eq!(v.strip, "Allergy: penicillin → anaphylaxis");
    assert_eq!(
        struck(&v.entered_in_error),
        ["Entered in error: de-label — tolerated amoxicillin (withdrawn 10 Nov 2026 by Dr Ada)"]
    );
}

#[test]
fn a_withdrawn_entry_leaves_the_strip_and_reads_entered_in_error() {
    let v = view(&pa(
        vec![pen(1, "anaphylaxis", 8)],
        vec![],
        vec![],
        vec![retraction(6, 1, 10)],
    ));
    assert!(v.drug_allergies.is_empty() && v.historical.is_empty());
    assert_eq!(v.drug, ListStatus::NotYetAsked);
    assert_eq!(v.strip, "Allergies not yet asked");
    assert_eq!(
        struck(&v.entered_in_error),
        ["Entered in error: Allergy: penicillin → anaphylaxis (withdrawn 10 Nov 2026 by Dr Ada)"]
    );
}

/// R-4a: a withdrawal covers the version it names and the versions that one replaced.
#[test]
fn withdrawing_a_version_covers_it_and_what_it_replaced() {
    let v = view(&pa(
        vec![pen(1, "rash", 8), correcting(pen(2, "anaphylaxis", 9), 1)],
        vec![],
        vec![],
        vec![retraction(6, 2, 11)],
    ));
    assert!(v.drug_allergies.is_empty() && v.historical.is_empty());
    assert_eq!(v.strip, "Allergies not yet asked");
    assert_eq!(
        struck(&v.entered_in_error),
        ["Entered in error: Allergy: penicillin → anaphylaxis (withdrawn 11 Nov 2026 by Dr Ada)"]
    );
}

/// R-4a: withdrawing A never strikes a later correction of it. Dr Y, offline, strikes the rash
/// line while Dr X has corrected it to anaphylaxis: the anaphylaxis stays on the strip.
#[test]
fn withdrawing_an_older_version_leaves_a_later_correction_live() {
    let v = view(&pa(
        vec![pen(1, "rash", 8), correcting(pen(2, "anaphylaxis", 9), 1)],
        vec![],
        vec![],
        vec![retraction(6, 1, 10)],
    ));
    assert_eq!(
        texts(&v.drug_allergies),
        ["Allergy: penicillin → anaphylaxis"]
    );
    assert!(v.drug_allergies[0].prominent);
    assert_eq!(
        v.drug_allergies[0].earlier,
        ["Earlier: Allergy: penicillin → rash (recorded 8 Nov 2026 by Dr Ada) — entered in error 10 Nov 2026 by Dr Ada"]
    );
    assert_eq!(v.strip, "Allergy: penicillin → anaphylaxis");
}

/// R-4a: a correction made after a de-label of its target is a later version the de-label did
/// not name: it reads live again (a de-label covers what it names and what that replaced).
#[test]
fn a_correction_after_a_delabel_reads_live_again() {
    let v = view(&pa(
        vec![pen(1, "rash", 8), correcting(pen(2, "urticaria", 10), 1)],
        vec![],
        vec![challenge(5, 1, 9)],
        vec![],
    ));
    assert!(v.historical.is_empty());
    assert_eq!(
        texts(&v.drug_allergies),
        ["Allergy: penicillin → urticaria"]
    );
    assert_eq!(
        v.drug_allergies[0].earlier,
        ["Earlier: Allergy: penicillin → rash (recorded 8 Nov 2026 by Dr Ada) — de-labelled 9 Nov 2026 by Dr Ada: tolerated amoxicillin Mar 2022"]
    );
    assert_eq!(v.strip, "Allergy: penicillin → urticaria");
}

#[test]
fn a_withdrawn_none_statement_stands_for_nothing() {
    let v = view(&pa(
        vec![],
        vec![none(1, "all", 8), none(2, "drug", 8)],
        vec![],
        vec![retraction(6, 1, 9), retraction(7, 2, 9)],
    ));
    assert_eq!(v.strip, "Allergies not yet asked");
    assert!(v.standing_none.is_empty() && v.previously_none.is_empty());
    let mut eie = struck(&v.entered_in_error);
    eie.sort();
    assert_eq!(
        eie,
        [
            "Entered in error: no known allergies (asked 8 Nov 2026 by Dr Ada)",
            "Entered in error: no known drug allergies (asked 8 Nov 2026 by Dr Ada)",
        ]
    );
}

/// Review Focus 3: a repudiated key withdraws an anaphylaxis — it stays on the strip.
#[test]
fn a_repudiated_keys_withdrawal_takes_nothing_away() {
    let mut w = retraction(6, 1, 10);
    w.key = KeyState::Repudiated;
    let v = view(&pa(vec![pen(1, "anaphylaxis", 8)], vec![], vec![], vec![w]));
    assert_eq!(v.strip, "Allergy: penicillin → anaphylaxis");
    assert!(v.entered_in_error.is_empty());
    assert_eq!(
        v.ignored_overlays,
        ["Not counted: entered in error by Dr Ada (key had been repudiated)"]
    );
}

/// R-4b: a suspect withdrawal counts when it ADDS a warning — striking a de-label puts the
/// allergy back.
#[test]
fn a_repudiated_keys_withdrawal_of_a_delabel_counts() {
    let mut w = retraction(6, 5, 10);
    w.key = KeyState::Repudiated;
    let v = view(&pa(
        vec![pen(1, "anaphylaxis", 8)],
        vec![],
        vec![challenge(5, 1, 9)],
        vec![w],
    ));
    assert!(v.historical.is_empty() && v.ignored_overlays.is_empty());
    assert_eq!(v.strip, "Allergy: penicillin → anaphylaxis");
    assert_eq!(
        struck(&v.entered_in_error),
        ["Entered in error: de-label — tolerated amoxicillin (withdrawn 10 Nov 2026 by Dr Ada (key had been repudiated))"]
    );
}

/// R-4b: striking a "none known" statement takes a reassurance away, so a suspect one counts.
#[test]
fn a_repudiated_keys_withdrawal_of_a_none_statement_counts() {
    let mut w = retraction(6, 1, 9);
    w.key = KeyState::Repudiated;
    let v = view(&pa(vec![], vec![none(1, "all", 8)], vec![], vec![w]));
    assert_eq!(v.strip, "Allergies not yet asked");
    assert!(v.standing_none.is_empty() && v.ignored_overlays.is_empty());
    assert_eq!(
        struck(&v.entered_in_error),
        ["Entered in error: no known allergies (asked 8 Nov 2026 by Dr Ada)"]
    );
}

#[test]
fn a_cross_chart_withdrawal_of_a_none_statement_counts() {
    let mut w = retraction(6, 1, 9);
    w.chart = id(2000);
    let v = view(&pa(vec![], vec![none(1, "drug", 8)], vec![], vec![w]));
    assert_eq!(v.strip, "Allergies not yet asked");
    assert!(v.standing_none.is_empty() && v.ignored_overlays.is_empty());
    assert_eq!(
        struck(&v.entered_in_error),
        ["Entered in error: no known drug allergies (asked 8 Nov 2026 by Dr Ada)"]
    );
}

#[test]
fn a_repudiated_keys_none_statement_does_not_stand() {
    let mut n = none(1, "all", 8);
    n.key = KeyState::Repudiated;
    let v = view(&pa(vec![], vec![n], vec![], vec![]));
    assert_eq!(v.strip, "Allergies not yet asked");
    assert_eq!(v.drug, ListStatus::NotYetAsked);
    assert!(v.standing_none.is_empty());
    assert_eq!(
        v.previously_none,
        ["Previously recorded: no known allergies (asked 8 Nov 2026 by Dr Ada (key had been repudiated))"]
    );
    assert_eq!(v.previously_none[0].event_id, id(1));
}

/// ADR-0094 precedent: an unconfirmed signer's act counts, marked. A repudiated key's entry still
/// warns (D4), marked.
#[test]
fn an_unconfirmed_signer_counts_and_is_marked() {
    let mut a = pen(1, "anaphylaxis", 8);
    a.key = KeyState::Unconfirmed;
    let mut b = entry(2, "drug", "latex", "allergy", "rash", 8);
    b.key = KeyState::Repudiated;
    let mut n = none(3, "all", 7);
    n.key = KeyState::Unconfirmed;
    let v = view(&pa(vec![a, b], vec![n], vec![], vec![]));
    assert_eq!(
        v.strip,
        "Allergy: penicillin → anaphylaxis; Allergy: latex → rash · no known other allergies (asked 7 Nov 2026)"
    );
    let signers: Vec<_> = v
        .drug_allergies
        .iter()
        .map(|l| l.signer.as_deref())
        .collect();
    assert_eq!(
        signers,
        [
            Some("Dr Ada (signer unconfirmed)"),
            Some("Dr Ada (key had been repudiated)")
        ]
    );
    assert_eq!(
        v.standing_none,
        ["No known other allergies — asked 7 Nov 2026 by Dr Ada (signer unconfirmed)"]
    );

    // An unconfirmed signer's withdrawal counts.
    let mut w = retraction(6, 1, 10);
    w.key = KeyState::Unconfirmed;
    let v = view(&pa(vec![pen(1, "anaphylaxis", 8)], vec![], vec![], vec![w]));
    assert_eq!(
        struck(&v.entered_in_error),
        ["Entered in error: Allergy: penicillin → anaphylaxis (withdrawn 10 Nov 2026 by Dr Ada (signer unconfirmed))"]
    );
}

#[test]
fn the_strip_marks_differing_accounts() {
    let mut ae = entry(3, "drug", "Penicillin", "adverse-effect", "gi-upset", 8);
    ae.chart = id(2000);
    let v = view(&pa(
        vec![
            pen(1, "rash", 8),
            pen(2, "anaphylaxis", 9),
            ae,
            entry(4, "drug", "cefalexin", "allergy", "rash", 8),
        ],
        vec![],
        vec![],
        vec![],
    ));
    assert_eq!(
        v.strip,
        "Allergy: penicillin → anaphylaxis — differing accounts; \
         Allergy: penicillin → rash — differing accounts; Allergy: cefalexin → rash · +1 adverse effect"
    );
    assert_eq!(
        texts(&v.adverse_effects),
        ["Previous adverse effect: Penicillin → GI upset — differing accounts"]
    );
}

/// The strip's quiet counts: adverse effects, then historical, then unrecognised.
#[test]
fn the_historical_count_follows_the_adverse_effect_count() {
    let v = view(&pa(
        vec![
            pen(1, "rash", 8),
            entry(2, "drug", "statin", "adverse-effect", "other", 8),
            entry(3, "drug", "foo", "intolerance", "rash", 8),
            entry(4, "drug", "codeine", "allergy", "rash", 8),
            entry(5, "drug", "morphine", "allergy", "rash", 8),
        ],
        vec![],
        vec![challenge(6, 4, 9), challenge(7, 5, 9)],
        vec![],
    ));
    assert_eq!(
        v.strip,
        "Allergy: penicillin → rash · +1 adverse effect · +2 historical · +1 unrecognised entry"
    );
}

/// R-P3 (Review Focus 2): an overlay this terminal cannot read leaves its target live, and the
/// strip still leads with the count.
#[test]
fn an_unreadable_overlay_leaves_its_target_live() {
    let mut p = pa(vec![pen(1, "anaphylaxis", 8)], vec![], vec![], vec![]);
    p.unreadable = 1;
    let v = view(&p);
    assert_eq!(
        texts(&v.drug_allergies),
        ["Allergy: penicillin → anaphylaxis"]
    );
    assert_eq!(
        v.strip,
        "1 allergy record cannot be read on this terminal · Allergy: penicillin → anaphylaxis"
    );
}

/// R-4c: a counted overlay naming an older version is said on that version's "Earlier:" line —
/// the clinician's signed act never silently vanishes under a later correction.
#[test]
fn an_overlay_on_an_older_version_is_shown_on_its_earlier_line() {
    let entries = || vec![pen(1, "rash", 8), correcting(pen(2, "anaphylaxis", 9), 1)];
    let mut w = retraction(6, 1, 10);
    w.key = KeyState::Unconfirmed;
    let v = view(&pa(entries(), vec![], vec![], vec![w]));
    assert_eq!(
        v.drug_allergies[0].earlier,
        [
            "Earlier: Allergy: penicillin → rash (recorded 8 Nov 2026 by Dr Ada) \
          — entered in error 10 Nov 2026 by Dr Ada (signer unconfirmed)"
        ]
    );
    let v = view(&pa(entries(), vec![], vec![challenge(5, 1, 10)], vec![]));
    assert_eq!(
        v.drug_allergies[0].earlier,
        [
            "Earlier: Allergy: penicillin → rash (recorded 8 Nov 2026 by Dr Ada) \
          — de-labelled 10 Nov 2026 by Dr Ada: tolerated amoxicillin Mar 2022"
        ]
    );
    // An unknown challenge date is left out.
    let mut c = challenge(5, 1, 10);
    (c.date_precision, c.date_value) = ("not-known".into(), None);
    let v = view(&pa(entries(), vec![], vec![c], vec![]));
    assert_eq!(
        v.drug_allergies[0].earlier,
        [
            "Earlier: Allergy: penicillin → rash (recorded 8 Nov 2026 by Dr Ada) \
          — de-labelled 10 Nov 2026 by Dr Ada: tolerated amoxicillin"
        ]
    );
}

/// Review minor 1: withdrawing B in A←B strikes both, and A stays legible under B's line.
#[test]
fn a_withdrawn_lines_earlier_versions_stay_legible() {
    let v = view(&pa(
        vec![pen(1, "rash", 8), correcting(pen(2, "anaphylaxis", 9), 1)],
        vec![],
        vec![],
        vec![retraction(6, 2, 11)],
    ));
    assert_eq!(v.entered_in_error.len(), 1);
    assert_eq!(
        v.entered_in_error[0].earlier,
        ["Earlier: Allergy: penicillin → rash (recorded 8 Nov 2026 by Dr Ada)"]
    );
}

#[test]
fn a_withdrawn_entry_on_a_doubted_chart_keeps_its_mark() {
    let mut e = pen(1, "anaphylaxis", 8);
    e.chart = id(2000);
    let mut w = retraction(6, 1, 10);
    w.chart = id(2000);
    let mut p = pa(vec![e], vec![], vec![], vec![w]);
    p.doubted_charts = vec![id(2000)];
    let v = view(&p);
    assert_eq!(
        struck(&v.entered_in_error),
        ["Entered in error: Allergy: penicillin → anaphylaxis \
          (on a chart whose link to this one is in doubt) (withdrawn 10 Nov 2026 by Dr Ada)"]
    );
}

/// A peer's newer allergen kind reads on a de-labelled line as on a live one (" (food)").
#[test]
fn a_historical_line_keeps_a_newer_allergen_kind() {
    let mut c = challenge(5, 1, 9);
    (c.date_precision, c.date_value) = ("not-known".into(), None);
    c.tolerated = "peanut".into();
    let peanut = entry(1, "food", "peanut", "allergy", "anaphylaxis", 8);
    let v = view(&pa(vec![peanut], vec![], vec![c], vec![]));
    assert_eq!(
        texts(&v.historical),
        ["Historically reported: peanut allergy (food). Challenged: tolerated peanut"]
    );
}
