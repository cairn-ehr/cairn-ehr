//! Pure tests for `allergy_check.rs`: the medication rows as the check reads them, a picked
//! drug's code re-spelled or refused, the search in fixture mode, and the pane carrying the
//! check (or its failure line) onto the rows. These run in CI; the DB walks beside them
//! (`allergy_check_db_tests.rs`) do not (#905).
use super::*;
use crate::chart_set::chart_pane;
use crate::duplicates::view::DuplicateSection;
use crate::link::record_links::RecordLinks;
use cairn_medication_view::fixtures::sample_chart;
use cairn_medication_view::{MedicationRow, MedicationStatus};

/// The fixture chart's first row, renamed and recoded; each test changes one thing.
fn row(group: u128, term: &str, coding: Option<(&str, &str, &str)>) -> MedicationRow {
    let mut r = sample_chart().rows[0].clone();
    r.group_id = Uuid::from_u128(group);
    r.term = term.into();
    r.status = MedicationStatus::Active;
    r.holds = vec![];
    r.coding_system = coding.map(|c| c.0.into());
    r.coding_code = coding.map(|c| c.1.into());
    r.coding_display = coding.map(|c| c.2.into());
    r
}

#[test]
fn med_lines_come_from_the_rows() {
    let moiety = Uuid::from_u128(0xA40C);
    let code = moiety.to_string();
    let mut ceased = row(3, "aspirin", None);
    ceased.status = MedicationStatus::Ceased;
    // Its threads carry two codings and the row shows one: neither can stand for the line.
    let mut conflicted = row(5, "Coversyl", Some((MOIETY_SYSTEM, &code, "perindopril")));
    conflicted.coding_conflict = true;
    let rows = [
        row(1, "Amoxil", Some((MOIETY_SYSTEM, &code, "amoxicillin"))),
        // A coding under another system, or a code that is not a uuid, names no moiety.
        row(
            2,
            "Panadol",
            Some(("drugref-product", &code, "paracetamol")),
        ),
        ceased,
        row(
            4,
            "Augmentin",
            Some((MOIETY_SYSTEM, "not-a-uuid", "amoxicillin")),
        ),
        conflicted,
    ];
    let lines = med_lines(&rows);
    assert_eq!(lines.len(), 5);
    let l = &lines[0];
    assert_eq!(l.group_id, Uuid::from_u128(1));
    assert_eq!(l.name, "amoxicillin", "the name the clinician reads");
    assert_eq!(l.term, "Amoxil");
    assert_eq!(l.coding_display, Some("amoxicillin"));
    assert_eq!(l.moiety, Some(moiety));
    assert!(l.current);
    assert_eq!(lines[1].moiety, None);
    assert_eq!(lines[1].coding_display, Some("paracetamol"));
    assert!(!lines[2].current, "a ceased line is never checked");
    assert_eq!(lines[2].coding_display, None);
    assert_eq!(lines[3].moiety, None);
    assert_eq!(lines[4].moiety, None, "a conflicted group names no moiety");
    assert!(lines[4].current);
}

/// Review finding 1: two threads wrongly reconciled into one group, one coded amoxicillin, one
/// perindopril, showing perindopril. A coded amoxicillin allergy must never read that line clean:
/// with no drug list it is "not checked" (were the shown code used, drugref's `no-match` for
/// perindopril would have left it with no tag at all).
#[test]
fn a_conflicted_group_is_never_checked_clean() {
    use cairn_node::allergy::read::{AllergyRow, Coding, KeyState};
    let amox = Uuid::from_u128(0xA40C);
    let at = chrono::DateTime::from_timestamp(1_762_600_000, 0).unwrap();
    let allergy = AllergyRow {
        event_id: Uuid::from_u128(9),
        chart: Uuid::from_u128(1),
        allergen_kind: "drug".into(),
        term: "amoxicillin".into(),
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
        coding: Some(Coding {
            system: MOIETY_SYSTEM.into(),
            code: amox.to_string(),
            display: "amoxicillin".into(),
        }),
    };
    let a = PatientAllergies {
        entries: vec![allergy],
        none: vec![],
        challenges: vec![],
        retractions: vec![],
        doubted_charts: vec![],
        unreadable: 0,
    };
    let perindopril = Uuid::from_u128(0x9E81).to_string();
    let mut r = row(
        5,
        "Coversyl",
        Some((MOIETY_SYSTEM, &perindopril, "perindopril")),
    );
    r.coding_conflict = true;
    let lines = med_lines(std::slice::from_ref(&r));
    let resolved = cairn_node::allergy::thread::resolve(&a);
    let (_, meds) = cairn_node::allergy::check::codes_to_ask(&resolved, &lines);
    assert!(
        meds.is_empty(),
        "the drug list is not asked about either coding"
    );
    let checks = check_medications(&resolved, &lines, &DrugFacts::Unavailable, false);
    assert_eq!(
        checks.get(&r.group_id),
        Some(&LineCheck {
            unverified: true,
            ..LineCheck::default()
        })
    );
}

/// Ruling T5-c: the allergy rows are used only when they were read over the medication list's
/// own chart set; a set that moved between the two reads, or a failed read, is the worded line.
#[test]
fn allergies_over_another_chart_set_are_not_used() {
    let set = |ids: &[u128]| ChartSet::new(ids.iter().map(|n| Uuid::from_u128(*n))).unwrap();
    let rows = || PatientAllergies {
        entries: vec![],
        none: vec![],
        challenges: vec![],
        retractions: vec![],
        doubted_charts: vec![],
        unreadable: 0,
    };
    let read = |ids: &[u128]| Ok((set(ids), rows(), HashMap::new()));
    assert!(allergies_for_the_list(read(&[1, 2]), &set(&[2, 1])).is_ok());
    for moved in [&[1][..], &[1, 2, 3]] {
        assert_eq!(
            allergies_for_the_list(read(moved), &set(&[1, 2])).err(),
            Some(ALLERGIES_NOT_READ.to_string()),
            "{moved:?}"
        );
    }
    let failed = Err(ErrorView {
        text: "connection closed".into(),
        retry: crate::funnel::view::Retry::Now,
    });
    assert_eq!(
        allergies_for_the_list(failed, &set(&[1])).err(),
        Some(ALLERGIES_NOT_READ.to_string())
    );
}

/// Fixture mode: its allergies are read over the fixture chart, so the check runs and says
/// nothing (never the failure line).
#[tokio::test]
async fn fixture_mode_checks_quietly() {
    let chart: Uuid = cairn_gui_data::mock::fixtures::FIXTURE_UUID
        .parse()
        .unwrap();
    let state = AppState::mock(Some(chart));
    let pane = crate::commands::med_list_impl(&state, &chart.to_string(), None)
        .await
        .unwrap();
    assert_eq!(pane.allergy_check_failed, None);
}

/// Ruling T5-d (D1): a pick on an entry that is not a drug is refused before the key; on a drug
/// the code is checked as before.
#[test]
fn a_pick_on_a_non_drug_entry_is_refused() {
    let pick = CodingArg {
        system: MOIETY_SYSTEM.into(),
        code: Uuid::from_u128(0xA40C).to_string(),
        display: "amoxicillin".into(),
    };
    let err = coding_before_the_key(&pick, AllergenKind::Other).unwrap_err();
    assert_eq!(err.text, NOT_A_DRUG);
    assert_eq!(err.retry, crate::funnel::view::Retry::Never);
    assert_eq!(
        coding_before_the_key(&pick, AllergenKind::Drug),
        Ok(pick.code.clone())
    );
    let bad = CodingArg {
        code: "amoxicillin".into(),
        ..pick
    };
    assert_eq!(
        coding_before_the_key(&bad, AllergenKind::Drug)
            .unwrap_err()
            .text,
        BAD_CODE
    );
}

/// D9: a withheld line is still current, so it is checked.
#[test]
fn a_withheld_line_is_current() {
    let mut r = row(1, "warfarin", None);
    r.holds = sample_chart()
        .rows
        .iter()
        .flat_map(|r| r.holds.clone())
        .filter(|h| !h.pending)
        .take(1)
        .collect();
    assert!(r.is_withheld(), "the fixture chart carries a hold");
    assert!(med_lines(std::slice::from_ref(&r))[0].current);
}

#[test]
fn a_picked_code_is_respelled_canonically_or_refused() {
    let id = Uuid::from_u128(0xC1A55);
    let arg = |code: String| CodingArg {
        system: GROUP_SYSTEM.into(),
        code,
        display: "Penicillins".into(),
    };
    let upper = id.to_string().to_uppercase();
    assert_eq!(canonical_code(&arg(upper)), Ok(id.to_string()));
    assert_eq!(
        canonical_code(&arg(format!(" {} ", id.simple()))),
        Ok(id.to_string())
    );
    for bad in ["", "penicillin", "1234"] {
        let err = canonical_code(&arg(bad.into())).unwrap_err();
        assert_eq!(err.text, BAD_CODE, "{bad}");
        assert_eq!(err.retry, crate::funnel::view::Retry::Never);
    }
    assert_eq!(
        BAD_CODE,
        "That drug's code could not be read; type the name instead"
    );
}

/// Fixture mode has no drug list: the pick says so, never fails.
#[tokio::test]
async fn fixture_mode_has_no_drug_list() {
    let state = AppState::mock(None);
    let found = drug_search_impl(&state, "amox").await;
    assert!(!found.available);
    assert!(found.options.is_empty());
}

fn pane_with(check: Result<HashMap<Uuid, LineCheck>, String>) -> crate::chart_set::ChartPane {
    chart_pane(
        &sample_chart(),
        Uuid::from_u128(1),
        Ok(vec![]),
        Ok(RecordLinks::default()),
        DuplicateSection::checked_none_open(),
        None,
        check,
    )
}

/// The check lands on its own row, by group id; every other row says nothing.
#[test]
fn the_pane_carries_each_lines_check() {
    let chart = sample_chart();
    let (first, second) = (chart.rows[0].group_id, chart.rows[1].group_id);
    let check = HashMap::from([
        (
            first,
            LineCheck {
                marks: vec!["Allergy recorded: penicillin → anaphylaxis".into()],
                notes: vec!["Previous adverse effect: codeine → GI upset".into()],
                unverified: false,
            },
        ),
        (
            second,
            LineCheck {
                unverified: true,
                ..LineCheck::default()
            },
        ),
    ]);
    let pane = pane_with(Ok(check));
    assert_eq!(pane.allergy_check_failed, None);
    let rows = &pane.list.rows;
    assert_eq!(
        rows[0].allergy_marks,
        ["Allergy recorded: penicillin → anaphylaxis"]
    );
    assert_eq!(
        rows[0].allergy_notes,
        ["Previous adverse effect: codeine → GI upset"]
    );
    assert!(!rows[0].allergy_unverified);
    assert!(rows[1].allergy_marks.is_empty());
    assert!(rows[1].allergy_unverified);
    for r in &rows[2..] {
        assert!(r.allergy_marks.is_empty() && r.allergy_notes.is_empty());
        assert!(!r.allergy_unverified);
    }
}

/// Review Focus 4, the pure half: an unread allergy list keeps the medication list and says so
/// above it, in the exact words.
#[test]
fn a_failed_allergy_read_keeps_the_list_and_says_so() {
    let pane = pane_with(Err(ALLERGIES_NOT_READ.into()));
    assert_eq!(pane.list.rows.len(), sample_chart().rows.len());
    assert_eq!(
        pane.allergy_check_failed.as_deref(),
        Some("Allergies could not be read: medications not checked against them")
    );
    let json = serde_json::to_value(&pane).unwrap();
    assert!(json["allergy_check_failed"].is_string());
    let row = &json["list"]["rows"][0];
    assert_eq!(row["allergy_marks"], serde_json::json!([]));
    assert_eq!(row["allergy_notes"], serde_json::json!([]));
    assert_eq!(row["allergy_unverified"], false);
}

/// #1059, the window's half: `check_list` hands the unreadable count to the check even on the
/// path that asks the drug list nothing (no connection, no codes). A patient whose only allergy
/// record this terminal cannot read must not see a clean medication list.
#[tokio::test]
async fn an_unreadable_allergy_record_leaves_current_lines_not_checked() {
    let a = PatientAllergies {
        entries: vec![],
        none: vec![],
        challenges: vec![],
        retractions: vec![],
        doubted_charts: vec![],
        unreadable: 1,
    };
    let mut list = sample_chart();
    list.rows = vec![row(1, "Amoxil", None)];
    let checks = check_list(None::<&tokio_postgres::Client>, &a, &list).await;
    assert_eq!(
        checks.get(&Uuid::from_u128(1)),
        Some(&LineCheck {
            unverified: true,
            ..LineCheck::default()
        })
    );
}
