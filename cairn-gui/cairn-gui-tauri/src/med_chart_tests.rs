//! Pure tests for `med_chart.rs`: what the line signs (its term, its parts, its input and its
//! override), said before the key, and the refusal order against `AppState::mock` (no Tauri
//! runtime, as `allergies_tests.rs` does). The DB-gated walks are `med_chart_db_tests.rs`;
//! the list's combination lines are `med_chart_rows_tests.rs`.
use super::*;
use crate::allergy_check::BAD_CODE;

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

/// The shared helper, as Sign calls it on the line it was sent.
fn before_key(l: &ChartArg) -> Result<(String, Vec<PartArg>), ErrorView> {
    parts_before_the_key(&l.parts, &l.typed_drug)
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
    let (term, parts) = before_key(&line(
        vec![part("amox", coded(&upper, "amoxicillin"))],
        "amox",
        "started-here",
    ))
    .unwrap();
    assert_eq!(term, "amoxicillin");
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].term, "amoxicillin");
    assert_eq!(parts[0].coding.as_ref().unwrap().code, amox.to_string());
    let (term, _) = before_key(&line(
        vec![part("Amoxil", coded(amox, "amoxicillin"))],
        "Amoxil",
        "started-here",
    ))
    .unwrap();
    assert_eq!(term, "Amoxil");
}

/// Ruling R22: a combination signs its parts' signed terms joined " + " — "amox + clav" with
/// both picked is "amoxicillin + clavulanic acid", the generic names the record displays — and
/// an unpicked part keeps its typed words. The check reads every part under the signed term AND
/// the drug as typed (R12, R24), so a by-name allergy on either still marks.
#[test]
fn a_combination_signs_its_parts_names_and_checks_both_terms() {
    let amox = Uuid::from_u128(0xA40C);
    let clav = Uuid::from_u128(0xC1A4);
    let both = vec![
        part("amox", coded(amox, "amoxicillin")),
        part("clav", coded(clav, "clavulanic acid")),
    ];
    let (term, parts) = before_key(&line(both, " amox + clav ", "started-here")).unwrap();
    assert_eq!(term, "amoxicillin + clavulanic acid");
    let checked = the_line(" amox + clav ", &term, &parts);
    let terms: Vec<&str> = checked.iter().map(|l| l.term).collect();
    assert_eq!(
        terms,
        [
            "amoxicillin + clavulanic acid",
            "amoxicillin + clavulanic acid",
            "amox + clav",
            "amox + clav"
        ]
    );
    assert!(checked.iter().all(|l| l.group_id == Uuid::nil()));

    let (term, parts) = before_key(&line(
        vec![part("amox", coded(amox, "amoxicillin")), part("clav", None)],
        " amox + clav ",
        "started-here",
    ))
    .unwrap();
    assert_eq!(term, "amoxicillin + clav");
    assert_eq!(parts[0].term, "amoxicillin");
    assert_eq!(parts[1].term, "clav");
    assert!(parts[1].coding.is_none());
    // One part: the check reads the signed term, as before (R16).
    let (term, parts) = before_key(&line(
        vec![part("amox", coded(amox, "amoxicillin"))],
        "amox",
        "started-here",
    ))
    .unwrap();
    assert_eq!(the_line("amox", &term, &parts)[0].term, "amoxicillin");
}

#[test]
fn a_bad_pick_or_a_blank_name_is_refused_before_the_key() {
    let bad = |p: Vec<PartArg>, typed: &str| before_key(&line(p, typed, "started-here"));
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
    // A pick with no words to show would pass here and fail db/041 after the key.
    assert_eq!(
        bad(vec![part("amox", coded(Uuid::from_u128(2), "  "))], "amox")
            .unwrap_err()
            .text,
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
            "Charting despite a warning needs a reason"
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

/// The window drew a mark (so it sent a reason) but Sign's own fresh check could not run: the
/// reason must never be dropped silently — refused before the key, so the window checks again
/// and the clinician sees "not checked" before a line signs unchecked (review finding, PR #1066).
#[test]
fn a_reason_sent_against_a_check_that_could_not_run_is_refused() {
    let unchecked = LineCheck {
        unverified: true,
        ..LineCheck::default()
    };
    assert_eq!(
        override_before_the_key(ChartedAs::StartedHere, &unchecked, Some("no alternative"))
            .unwrap_err()
            .text,
        CHECK_DID_NOT_RUN
    );
    // A blank reason is no reason: the window drew no mark, so "not checked" signs as before.
    assert_eq!(
        override_before_the_key(ChartedAs::StartedHere, &unchecked, Some("  ")).unwrap(),
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

// The medication list's lines, with combinations, and the check at Sign: its own file (500 lines).
#[path = "med_chart_rows_tests.rs"]
mod rows;
