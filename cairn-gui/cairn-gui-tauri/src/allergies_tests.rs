//! Tests for `allergies.rs`: the onset parser and the refusals a write says before the key (pure),
//! the chart-on-screen rule (against `AppState::mock`, no Tauri runtime, as `commands_tests.rs`
//! does), and one DB-gated walk — record, then list — under the window's own login (on
//! `CAIRN_TEST_PG`, run locally: #905, CI never runs it).
use super::*;
use crate::funnel::view::{Recorded, Retry};
use crate::notes::review::db_tests::{credential, supervised_note};

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, 8).unwrap()
}

fn on_screen() -> (AppState, String) {
    let chart: Uuid = cairn_gui_data::mock::fixtures::FIXTURE_UUID
        .parse()
        .unwrap();
    (AppState::mock(Some(chart)), chart.to_string())
}

fn another_chart() -> String {
    Uuid::from_u128(424_242).to_string()
}

/// A complete entry, as the form sends it; each test spoils one field.
fn entry() -> EntryInput {
    EntryInput {
        allergen_kind: "drug".into(),
        term: "penicillin".into(),
        kind: Some("allergy".into()),
        category: Some("anaphylaxis".into()),
        reaction_text: None,
        onset: Some("2014".into()),
        source: "seen-documented".into(),
    }
}

#[test]
fn parse_onset_accepts_the_five_shapes() {
    let p = |s| parse_onset(s, today());
    assert_eq!(p(None), Ok(Onset::NotKnown));
    assert_eq!(p(Some("  ")), Ok(Onset::NotKnown));
    assert_eq!(p(Some("2019")), Ok(Onset::Year("2019".into())));
    assert_eq!(p(Some("03/2019")), Ok(Onset::Month("2019-03".into())));
    assert_eq!(p(Some("3/2019")), Ok(Onset::Month("2019-03".into())));
    assert_eq!(p(Some("14/03/2019")), Ok(Onset::Day("2019-03-14".into())));
    assert_eq!(p(Some("4/3/2019")), Ok(Onset::Day("2019-03-04".into())));
    assert_eq!(p(Some("2019-03")), Ok(Onset::Month("2019-03".into())));
    assert_eq!(p(Some(" 2019-03-14 ")), Ok(Onset::Day("2019-03-14".into())));
    // Today itself, at every precision, is not the future.
    assert_eq!(p(Some("2026")), Ok(Onset::Year("2026".into())));
    assert_eq!(p(Some("11/2026")), Ok(Onset::Month("2026-11".into())));
    assert_eq!(p(Some("08/11/2026")), Ok(Onset::Day("2026-11-08".into())));
}

#[test]
fn parse_onset_refuses_2019_13_and_31_02_2019_and_next_year() {
    for typed in [
        "2019-13",
        "31/02/2019",
        "2027",
        "12/2026",
        "09/11/2026",
        "spring 2019",
        "19",
        "2019/03",
        "14-03-2019",
        "00/2019",
    ] {
        let err = parse_onset(Some(typed), today()).unwrap_err();
        assert!(err.contains(&format!("\"{typed}\"")), "{typed}: {err}");
    }
    assert_eq!(
        parse_onset(Some("2019-13"), today()).unwrap_err(),
        "\"2019-13\" is not a date this window can read — type 2019, 03/2019 or 14/03/2019, \
         or leave it empty if it is not known."
    );
    assert_eq!(
        parse_onset(Some("2027"), today()).unwrap_err(),
        "\"2027\" is after today — when it happened cannot be later than today."
    );
}

/// No calendar has a year 0000 (chrono would accept it as 1 BC): refused at every precision, naming
/// what was typed, and under the onset field (the fragment allergies.js routes by).
#[test]
fn parse_onset_refuses_year_0000_at_every_precision() {
    for typed in ["0000", "03/0000", "14/03/0000", "0000-03", "0000-03-14"] {
        let err = parse_onset(Some(typed), today()).unwrap_err();
        assert!(err.starts_with(&format!("\"{typed}\" ")), "{typed}: {err}");
        assert!(err.contains("is not a date this window can read"), "{err}");
        assert!(err.contains("year 0000"), "{err}");
    }
}

/// The form's refusals, in the brief's order, each before the key.
#[test]
fn record_refuses_a_missing_kind_before_the_key() {
    let mut e = entry();
    e.kind = None;
    let err = entry_before_the_key(&e, today()).err().unwrap();
    assert_eq!(err.text, "Choose Allergy or Adverse effect.");
    assert_eq!(err.retry, Retry::Never);

    // An empty term is said first, a missing category next, then the onset.
    let mut e = entry();
    e.term = "  ".into();
    e.kind = None;
    e.category = None;
    assert_eq!(
        entry_before_the_key(&e, today()).err().unwrap().text,
        "Name the substance."
    );
    let mut e = entry();
    e.category = None;
    e.onset = Some("2019-13".into());
    assert_eq!(
        entry_before_the_key(&e, today()).err().unwrap().text,
        "Choose what happened, or Not known."
    );
    let mut e = entry();
    e.onset = Some("2019-13".into());
    assert!(entry_before_the_key(&e, today())
        .err()
        .unwrap()
        .text
        .starts_with("\"2019-13\" is not a date"));

    // A word the window never sends is refused, named — never read as a neighbour.
    for spoil in [
        |e: &mut EntryInput| e.kind = Some("Allergy".into()),
        |e: &mut EntryInput| e.category = Some("hives".into()),
        |e: &mut EntryInput| e.allergen_kind = "food".into(),
        |e: &mut EntryInput| e.source = "hearsay".into(),
    ] {
        let mut e = entry();
        spoil(&mut e);
        let err = entry_before_the_key(&e, today()).err().unwrap();
        assert!(
            err.text.starts_with("this window sent a word"),
            "{}",
            err.text
        );
    }

    // A complete entry reads through, trimmed, with blank free text as none.
    let mut e = entry();
    e.term = " penicillin ".into();
    e.reaction_text = Some("  ".into());
    let read = entry_before_the_key(&e, today()).unwrap();
    assert_eq!(read.term, "penicillin");
    assert_eq!(read.reaction_text, None);
    assert_eq!(read.onset, Onset::Year("2014".into()));
}

/// Checked FIRST, before fixture mode or the form: a write naming a chart that is not on screen
/// is refused for THAT reason, and so is a read.
#[tokio::test]
async fn record_refuses_a_chart_not_on_screen() {
    let (state, here) = on_screen();
    let other = another_chart();
    let mut bad = entry();
    bad.kind = None;
    let err = allergy_record_impl(&state, &other, vec![other.clone()], bad, None)
        .await
        .unwrap_err();
    assert!(err.text.contains("not the chart"), "{}", err.text);
    let err = allergy_none_impl(&state, &other, vec![other.clone()], "all", None)
        .await
        .unwrap_err();
    assert!(err.text.contains("not the chart"), "{}", err.text);
    assert!(allergy_list_impl(&state, &other)
        .await
        .unwrap_err()
        .text
        .contains("not the chart"));

    // The right chart, the wrong set: the list on screen is behind the record.
    let err = allergy_record_impl(&state, &here, vec![other.clone()], entry(), None)
        .await
        .unwrap_err();
    assert_eq!(err.retry, Retry::AfterReread(Recorded::No));
    // The right chart and set: fixture mode's own refusal, before the form's.
    let mut bad = entry();
    bad.kind = None;
    let err = allergy_record_impl(&state, &here, vec![here.clone()], bad, None)
        .await
        .unwrap_err();
    assert!(err.text.contains("fixture mode"), "{}", err.text);
    let err = allergy_none_impl(&state, &here, vec![here.clone()], "all", None)
        .await
        .unwrap_err();
    assert!(err.text.contains("fixture mode"), "{}", err.text);

    // Fixture mode has no allergies: an empty list, never a failure, over the chart on screen.
    let pane = allergy_list_impl(&state, &here).await.unwrap();
    assert_eq!(pane.strip, "Allergies not yet asked");
    assert_eq!(pane.charts, vec![here]);
}

/// The window's own login records an entry and a "none known" statement and lists them: a
/// missing kind is refused before the key (none is held), the entry lands under Dr Bea's key and
/// reaches the strip, the act is timed, and the later "none known" is shown only as history.
#[tokio::test]
async fn record_then_list_shows_the_entry_on_the_strip() {
    let Some((_guard, owner, state, chart, _)) = supervised_note().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let patient = chart.to_string();
    let charts = vec![patient.clone()];
    let pane = allergy_list_impl(&state, &patient).await.unwrap();
    assert_eq!(pane.strip, "Allergies not yet asked");
    assert_eq!(pane.charts, charts);

    let mut bad = entry();
    bad.kind = None;
    let err = allergy_record_impl(&state, &patient, charts.clone(), bad, None)
        .await
        .unwrap_err();
    assert_eq!(err.text, "Choose Allergy or Adverse effect.");
    // The timing table outlives the fixture's truncation and aggregates (one row per kind and
    // bucket, `n` samples): count samples from here.
    let timed = async |kind: &str| -> i64 {
        owner
            .query_one(
                "SELECT coalesce(sum(n), 0)::bigint FROM ui_gesture_timing \
                 WHERE gesture_kind = $1",
                &[&kind],
            )
            .await
            .unwrap()
            .get(0)
    };
    let (records, nones) = (timed("allergy-record").await, timed("allergy-none").await);

    let done = allergy_record_impl(&state, &patient, charts.clone(), entry(), credential("bea"))
        .await
        .unwrap();
    assert_eq!(done.outcome, "Recorded.");
    let pane = done.pane.expect("the list is read again");
    assert_eq!(pane.strip, "Allergy: penicillin → anaphylaxis · 2014");
    assert_eq!(pane.drug, "entries");
    let line = &pane.drug_allergies[0];
    assert!(line.prominent);
    assert_eq!(line.signer.as_deref(), Some("Dr Bea"));
    assert_eq!(timed("allergy-record").await, records + 1, "timed once");

    // Bea's key is held now: "none known" signs with no credential typed.
    let pane = allergy_none_impl(&state, &patient, charts.clone(), "all", None)
        .await
        .unwrap()
        .pane
        .expect("the list is read again");
    // Ruling R16: the drug allergy overrides "No known allergies" for drugs only; for other
    // allergies it stands, and its line names who signed it.
    assert!(
        pane.strip.starts_with(
            "Allergy: penicillin → anaphylaxis · 2014 · no known other allergies (asked "
        ),
        "{}",
        pane.strip
    );
    assert!(
        pane.previously_none.is_empty(),
        "{:?}",
        pane.previously_none
    );
    assert_eq!(pane.standing_none.len(), 1, "{:?}", pane.standing_none);
    assert!(
        pane.standing_none[0].starts_with("No known other allergies — asked ")
            && pane.standing_none[0].ends_with(" by Dr Bea"),
        "{:?}",
        pane.standing_none
    );
    assert_eq!(pane.unreadable, 0);
    assert_eq!(timed("allergy-none").await, nones + 1, "timed once");
    let err = allergy_none_impl(&state, &patient, charts, "everything", None)
        .await
        .err()
        .unwrap();
    assert!(
        err.text.starts_with("this window sent a word"),
        "{}",
        err.text
    );
}

/// A failure marked outcome-unknown by the node (#713, #814), as `write_failure_tests` makes one.
fn interrupted(event: Uuid) -> anyhow::Error {
    cairn_node::db_diagnosis::commit_outcome_unknown(event, anyhow::anyhow!("connection closed"))
        .context("signing the allergy")
}

/// #814: a sign whose commit got no ordinary answer is NOT known — said so, naming the event, and
/// re-read, never offered again live. Any other failure is the node's own sentence.
#[test]
fn an_interrupted_sign_is_not_known_and_never_retried() {
    let event = Uuid::from_u128(814);
    let view = sign_failure(&interrupted(event));
    assert!(
        view.text
            .starts_with("Whether this allergy was recorded is NOT known — "),
        "{}",
        view.text
    );
    assert!(
        view.text.contains(&format!("(event {event}: ")),
        "{}",
        view.text
    );
    assert_eq!(view.retry, Retry::AfterReread(Recorded::Unknown));
    let plain = anyhow::anyhow!("the floor refused it");
    assert_eq!(
        sign_failure(&plain),
        crate::write_failure::node_failure(&plain)
    );
}

/// Ruling R11: a sign that landed is an Ok whatever the re-read did. The failure branch is
/// tested here, at `written`, not by failing a real re-read: no seam fails ONLY the re-read on the
/// window's connection without mutating a role other suites share (privileges cannot be
/// subtracted from a role that inherits `cairn_window`), and `sign_then_reread` hands its re-read
/// straight to `written`.
#[test]
fn a_landed_sign_is_an_ok_with_or_without_its_pane() {
    let pane = || AllergyPane {
        strip: "Allergies not yet asked".into(),
        drug: "not-yet-asked",
        other: "not-yet-asked",
        drug_allergies: vec![],
        other_allergies: vec![],
        adverse_effects: vec![],
        previously_none: vec![],
        standing_none: vec![],
        unreadable: 0,
        charts: vec![],
    };
    let ok = written(Ok(pane()));
    assert_eq!(ok.outcome, "Recorded.");
    assert!(ok.pane.is_some());
    let failed = written(Err(ErrorView {
        text: "connection closed".into(),
        retry: Retry::Now,
    }));
    assert!(failed.pane.is_none());
    assert_eq!(
        failed.outcome,
        "Signed — but the allergy list could not be read again (connection closed). \
         Use “Read this chart again”."
    );
}

/// The pane carries what the node's view says about records this terminal cannot read, and the
/// standing "none known" statement with its signer, for allergies.js to draw.
#[test]
fn the_pane_carries_unreadable_records_and_the_standing_statement() {
    use cairn_node::allergy::read::NoneRow;
    let chart = Uuid::from_u128(7);
    let at = chrono::DateTime::from_timestamp(1_762_600_000, 0).unwrap();
    let a = PatientAllergies {
        entries: vec![],
        none: vec![NoneRow {
            event_id: Uuid::from_u128(8),
            chart,
            scope: "drug".into(),
            recorded_at: at,
            hlc: (at.timestamp_millis(), 0),
            signer: Some("Dr Bea".into()),
        }],
        doubted_charts: vec![],
        unreadable: 1,
    };
    let p = pane(&a, &ChartSet::single(chart));
    assert_eq!(p.unreadable, 1);
    assert!(
        p.strip
            .starts_with("1 allergy record cannot be read on this terminal · NKDA (asked "),
        "{}",
        p.strip
    );
    assert_eq!(p.standing_none.len(), 1);
    assert!(
        p.standing_none[0].starts_with("No known drug allergies — asked "),
        "{:?}",
        p.standing_none
    );
    assert!(p.standing_none[0].ends_with(" by Dr Bea"));
}
