//! Tests for `notes.rs`, the note pane's commands: the wrong-chart and fixture-mode refusals
//! (against `AppState::mock`, no Tauri runtime, as `commands_tests.rs` does for the medication
//! writes), and the pure wording and time-reading functions.
use super::*;
use crate::funnel::view::Retry;
use cairn_node::note::view::NoteGrade;
use chrono::{FixedOffset, LocalResult, NaiveDate};

fn on_screen() -> (AppState, String) {
    let chart = Uuid::from_u128(7);
    (AppState::mock(Some(chart)), chart.to_string())
}

fn another_chart() -> String {
    Uuid::from_u128(424_242).to_string()
}

fn a_draft() -> String {
    Uuid::from_u128(99).to_string()
}

/// Checked FIRST: whatever else would refuse, a sign naming a chart that is not on screen is
/// refused for THAT reason (Review Focus 5: the clinician switched charts mid-note).
#[tokio::test]
async fn sign_refuses_a_chart_not_on_screen() {
    let (state, _) = on_screen();
    let err = note_sign_impl(&state, &another_chart(), &a_draft(), "seen", None, None)
        .await
        .unwrap_err();
    assert!(err.text.contains("not the chart"), "{}", err.text);
    // A save as well: a debounced save for the chart just left must not land on it.
    let err = note_draft_save_impl(&state, &another_chart(), &a_draft(), "seen", None)
        .await
        .unwrap_err();
    assert!(err.text.contains("not the chart"), "{}", err.text);
}

/// With the right chart, fixture mode's own refusal is what the clinician reads — for a sign
/// and for a save — and the list reads as an empty pane, never a failure.
#[tokio::test]
async fn sign_in_fixture_mode_refuses() {
    let (state, chart) = on_screen();
    let err = note_sign_impl(&state, &chart, &a_draft(), "seen", None, None)
        .await
        .unwrap_err();
    assert_eq!(err.retry, Retry::Never);
    assert!(err.text.contains("fixture mode"), "{}", err.text);
    let err = note_draft_save_impl(&state, &chart, &a_draft(), "seen", None)
        .await
        .unwrap_err();
    assert!(err.text.contains("fixture mode"), "{}", err.text);
    let pane = note_list_impl(&state, &chart).await.unwrap();
    assert!(pane.notes.is_empty() && pane.drafts.is_empty());
    assert_eq!(pane.withheld_sentence, None);
}

#[test]
fn save_words_each_outcome() {
    let saved = save_view(SaveOutcome::Saved);
    assert!(saved.saved);
    assert_eq!(saved.sentence, None);
    let words = |o| save_view(o).sentence.expect("a refusal is worded");
    assert!(!save_view(SaveOutcome::Spent).saved);
    assert_eq!(
        words(SaveOutcome::Spent),
        "this note has already been signed"
    );
    assert_eq!(
        words(SaveOutcome::Elsewhere),
        "this draft belongs to another chart or terminal — nothing was saved"
    );
    assert_eq!(
        words(SaveOutcome::NoChart),
        "this patient has no chart on this node"
    );
}

/// Each refusal a sign can meet is a sentence, and the one a re-read mends (the note is already
/// signed: the list on screen is behind) is the only one that asks for it.
#[test]
fn sign_words_each_refusal() {
    let empty = sign_refusal_view(SignRefusal::Empty);
    assert_eq!(empty.text, "there is nothing to sign — the note is empty");
    assert_eq!(empty.retry, Retry::Never);
    let spent = sign_refusal_view(SignRefusal::Spent);
    assert_eq!(spent.text, "this note has already been signed");
    assert_eq!(spent.retry, Retry::AfterReread(Recorded::No));
    assert_eq!(
        sign_refusal_view(SignRefusal::Elsewhere).text,
        "this draft belongs to another chart or terminal — nothing was signed; your text is \
         still here"
    );
    assert_eq!(
        sign_refusal_view(SignRefusal::NoChart).text,
        "this patient has no chart on this node"
    );
}

/// What a sign refuses before it asks for the key — a passphrase that signs nothing is a step
/// paper never has: an empty note, a malformed draft id, a stated time that cannot be read.
#[test]
fn an_empty_note_or_a_bad_time_is_refused_before_the_key() {
    let empty = sign_checks_before_the_key(&a_draft(), "  \n ", None).unwrap_err();
    assert_eq!(empty.text, "there is nothing to sign — the note is empty");
    assert!(sign_checks_before_the_key("not-a-uuid", "seen", None).is_err());
    assert!(sign_checks_before_the_key(&a_draft(), "seen", Some("noon")).is_err());
    let (draft, stated) = sign_checks_before_the_key(&a_draft(), "seen", Some("")).unwrap();
    assert_eq!(draft.to_string(), a_draft());
    assert_eq!(stated, None);
}

fn utc(s: &str) -> DateTime<Utc> {
    s.parse().unwrap()
}

/// Brisbane, UTC+10, no daylight saving.
fn bne() -> FixedOffset {
    FixedOffset::east_opt(10 * 3600).unwrap()
}

#[test]
fn an_ordinary_local_time_reads_in_the_zone() {
    assert_eq!(
        stated_time_in("2026-11-03T03:00", &bne()),
        Ok(utc("2026-11-02T17:00:00Z"))
    );
    // A browser with a seconds step sends them; they are kept.
    assert_eq!(
        stated_time_in("2026-11-03T03:00:30", &bne()),
        Ok(utc("2026-11-02T17:00:30Z"))
    );
    assert!(stated_time_in("yesterday", &bne()).is_err());
}

#[test]
fn an_empty_stated_time_is_none() {
    assert_eq!(parse_stated(None), Ok(None));
    assert_eq!(parse_stated(Some("")), Ok(None));
    assert!(parse_stated(Some("2026-11-03T03:00")).unwrap().is_some());
}

/// Sydney's clock change, as the zone answers it (`LocalResult`): the doubled hour of 5 Apr 2026
/// (02:30 happens at +11 and again at +10) takes the EARLIER instant; the gap of 4 Oct 2026 (02:30
/// never happens) is refused with a sentence that says so. Driven through `.earliest()` and
/// `pick_local`, exactly as `stated_time_in` treats whatever the zone answers, because no zone with daylight saving
/// is in this tree (no chrono-tz) and `Local` is whatever the test machine is set to.
#[test]
fn a_clock_change_takes_the_earlier_time_and_refuses_the_gap() {
    let at = |offset_h: i32| {
        FixedOffset::east_opt(offset_h * 3600)
            .unwrap()
            .with_ymd_and_hms(2026, 4, 5, 2, 30, 0)
            .unwrap()
    };
    let doubled = LocalResult::Ambiguous(at(11), at(10));
    assert_eq!(
        pick_local("2026-04-05T02:30", doubled.earliest()),
        Ok(utc("2026-04-04T15:30:00Z"))
    );
    let gap =
        pick_local::<FixedOffset>("2026-10-04T02:30", LocalResult::None.earliest()).unwrap_err();
    assert!(gap.contains("2026-10-04T02:30"), "{gap}");
    assert!(gap.contains("clocks"), "{gap}");
}

fn draft(terminal: &str) -> NoteDraft {
    NoteDraft {
        draft_id: Uuid::from_u128(5),
        terminal_id: terminal.into(),
        narrative: "BP <90".into(),
        stated_time: Some(utc("2026-11-02T17:00:00Z")),
        recording_started: utc("2026-11-02T17:12:00Z"),
        last_keystroke: utc("2026-11-02T17:40:00Z"),
    }
}

fn nov(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, d).unwrap()
}

#[test]
fn a_draft_says_when_it_was_started_and_last_typed_and_where() {
    let here = draft_view(&draft("pc-1"), Some("pc-1"), &bne(), nov(3));
    assert!(here.here);
    assert_eq!(
        here.words,
        "unsigned draft · started 03:12 · last typed 03:40"
    );
    assert_eq!(here.narrative, "BP <90", "the raw text, to reopen");
    assert_eq!(here.stated_time.as_deref(), Some("2026-11-03T03:00"));
    let there = draft_view(&draft("pc-2"), Some("pc-1"), &bne(), nov(4));
    assert!(!there.here);
    assert_eq!(
        there.words,
        "unsigned draft · started 3 Nov 03:12 · last typed 3 Nov 03:40 \
         · being written at another terminal"
    );
}

#[test]
fn the_pane_words_each_note_and_any_withheld_ones() {
    // Last year, so the words carry the date whatever today is.
    let t = utc("2025-11-02T17:00:00Z");
    let list = PatientNoteList {
        notes: vec![NoteRow {
            event_id: Uuid::from_u128(1),
            chart: Uuid::from_u128(7),
            text: "BP <90".into(),
            t_effective: t,
            effective_basis: "stated".into(),
            recording_started: t,
            grade: NoteGrade::Signed {
                by: "Dr Ada".into(),
            },
        }],
        withheld_on_linked: 2,
        drafts: vec![draft("pc-1")],
    };
    let pane = note_pane(list, Some("pc-1"), &bne(), nov(3));
    let row = &pane.notes[0];
    assert_eq!(row.text, "BP <90");
    assert_eq!(row.byline, "signed by Dr Ada");
    assert_eq!(row.when, "3 Nov 2025 03:00 (stated)");
    assert!(pane.drafts[0].here);
    let withheld = pane.withheld_sentence.expect("withheld notes are said");
    assert!(withheld.starts_with("2 note(s)"), "{withheld}");
    let none = note_pane(
        PatientNoteList {
            notes: vec![],
            withheld_on_linked: 0,
            drafts: vec![],
        },
        Some("pc-1"),
        &bne(),
        nov(3),
    );
    assert_eq!(none.withheld_sentence, None);
}

/// The id is filled by `main`'s setup hook, which Tauri runs after it has built the window: a
/// note command served before that refuses, retryable, rather than writing under a blank name.
#[test]
fn a_note_command_before_the_terminal_id_is_read_refuses() {
    let (mut state, _) = on_screen();
    assert_eq!(terminal_of(&state).unwrap(), "fixture-terminal");
    state.terminal_id = std::sync::OnceLock::new();
    let err = terminal_of(&state).unwrap_err();
    assert_eq!(err.retry, Retry::Now);
    assert!(err.text.contains("terminal's id"), "{}", err.text);
}

/// A reopened draft's stated time comes back as the field holds it, seconds included, so the next
/// save or Sign sends the same instant — never one silently rounded to the minute.
#[test]
fn a_stated_time_with_seconds_survives_reopening() {
    let mut d = draft("pc-1");
    d.stated_time = Some(utc("2026-11-02T17:00:30Z"));
    let view = draft_view(&d, Some("pc-1"), &bne(), nov(3));
    assert_eq!(view.stated_time.as_deref(), Some("2026-11-03T03:00:30"));
    let back = stated_time_in(view.stated_time.as_deref().unwrap(), &bne());
    assert_eq!(back, Ok(utc("2026-11-02T17:00:30Z")));
}

/// Before setup has read this terminal's id, the list still shows every draft — none as this
/// terminal's, so none opens for editing here — rather than refusing to show the chart's notes.
#[test]
fn without_a_terminal_id_no_draft_is_here() {
    let list = PatientNoteList {
        notes: vec![],
        withheld_on_linked: 0,
        drafts: vec![draft("pc-1")],
    };
    let pane = note_pane(list, None, &bne(), nov(3));
    assert_eq!(pane.drafts.len(), 1);
    assert!(!pane.drafts[0].here);
}
