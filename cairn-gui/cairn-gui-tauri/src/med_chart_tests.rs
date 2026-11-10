//! Pure tests for `med_chart.rs`: what the line signs (its term, its parts, its input and its
//! override), said before the key, and the refusal order against `AppState::mock` (no Tauri
//! runtime, as `allergies_tests.rs` does). The DB-gated walks are `med_chart_db_tests.rs`.
use super::*;
use crate::allergy_check::{charted_lines, BAD_CODE};
use cairn_medication_view::fixtures::sample_chart;
use cairn_medication_view::{MedicationRow, MedicationStatus, MemberVouch};
use cairn_node::allergy::check::{check_medications, MedLine};
use cairn_node::allergy::drug_facts::DrugFacts;
use cairn_node::allergy::read::{AllergyRow, KeyState, PatientAllergies};
use cairn_node::medication::ChartingRow;
use std::collections::HashMap;

fn on_screen() -> (AppState, String) {
    let chart: Uuid = cairn_gui_data::mock::fixtures::FIXTURE_UUID
        .parse()
        .unwrap();
    (AppState::mock(Some(chart)), chart.to_string())
}

fn coded(code: impl ToString, display: &str) -> Option<CodingArg> {
    Some(CodingArg {
        system: MOIETY_SYSTEM.into(),
        code: code.to_string(),
        display: display.into(),
    })
}

fn part(term: &str, coding: Option<CodingArg>) -> PartArg {
    PartArg {
        term: term.into(),
        coding,
    }
}

fn line(parts: Vec<PartArg>, typed_drug: &str, charted_as: &str) -> ChartArg {
    ChartArg {
        parts,
        typed_drug: typed_drug.into(),
        directions: "500mg tds".into(),
        charted_as: charted_as.into(),
        reason: None,
    }
}

/// A check with one allergy mark against the record `head`.
fn hit(head: u128) -> LineCheck {
    LineCheck {
        marks: vec!["Allergy recorded: amoxicillin → anaphylaxis".into()],
        against: vec![Uuid::from_u128(head)],
        ..LineCheck::default()
    }
}

#[tokio::test]
async fn med_line_read_reads_the_directions() {
    let d = med_line_read("500mg tds".into()).await;
    assert_eq!(d.dose_amount.as_deref(), Some("500"));
    assert_eq!(d.frequency.as_deref(), Some("three times daily"));
}

#[tokio::test]
async fn no_drug_list_in_fixture_mode_is_not_an_error() {
    let (state, _) = on_screen();
    let found = med_drug_search_impl(&state, "amoxicillin").await;
    assert!(!found.available && found.options.is_empty());
}

/// One part: the pick finishes what was typed part-way ("amox" → amoxicillin), and a code is
/// re-spelled canonically. A brand typed in full stays the clinician's words.
#[test]
fn one_part_signs_the_finished_term_and_its_canonical_code() {
    let amox = Uuid::from_u128(0xA40C);
    let upper = amox.to_string().to_uppercase();
    let (term, parts) = parts_before_the_key(&line(
        vec![part("amox", coded(&upper, "amoxicillin"))],
        "amox",
        "started-here",
    ))
    .unwrap();
    assert_eq!(term, "amoxicillin");
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].term, "amoxicillin");
    assert_eq!(parts[0].coding.as_ref().unwrap().code, amox.to_string());
    let (term, _) = parts_before_the_key(&line(
        vec![part("Amoxil", coded(amox, "amoxicillin"))],
        "Amoxil",
        "started-here",
    ))
    .unwrap();
    assert_eq!(term, "Amoxil");
}

/// Two parts: the line's term is the drug as typed, each part's term its own finished pick.
#[test]
fn a_combination_signs_the_typed_drug_and_each_part() {
    let amox = Uuid::from_u128(0xA40C);
    let (term, parts) = parts_before_the_key(&line(
        vec![part("amox", coded(amox, "amoxicillin")), part("clav", None)],
        " amox + clav ",
        "started-here",
    ))
    .unwrap();
    assert_eq!(term, "amox + clav");
    assert_eq!(parts[0].term, "amoxicillin");
    assert_eq!(parts[1].term, "clav");
    assert!(parts[1].coding.is_none());
}

#[test]
fn a_bad_pick_or_a_blank_name_is_refused_before_the_key() {
    let bad = |p: Vec<PartArg>, typed: &str| parts_before_the_key(&line(p, typed, "started-here"));
    assert_eq!(
        bad(
            vec![part("amox", coded("amoxicillin", "amoxicillin"))],
            "amox"
        )
        .unwrap_err()
        .text,
        BAD_CODE
    );
    let mut class = coded(Uuid::from_u128(1), "Penicillins (allergy group)");
    class.as_mut().unwrap().system = "drugref-class".into();
    assert_eq!(
        bad(vec![part("pen", class)], "pen").unwrap_err().text,
        NOT_A_LISTED_DRUG
    );
    assert_eq!(bad(vec![part("  ", None)], "  ").unwrap_err().text, NAME_IT);
    assert_eq!(bad(vec![], " ").unwrap_err().text, NAME_IT);
    assert_eq!(
        bad(vec![part("amox", None), part(" ", None)], "amox + ")
            .unwrap_err()
            .text,
        NAME_IT
    );
    // No parts sent: the drug as typed, uncoded.
    let (term, parts) = bad(vec![], "metformin").unwrap();
    assert_eq!((term.as_str(), parts.len()), ("metformin", 1));
}

#[test]
fn a_start_line_with_a_mark_needs_a_trimmed_reason() {
    let start = ChartedAs::StartedHere;
    for blank in [None, Some(""), Some("   ")] {
        assert_eq!(
            override_before_the_key(start, &hit(9), blank)
                .unwrap_err()
                .text,
            "Charting despite a listed allergy needs a reason"
        );
    }
    assert_eq!(
        override_before_the_key(start, &hit(9), Some("  no alternative ")).unwrap(),
        Some("no alternative")
    );
    // No mark: nothing to override, a reason typed or not.
    assert_eq!(
        override_before_the_key(start, &LineCheck::default(), None).unwrap(),
        None
    );
    let unchecked = LineCheck {
        unverified: true,
        ..LineCheck::default()
    };
    assert_eq!(
        override_before_the_key(start, &unchecked, None).unwrap(),
        None
    );
}

#[test]
fn an_already_taking_line_never_needs_a_reason() {
    for reason in [None, Some("   "), Some("typed anyway")] {
        assert_eq!(
            override_before_the_key(ChartedAs::PreExisting, &hit(9), reason).unwrap(),
            None
        );
    }
}

#[test]
fn the_press_sets_the_source_and_the_start() {
    let d = read_directions("1 tab 500 mg po tds");
    let start = charted_input(
        "amoxicillin",
        None,
        &d,
        "1 tab 500 mg po tds",
        ChartedAs::StartedHere,
        "2026-11-11",
    );
    assert_eq!(start.info_source, "clinician-observed");
    assert_eq!(
        (start.started, start.started_precision),
        (Some("2026-11-11"), Some("day"))
    );
    assert_eq!(
        start.sig,
        Some("1 tab 500 mg po tds"),
        "the directions as typed"
    );
    assert_eq!(start.formulation, d.form.as_deref());
    assert_eq!(
        (start.dose_amount, start.dose_unit),
        (d.dose_amount.as_deref(), d.dose_unit.as_deref())
    );
    let already = charted_input(
        "amoxicillin",
        None,
        &d,
        "  ",
        ChartedAs::PreExisting,
        "2026-11-11",
    );
    assert_eq!(already.info_source, "patient-reported");
    assert_eq!((already.started, already.started_precision), (None, None));
    assert_eq!(already.sig, None, "no directions typed");
}

#[test]
fn only_the_two_presses_are_charted() {
    assert_eq!(press("started-here").unwrap(), ChartedAs::StartedHere);
    assert_eq!(press("pre-existing").unwrap(), ChartedAs::PreExisting);
    for word in ["unknown", "start", ""] {
        assert!(
            press(word).unwrap_err().text.contains("does not know"),
            "{word}"
        );
    }
}

#[tokio::test]
async fn med_chart_refuses_another_chart_then_fixture_mode_first() {
    let (state, chart) = on_screen();
    let other = Uuid::from_u128(424_242).to_string();
    let l = || line(vec![], "metformin", "no such press");
    let err = med_chart_impl(&state, &other, vec![other.clone()], l(), None)
        .await
        .unwrap_err();
    assert!(!err.text.contains("fixture mode"), "{}", err.text);
    // The chart on screen, but fixture mode: refused before the press is even read.
    let err = med_chart_impl(&state, &chart, vec![chart.clone()], l(), None)
        .await
        .unwrap_err();
    assert!(err.text.contains("fixture mode"), "{}", err.text);
}

/// A Sign that landed is an Ok whether or not the list could be read again.
#[test]
fn a_landed_sign_is_always_ok() {
    let w = charted(Err("connection closed".into()));
    assert!(w.pane.is_none());
    assert_eq!(
        w.outcome,
        "Signed — but the medication list could not be read again (connection closed). \
         Use “Read this chart again”."
    );
}

/// The view says a reason is needed exactly when there is an allergy mark.
#[test]
fn the_view_carries_the_check() {
    let v = view(hit(9));
    assert!(v.reason_needed && !v.unverified && v.marks.len() == 1);
    let v = view(LineCheck {
        notes: vec!["Previous adverse effect: x → nausea".into()],
        unverified: true,
        ..LineCheck::default()
    });
    assert!(!v.reason_needed && v.unverified && v.notes.len() == 1);
}

// --- the medication list's lines, with combinations (allergy_check::charted_lines) ---

fn row(group: u128, term: &str, members: &[u128]) -> MedicationRow {
    let mut r = sample_chart().rows[0].clone();
    r.group_id = Uuid::from_u128(group);
    r.term = term.into();
    r.status = MedicationStatus::Active;
    r.coding_system = None;
    r.coding_code = None;
    r.coding_display = None;
    let vouch = r.members[0].clone();
    r.members = members
        .iter()
        .map(|m| MemberVouch {
            medication_id: Uuid::from_u128(*m),
            ..vouch.clone()
        })
        .collect();
    r
}

fn combo(parts: &[(&str, Option<Uuid>)]) -> ChartingRow {
    ChartingRow {
        charted_as: Some("started-here".into()),
        route: None,
        strength: None,
        components: parts
            .iter()
            .map(|(t, c)| {
                let coding = c.map(|c| (MOIETY_SYSTEM.to_string(), c.to_string(), t.to_string()));
                (t.to_string(), coding)
            })
            .collect(),
        override_reason: None,
    }
}

#[test]
fn an_uncharted_list_reads_as_before() {
    let rows = [row(1, "Amoxil", &[11]), row(2, "Diabex", &[21])];
    let mut charting = HashMap::new();
    // A charted single drug (no components) is still one line.
    charting.insert(Uuid::from_u128(21), combo(&[]));
    let lines = charted_lines(&rows, &charting);
    let old = crate::allergy_check::med_lines(&rows);
    assert_eq!(lines.len(), 2);
    for (a, b) in lines.iter().zip(&old) {
        assert_eq!(
            (a.group_id, a.term, a.moiety, a.current),
            (b.group_id, b.term, b.moiety, b.current)
        );
    }
}

#[test]
fn a_combination_is_one_line_per_component_under_its_group() {
    let amox = Uuid::from_u128(0xA40C);
    let rows = [row(1, "Augmentin", &[11])];
    let charting = HashMap::from([(
        Uuid::from_u128(11),
        combo(&[("amoxicillin", Some(amox)), ("clavulanic acid", None)]),
    )]);
    let lines = charted_lines(&rows, &charting);
    assert_eq!(lines.len(), 2, "the components, not the row");
    assert!(lines
        .iter()
        .all(|l| l.group_id == Uuid::from_u128(1) && l.current));
    // R12: every part carries the LINE's term, so a by-name allergy on the brand still meets it.
    assert!(lines.iter().all(|l| l.term == "Augmentin"));
    assert_eq!(
        (lines[0].name, lines[0].moiety),
        ("amoxicillin", Some(amox))
    );
    assert_eq!(lines[0].coding_display, Some("amoxicillin"));
    assert_eq!((lines[1].name, lines[1].moiety), ("clavulanic acid", None));
}

/// A group reconciling a combination with a single drug keeps the row's own line too.
#[test]
fn a_mixed_group_keeps_its_row_line() {
    let rows = [row(1, "Augmentin", &[11, 12])];
    let charting = HashMap::from([(
        Uuid::from_u128(11),
        combo(&[("amoxicillin", None), ("clavulanic acid", None)]),
    )]);
    let lines: Vec<MedLine> = charted_lines(&rows, &charting);
    assert_eq!(lines.len(), 3);
}

fn uncoded_allergy(term: &str) -> PatientAllergies {
    let at = chrono::DateTime::from_timestamp(1_762_600_000, 0).unwrap();
    let a = AllergyRow {
        event_id: Uuid::from_u128(9),
        chart: Uuid::from_u128(1),
        allergen_kind: "drug".into(),
        term: term.into(),
        kind: "allergy".into(),
        category: "anaphylaxis".into(),
        reaction_text: None,
        onset_precision: "unknown".into(),
        onset_value: None,
        source: "patient-stated".into(),
        recorded_at: at,
        hlc: (at.timestamp_millis(), 0),
        signer: None,
        corrects: None,
        key: KeyState::Standing,
        coding: None,
    };
    PatientAllergies {
        entries: vec![a],
        none: vec![],
        challenges: vec![],
        retractions: vec![],
        doubted_charts: vec![],
        unreadable: 0,
    }
}

/// R12: an allergy recorded by name on the brand ("Augmentin", uncoded) marks a line typed
/// "Augmentin" and charted as two coded components — on the list and on the line being charted.
#[tokio::test]
async fn a_brand_allergy_marks_a_combination_typed_by_the_brand() {
    let (amox, clav) = (Uuid::from_u128(0xA40C), Uuid::from_u128(0xC1A7));
    let a = uncoded_allergy("Augmentin");
    let r = cairn_node::allergy::thread::resolve(&a);

    let rows = [row(1, "Augmentin", &[11])];
    let charting = HashMap::from([(
        Uuid::from_u128(11),
        combo(&[("amoxicillin", Some(amox)), ("clavulanic acid", Some(clav))]),
    )]);
    let checks = check_medications(
        &r,
        &charted_lines(&rows, &charting),
        &DrugFacts::Unavailable,
        false,
    );
    let on_list = &checks[&Uuid::from_u128(1)];
    assert_eq!(
        on_list.marks.len(),
        1,
        "a mark, not just \"not checked\": {on_list:?}"
    );
    assert!(!on_list.unverified);

    let (term, parts) = parts_before_the_key(&line(
        vec![
            part("amoxicillin", coded(amox, "amoxicillin")),
            part("clavulanic acid", coded(clav, "clavulanic acid")),
        ],
        "Augmentin",
        "started-here",
    ))
    .unwrap();
    let charting_now = check_lines(
        None::<&tokio_postgres::Client>,
        &a,
        &the_line(&term, &parts),
    )
    .await;
    let now = charting_now.get(&Uuid::nil()).cloned().unwrap_or_default();
    assert_eq!(now.against, [Uuid::from_u128(9)]);
}
