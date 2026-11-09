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
    ];
    let lines = med_lines(&rows);
    assert_eq!(lines.len(), 4);
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
