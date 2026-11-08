//! Pure tests for the allergy strip / list view. No database.
use super::*;
use crate::allergy::read::{AllergyRow, NoneRow, PatientAllergies};
use chrono::{DateTime, FixedOffset, TimeZone, Utc};

fn bne() -> FixedOffset {
    FixedOffset::east_opt(10 * 3600).unwrap()
}
fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}
/// Noon UTC on a day: the same calendar day at +10:00 (22:00).
fn at(d: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, d, 12, 0, 0).unwrap()
}
fn hlc(t: DateTime<Utc>) -> (i64, i32) {
    (t.timestamp_millis(), 0)
}
fn entry(n: u128, ak: &str, term: &str, kind: &str, cat: &str, day: u32) -> AllergyRow {
    AllergyRow {
        event_id: id(n),
        chart: id(1000),
        allergen_kind: ak.into(),
        term: term.into(),
        kind: kind.into(),
        category: cat.into(),
        reaction_text: None,
        onset_precision: "not-known".into(),
        onset_value: None,
        source: "seen-documented".into(),
        recorded_at: at(day),
        hlc: hlc(at(day)),
        signer: None,
    }
}
fn none(n: u128, scope: &str, day: u32) -> NoneRow {
    NoneRow {
        event_id: id(n),
        chart: id(1000),
        scope: scope.into(),
        recorded_at: at(day),
        hlc: hlc(at(day)),
        signer: None,
    }
}
fn pa(entries: Vec<AllergyRow>, none: Vec<NoneRow>) -> PatientAllergies {
    PatientAllergies {
        entries,
        none,
        doubted_charts: vec![],
        unreadable: 0,
    }
}
fn view(p: &PatientAllergies) -> AllergyView {
    build_view(p, bne())
}
fn strip(p: &PatientAllergies) -> String {
    view(p).strip
}

#[test]
fn nothing_recorded_says_not_yet_asked() {
    assert_eq!(strip(&pa(vec![], vec![])), "Allergies not yet asked");
}

#[test]
fn none_for_all_reads_no_known_allergies() {
    let v = view(&pa(vec![], vec![none(1, "all", 8)]));
    assert_eq!(v.strip, "No known allergies (asked 8 Nov 2026)");
    assert!(matches!(v.drug, ListStatus::NoneKnown { .. }));
    assert!(matches!(v.other, ListStatus::NoneKnown { .. }));
}

#[test]
fn none_for_drugs_leaves_other_not_yet_asked() {
    let v = view(&pa(vec![], vec![none(1, "drug", 8)]));
    assert_eq!(
        v.strip,
        "NKDA (asked 8 Nov 2026) · other allergies: not yet asked"
    );
    assert_eq!(v.other, ListStatus::NotYetAsked);
}

#[test]
fn nkda_plus_a_latex_entry() {
    let p = pa(
        vec![entry(2, "other", "latex", "allergy", "urticaria", 9)],
        vec![none(1, "drug", 8)],
    );
    assert_eq!(
        strip(&p),
        "NKDA (asked 8 Nov 2026) · Other: latex → urticaria"
    );
}

#[test]
fn the_maintainers_three_examples_read_exactly() {
    let mut a = entry(1, "drug", "penicillin", "allergy", "anaphylaxis", 8);
    a.onset_precision = "year".into();
    a.onset_value = Some("2014".into());
    assert_eq!(
        entry_line_text(&a),
        "Allergy: penicillin → anaphylaxis · 2014"
    );

    let mut b = entry(2, "drug", "penicillin", "allergy", "not-known", 8);
    b.source = "patient-stated".into();
    assert_eq!(
        entry_line_text(&b),
        "Patient stated allergy: penicillin. Effect not known."
    );

    let mut c = entry(3, "drug", "metformin", "adverse-effect", "other", 8);
    c.reaction_text = Some("loose bowel motions".into());
    c.onset_precision = "year".into();
    c.onset_value = Some("2019".into());
    assert_eq!(
        entry_line_text(&c),
        "Previous adverse effect (2019): metformin → loose bowel motions"
    );
}

#[test]
fn onset_formats_are_unambiguous() {
    let mut a = entry(1, "drug", "X", "allergy", "rash", 8);
    a.onset_precision = "day".into();
    a.onset_value = Some("2019-03-04".into());
    assert_eq!(entry_line_text(&a), "Allergy: X → rash · 4 Mar 2019");
    a.onset_precision = "month".into();
    a.onset_value = Some("2019-03".into());
    assert_eq!(entry_line_text(&a), "Allergy: X → rash · Mar 2019");
}

#[test]
fn an_allergy_overrides_no_known_allergies_in_either_order() {
    let a = entry(2, "drug", "penicillin", "allergy", "rash", 9);
    // The none row is older, then newer than the entry: same view.
    let older = view(&pa(vec![a.clone()], vec![none(1, "all", 8)]));
    let newer = view(&pa(vec![a], vec![none(1, "all", 10)]));
    // Ruling R16 (design §3.4 "in its list"): the drug allergy overrides the statement for drugs
    // only; for other allergies it still stands.
    assert_eq!(
        older.strip,
        "Allergy: penicillin → rash · no known other allergies (asked 8 Nov 2026)"
    );
    assert_eq!(
        newer.strip,
        "Allergy: penicillin → rash · no known other allergies (asked 10 Nov 2026)"
    );
    assert!(older.previously_none.is_empty());
    assert_eq!(
        older.standing_none,
        vec!["No known other allergies — asked 8 Nov 2026"]
    );
}

#[test]
fn no_known_allergies_plus_a_latex_entry_still_stands_for_drugs() {
    let p = pa(
        vec![entry(2, "other", "latex", "allergy", "urticaria", 9)],
        vec![none(1, "all", 8)],
    );
    let v = view(&p);
    assert_eq!(
        v.strip,
        "No known drug allergies (asked 8 Nov 2026) · Other: latex → urticaria"
    );
    assert!(v.previously_none.is_empty());
    assert_eq!(
        v.standing_none,
        vec!["No known drug allergies — asked 8 Nov 2026"]
    );
}

#[test]
fn an_all_none_overridden_in_both_lists_is_only_history() {
    let p = pa(
        vec![
            entry(2, "drug", "penicillin", "allergy", "rash", 9),
            entry(3, "other", "latex", "allergy", "urticaria", 9),
        ],
        vec![none(1, "all", 8)],
    );
    let v = view(&p);
    assert_eq!(
        v.strip,
        "Allergy: penicillin → rash · Other: latex → urticaria"
    );
    assert!(v.standing_none.is_empty());
    assert_eq!(
        v.previously_none,
        vec!["Previously recorded: no known allergies (asked 8 Nov 2026)"]
    );
}

#[test]
fn the_standing_statement_and_history_name_who_signed() {
    let mut n = none(1, "all", 8);
    n.signer = Some("Dr Bea".into());
    let v = view(&pa(vec![], vec![n.clone()]));
    assert_eq!(
        v.standing_none,
        vec!["No known allergies — asked 8 Nov 2026 by Dr Bea"]
    );
    let mut d = none(2, "drug", 9);
    d.signer = Some("Dr Cy".into());
    let a = entry(3, "drug", "penicillin", "allergy", "rash", 10);
    // An overridden drug-scope statement is history, its signer named.
    let v = view(&pa(vec![a], vec![d]));
    assert!(v.standing_none.is_empty());
    assert_eq!(
        v.previously_none,
        vec!["Previously recorded: no known drug allergies (asked 9 Nov 2026 by Dr Cy)"]
    );
    // Two statements standing, one per list, newest drug first.
    let v = view(&pa(vec![], vec![n, none(4, "drug", 9)]));
    assert_eq!(
        v.standing_none,
        vec![
            "No known drug allergies — asked 9 Nov 2026",
            "No known other allergies — asked 8 Nov 2026 by Dr Bea"
        ]
    );
}

/// An allergy event this terminal cannot read (no custody, or shredded) has no projected row:
/// the strip leads with it and never says "not yet asked" over it.
#[test]
fn an_unreadable_record_leads_the_strip_and_silences_not_yet_asked() {
    let mut p = pa(vec![], vec![]);
    p.unreadable = 1;
    let v = view(&p);
    assert_eq!(v.strip, "1 allergy record cannot be read on this terminal");
    assert_eq!(v.unreadable, 1);
    let mut p = pa(vec![], vec![none(1, "drug", 8)]);
    p.unreadable = 1;
    assert_eq!(
        strip(&p),
        "1 allergy record cannot be read on this terminal · NKDA (asked 8 Nov 2026)"
    );
    let mut p = pa(
        vec![
            entry(2, "other", "latex", "allergy", "rash", 8),
            entry(3, "drug", "metformin", "adverse-effect", "rash", 8),
        ],
        vec![],
    );
    p.unreadable = 3;
    assert_eq!(
        strip(&p),
        "3 allergy records cannot be read on this terminal · Other: latex → rash · +1 adverse effect"
    );
}

#[test]
fn an_allergy_on_a_linked_chart_overrides_a_none_on_this_one() {
    let mut a = entry(2, "drug", "penicillin", "allergy", "rash", 9);
    a.chart = id(2000);
    let v = view(&pa(vec![a], vec![none(1, "drug", 8)]));
    assert_eq!(v.drug, ListStatus::Entries);
    assert_eq!(v.previously_none.len(), 1);
}

#[test]
fn six_drug_allergies_all_reach_the_strip() {
    let es = (0..6)
        .map(|i| entry(10 + i, "drug", &format!("drug{i}"), "allergy", "rash", 8))
        .collect();
    let s = strip(&pa(es, vec![]));
    for i in 0..6 {
        assert!(s.contains(&format!("drug{i}")), "{s}");
    }
}

#[test]
fn adverse_effects_are_counted_not_listed_on_the_strip() {
    let p = pa(
        vec![
            entry(1, "drug", "metformin", "adverse-effect", "rash", 8),
            entry(2, "drug", "statin", "adverse-effect", "rash", 8),
        ],
        vec![],
    );
    let v = view(&p);
    assert_eq!(v.strip, "Allergies not yet asked · +2 adverse effects");
    assert_eq!(v.adverse_effects.len(), 2);
    assert!(!v.adverse_effects[0].prominent);
}

#[test]
fn an_adverse_effect_does_not_override_nkda() {
    let p = pa(
        vec![entry(1, "drug", "metformin", "adverse-effect", "rash", 9)],
        vec![none(2, "drug", 8)],
    );
    assert_eq!(
        strip(&p),
        // Follows the other-part rule (drug NoneKnown + other NotYetAsked), not the brief's literal.
        "NKDA (asked 8 Nov 2026) · other allergies: not yet asked · +1 adverse effect"
    );
}

#[test]
fn order_puts_anaphylaxis_first_and_gi_upset_last() {
    let cats = [
        "gi-upset",
        "rash",
        "weird",
        "anaphylaxis",
        "other",
        "urticaria",
        "angioedema",
        "not-known",
    ];
    let es = cats
        .iter()
        .enumerate()
        .map(|(i, c)| entry(i as u128 + 1, "drug", c, "allergy", c, 8))
        .collect();
    let v = view(&pa(es, vec![]));
    assert_eq!(v.drug_allergies.len(), 8);
    let terms: Vec<String> = v.drug_allergies.iter().map(|l| l.text.clone()).collect();
    let first_words: Vec<_> = terms
        .iter()
        .map(|t| t.split(' ').nth(1).unwrap().to_string())
        .collect();
    assert_eq!(
        first_words,
        [
            "anaphylaxis",
            "not-known",
            "angioedema",
            "urticaria",
            "rash",
            "other",
            "gi-upset",
            "weird"
        ]
    );
}

#[test]
fn ties_are_newest_first() {
    let es = vec![
        entry(1, "drug", "old", "allergy", "rash", 8),
        entry(2, "drug", "new", "allergy", "rash", 9),
    ];
    let v = view(&pa(es, vec![]));
    assert!(v.drug_allergies[0].text.contains("new"));
}

#[test]
fn a_doubted_charts_entry_is_shown_and_marked() {
    let mut a = entry(1, "drug", "penicillin", "allergy", "rash", 8);
    a.chart = id(2000);
    let mut p = pa(vec![a], vec![]);
    p.doubted_charts = vec![id(2000)];
    let v = view(&p);
    assert!(v.drug_allergies[0].doubted_chart);
    assert!(v.drug_allergies[0]
        .text
        .ends_with(" (on a chart whose link to this one is in doubt)"));
    assert!(v.strip.contains("link to this one is in doubt"));
}

#[test]
fn unknown_kind_is_listed_not_prominent_and_off_the_strip() {
    let p = pa(
        vec![entry(1, "drug", "foo", "intolerance", "rash", 8)],
        vec![],
    );
    let v = view(&p);
    assert_eq!(v.drug_allergies.len(), 1);
    assert!(!v.drug_allergies[0].prominent);
    assert_eq!(v.drug_allergies[0].text, "intolerance: foo → rash");
    assert_eq!(v.drug, ListStatus::NotYetAsked);
    assert_eq!(v.strip, "Allergies not yet asked · +1 unrecognised entry");
}

#[test]
fn dates_use_the_node_local_day() {
    // 20:00 UTC on the 7th is 06:00 on the 8th at +10:00.
    let t = Utc.with_ymd_and_hms(2026, 11, 7, 20, 0, 0).unwrap();
    let mut n = none(1, "all", 7);
    n.recorded_at = t;
    n.hlc = hlc(t);
    let v = view(&pa(vec![], vec![n]));
    assert_eq!(v.strip, "No known allergies (asked 8 Nov 2026)");
    let mut e = entry(2, "drug", "x", "allergy", "rash", 7);
    e.recorded_at = t;
    assert_eq!(
        view(&pa(vec![e], vec![])).drug_allergies[0].recorded,
        chrono::NaiveDate::from_ymd_opt(2026, 11, 8).unwrap()
    );
}

#[test]
fn a_peers_newer_allergen_kind_counts_as_other() {
    let p = pa(
        vec![entry(1, "food", "peanut", "allergy", "anaphylaxis", 9)],
        vec![none(2, "all", 8)],
    );
    let v = view(&p);
    assert_eq!(v.other, ListStatus::Entries);
    assert!(v.other_allergies[0].prominent);
    assert_eq!(
        v.strip,
        "No known drug allergies (asked 8 Nov 2026) · Other: peanut → anaphylaxis (food)"
    );
    assert!(v.previously_none.is_empty());
}

#[test]
fn unrecognised_kinds_are_counted_on_the_strip() {
    let one = pa(
        vec![entry(1, "drug", "foo", "intolerance", "rash", 8)],
        vec![],
    );
    assert_eq!(
        strip(&one),
        "Allergies not yet asked · +1 unrecognised entry"
    );
    let two = pa(
        vec![
            entry(1, "drug", "foo", "intolerance", "rash", 8),
            entry(2, "other", "bar", "sensitivity", "rash", 8),
            entry(3, "drug", "m", "adverse-effect", "rash", 8),
        ],
        vec![none(4, "all", 7)],
    );
    // An unrecognised kind never overrides a none.
    assert_eq!(
        strip(&two),
        "No known allergies (asked 7 Nov 2026) · +1 adverse effect · +2 unrecognised entries"
    );
}

#[test]
fn entries_join_with_semicolons_and_sections_with_dots() {
    let mut a = entry(1, "drug", "penicillin", "allergy", "anaphylaxis", 8);
    a.onset_precision = "year".into();
    a.onset_value = Some("2014".into());
    let p = pa(
        vec![
            a,
            entry(2, "drug", "cefalexin", "allergy", "rash", 8),
            entry(3, "other", "latex", "allergy", "urticaria", 8),
            entry(4, "drug", "metformin", "adverse-effect", "rash", 8),
        ],
        vec![],
    );
    assert_eq!(
        strip(&p),
        "Allergy: penicillin → anaphylaxis · 2014; Allergy: cefalexin → rash · Other: latex → urticaria · +1 adverse effect"
    );
}

#[test]
fn other_part_drops_only_the_plain_allergy_prefix() {
    let mut e = entry(1, "other", "latex", "allergy", "urticaria", 8);
    e.source = "patient-stated".into();
    let v = view(&pa(vec![e], vec![]));
    assert!(
        v.strip
            .contains("Other: Patient stated allergy: latex → urticaria"),
        "{}",
        v.strip
    );
    assert!(v.other_allergies[0]
        .text
        .starts_with("Patient stated allergy: "));
    let v = view(&pa(
        vec![entry(2, "other", "latex", "allergy", "rash", 8)],
        vec![],
    ));
    assert_eq!(v.other_allergies[0].text, "Allergy: latex → rash");
}

#[test]
fn a_newer_drug_none_over_an_older_all_none_still_says_others_were_asked() {
    let v = view(&pa(vec![], vec![none(1, "all", 5), none(2, "drug", 8)]));
    assert_eq!(
        v.strip,
        "NKDA (asked 8 Nov 2026) · no known other allergies (asked 5 Nov 2026)"
    );
    assert!(v.previously_none.is_empty());
}

#[test]
fn drug_not_asked_while_other_has_entries() {
    let v = view(&pa(
        vec![entry(1, "other", "latex", "allergy", "rash", 8)],
        vec![],
    ));
    assert_eq!(
        v.strip,
        "Drug allergies not yet asked · Other: latex → rash"
    );
}
