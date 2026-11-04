//! Tests for `notes.rs`, the note pane's commands: the wrong-chart and fixture-mode refusals
//! (against `AppState::mock`, no Tauri runtime, as `commands_tests.rs` does for the medication
//! writes), and the pure wording and time-reading functions.
use super::time::{parse_stated, pick_local, stated_time_in};
use super::*;
use crate::funnel::view::Retry;
use chrono::{FixedOffset, LocalResult, TimeZone};

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
    let err = note_sign_impl(
        &state,
        &another_chart(),
        &a_draft(),
        "seen",
        None,
        None,
        None,
    )
    .await
    .unwrap_err();
    assert!(err.text.contains("not the chart"), "{}", err.text);
    // A save as well: a debounced save for the chart just left must not land on it.
    let err = note_draft_save_impl(&state, &another_chart(), &a_draft(), "seen", None, None)
        .await
        .unwrap_err();
    assert!(err.text.contains("not the chart"), "{}", err.text);
    // And a take: Continue here acts only on the chart on screen.
    let err = note_draft_take_impl(&state, &another_chart(), &a_draft())
        .await
        .unwrap_err();
    assert!(err.text.contains("not the chart"), "{}", err.text);
}

/// With the right chart, fixture mode's own refusal is what the clinician reads — for a sign
/// and for a save — and the list reads as an empty pane, never a failure.
#[tokio::test]
async fn sign_in_fixture_mode_refuses() {
    let (state, chart) = on_screen();
    let err = note_sign_impl(&state, &chart, &a_draft(), "seen", None, None, None)
        .await
        .unwrap_err();
    assert_eq!(err.retry, Retry::Never);
    assert!(err.text.contains("fixture mode"), "{}", err.text);
    let err = note_draft_save_impl(&state, &chart, &a_draft(), "seen", None, None)
        .await
        .unwrap_err();
    assert!(err.text.contains("fixture mode"), "{}", err.text);
    let err = note_draft_take_impl(&state, &chart, &a_draft())
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
    assert_eq!(
        words(SaveOutcome::NoSuchWriter),
        "the name at this terminal is not an enrolled member — say who's here again"
    );
    assert!(!saved.moved && !save_view(SaveOutcome::Elsewhere).moved);
}

/// The old terminal's next save after a Continue elsewhere: `moved`, so the webview keeps what it
/// had not saved as a new draft, and says so in this one sentence.
#[test]
fn moved_sentence_exact() {
    let moved = save_view(SaveOutcome::Moved);
    assert!(moved.moved && !moved.saved);
    assert_eq!(
        moved.sentence.as_deref(),
        Some(
            "This note is now being continued at another terminal. What you typed since the \
             last save is kept here as a new draft."
        )
    );
}

#[test]
fn take_words_each_outcome() {
    let taken = take_view(TakeOutcome::Taken);
    assert!(taken.taken && taken.sentence.is_none());
    let words = |o| take_view(o).sentence.expect("a refusal is worded");
    assert_eq!(
        words(TakeOutcome::Spent),
        "this note has already been signed"
    );
    assert_eq!(
        words(TakeOutcome::Elsewhere),
        "this draft belongs to another chart or terminal — nothing was saved"
    );
    assert_eq!(words(TakeOutcome::Gone), "this draft is no longer open");
}

/// Global constraint: the writer of a save is the one the editor captured when it opened
/// (`noteDraft.writer`), never re-read from the current claim — a debounced save landing after a
/// switch to Dr B still names Nurse A, who typed it. Read from the source, because a fixture window
/// refuses before the door is reached: both doors are handed the `writer` the webview sent, and
/// neither impl reads who's here.
#[test]
fn save_passes_the_given_writer_not_the_claim() {
    let src = include_str!("notes.rs");
    let flat = |f: &str| -> String {
        crate::source_guard::rust_fn(src, f)
            .split_whitespace()
            .collect()
    };
    let save = flat("note_draft_save_impl");
    assert!(
        save.contains("save_part(&*db,draft,patient,terminal,writer,"),
        "{save}"
    );
    let sign = flat("note_sign_impl");
    assert!(
        sign.contains("SignNoteInput{draft,patient,terminal,writer,"),
        "{sign}"
    );
    for body in [save, sign] {
        assert!(!body.contains("whos_here"), "{body}");
    }
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
    assert_eq!(
        sign_refusal_view(SignRefusal::Moved).text,
        sign_refusal_view(SignRefusal::Elsewhere).text
    );
    let gone = sign_refusal_view(SignRefusal::NoSuchWriter).text;
    assert!(gone.contains("not enrolled"), "{gone}");
    assert!(
        gone.contains("nothing was signed; your text is still here"),
        "a sign's refusal says nothing was SIGNED: {gone}"
    );
}

/// What a sign refuses before it asks for the key — a passphrase that signs nothing is a step
/// paper never has: an empty note, a malformed draft id, a stated time that cannot be read.
#[test]
fn an_empty_note_or_a_bad_time_is_refused_before_the_key() {
    let empty = sign_checks_before_the_key(&a_draft(), "  \n ", false, None).unwrap_err();
    assert_eq!(empty.text, "there is nothing to sign — the note is empty");
    assert!(sign_checks_before_the_key("not-a-uuid", "seen", false, None).is_err());
    assert!(sign_checks_before_the_key(&a_draft(), "seen", false, Some("noon")).is_err());
    let (draft, stated) = sign_checks_before_the_key(&a_draft(), "seen", false, Some("")).unwrap();
    assert_eq!(draft.to_string(), a_draft());
    assert_eq!(stated, None);
}

/// Carry 2: the node signs a draft whose on-screen part is blank when a part above it holds text
/// (blank parts are skipped), so the window asks for the key then — and refuses only when the
/// on-screen text AND every other part are blank.
#[test]
fn a_blank_editor_under_a_written_part_still_signs() {
    assert!(sign_checks_before_the_key(&a_draft(), " ", true, None).is_ok());
    let empty = sign_checks_before_the_key(&a_draft(), " ", false, None).unwrap_err();
    assert_eq!(empty.text, "there is nothing to sign — the note is empty");
}

/// Final review M2: `datetime-local` takes years up to 275760, so a slip like "20266" reads as a
/// time, and the floor's wire pin (db/001 `cairn_t_effective`: a four-digit year; PostgreSQL has
/// no year 0) would refuse it only AFTER the passphrase. Said before the key, naming the year.
/// The SAVE still keeps such a time: refusing it there would leave the text unsaved.
#[test]
fn a_stated_year_the_record_cannot_hold_is_refused_before_the_key() {
    let far = sign_checks_before_the_key(&a_draft(), "seen", false, Some("20266-11-03T03:00"))
        .unwrap_err();
    assert_eq!(
        far.text,
        "the time it happened, 20266-11-03T03:00, has a year the record cannot hold \
         (years 1 to 9999) — check the year you typed"
    );
    assert!(
        sign_checks_before_the_key(&a_draft(), "seen", false, Some("0000-06-01T12:00")).is_err()
    );
    assert!(
        sign_checks_before_the_key(&a_draft(), "seen", false, Some("9999-12-31T09:00")).is_ok()
    );
    assert!(
        parse_stated(Some("20266-11-03T03:00")).is_ok(),
        "a save keeps it"
    );
}

pub(crate) fn utc(s: &str) -> DateTime<Utc> {
    s.parse().unwrap()
}

/// Brisbane, UTC+10, no daylight saving.
pub(crate) fn bne() -> FixedOffset {
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
/// never happens) is refused with a sentence that says so. Driven through `pick_local` with the
/// zone's whole answer, exactly as `stated_time_in` hands it over, because no zone with daylight
/// saving is in this tree (no chrono-tz) and `Local` is whatever the test machine is set to.
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
        pick_local("2026-04-05T02:30", doubled),
        Ok(utc("2026-04-04T15:30:00Z"))
    );
    let gap = pick_local::<FixedOffset>("2026-10-04T02:30", LocalResult::None).unwrap_err();
    assert!(gap.contains("2026-10-04T02:30"), "{gap}");
    assert!(gap.contains("clocks"), "{gap}");
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
