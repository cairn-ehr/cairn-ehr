//! The note pane's commands (sign-in slice 3a): list a chart's notes and drafts, save a draft as
//! it is typed, and sign it.
//!
//! Thin adapters over `cairn_node::note`, split as `commands.rs` is: each `#[tauri::command]`
//! forwards to a plain `*_impl(&AppState, …)` so the one rule a wrong-chart defect would break —
//! act only on the chart on SCREEN (`AppState::displayed_patient`) — is tested against
//! `AppState::mock` with no Tauri runtime (`notes_tests.rs`). Every sentence the clinician reads is
//! made by a pure function here, so its wording is pinned without a database or a webview.
//!
//! Three things the window deliberately does NOT do, because the node does them:
//! * escape markup — `sign_note` escapes the raw text once, at signing (ADR-0041); drafts and
//!   this file carry the text exactly as typed;
//! * send a time for recording — the save door stamps `recording_started` and `last_keystroke`
//!   from the database's clock; the only time sent is the writer's own STATED time;
//! * decide whether a draft is this terminal's to edit — the door compares the terminal id each
//!   save names (`AppState::terminal_id`, read through `terminal_of`) and answers `elsewhere`.
use crate::commands::record_timing;
use crate::funnel::view::{ErrorView, Recorded, Retry};
use crate::link::judgement_view::{refused, stale};
use crate::signer::Credential;
use crate::state::{AppState, Now};
use crate::write_failure::{chart_read_failed, key_locked, node_cannot_write, node_failure};
use cairn_node::note::draft::{NoteDraft, SaveOutcome};
use cairn_node::note::read::{NoteRow, PatientNoteList};
use cairn_node::note::sign::SignRefusal;
use cairn_node::note::view::{grade_words, instant, time_words};
use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeZone, Utc};
use std::time::Instant;
use uuid::Uuid;

/// One signed note, as the pane shows it.
#[derive(Debug, serde::Serialize)]
pub struct NoteRowView {
    pub event_id: String,
    /// The text exactly as its writer typed it. The webview sets it as `textContent`.
    pub text: String,
    /// Who signed it, graded (`view::grade_words`): "signed by …", a claim, or writer not known.
    pub byline: String,
    /// When it applies (`view::time_words`): a stated time, or when writing began.
    pub when: String,
}

/// One open draft on the chart.
#[derive(Debug, serde::Serialize)]
pub struct DraftView {
    pub draft_id: String,
    /// The RAW text, so a draft of this terminal reopens into the editor as it was typed.
    pub narrative: String,
    /// The writer's stated time as the `datetime-local` field holds it (`2026-11-03T03:00`, this
    /// machine's zone; with seconds when it has them), `None` when none was stated.
    pub stated_time: Option<String>,
    /// Whether the draft was started at THIS terminal — the only place it may be edited.
    pub here: bool,
    /// "unsigned draft · started 03:12 · last typed 03:40", plus where it is being written.
    pub words: String,
}

/// What the note pane shows for the chart on screen.
#[derive(Debug, serde::Serialize)]
pub struct NotePane {
    pub notes: Vec<NoteRowView>,
    /// Said when notes on a linked chart are held back because a link is in doubt.
    pub withheld_sentence: Option<String>,
    pub drafts: Vec<DraftView>,
}

/// What a draft save did. `saved: false` always carries the sentence why nothing was written.
#[derive(Debug, PartialEq, Eq, serde::Serialize)]
pub struct SaveView {
    pub saved: bool,
    pub sentence: Option<String>,
}

/// What one Sign did: signed, or — its commit got no ordinary answer (#814) — NOT known.
#[derive(Debug, serde::Serialize)]
pub struct NoteSignReport {
    pub text: String,
    /// `Unknown` when the outcome is not known: the pane may be behind the record.
    pub behind_the_record: Option<Recorded>,
}

/// The chart's notes and open drafts.
#[tauri::command]
pub async fn note_list(
    state: tauri::State<'_, AppState>,
    patient_id: String,
) -> Result<NotePane, ErrorView> {
    note_list_impl(&state, &patient_id).await
}

/// Save the text on screen into its draft (the webview calls this, debounced, as the clinician
/// types).
#[tauri::command]
pub async fn note_draft_save(
    state: tauri::State<'_, AppState>,
    patient_id: String,
    draft_id: String,
    narrative: String,
    stated_time: Option<String>,
) -> Result<SaveView, ErrorView> {
    note_draft_save_impl(
        &state,
        &patient_id,
        &draft_id,
        &narrative,
        stated_time.as_deref(),
    )
    .await
}

/// Sign the note on screen — the text sent WITH Sign, never an older autosave (Review Focus 4).
#[tauri::command]
pub async fn note_sign(
    state: tauri::State<'_, AppState>,
    patient_id: String,
    draft_id: String,
    narrative: String,
    stated_time: Option<String>,
    credential: Option<Credential>,
) -> Result<NoteSignReport, ErrorView> {
    note_sign_impl(
        &state,
        &patient_id,
        &draft_id,
        &narrative,
        stated_time.as_deref(),
        credential,
    )
    .await
}

/// The sentence a save or sign answers when the draft was already signed.
const SPENT: &str = "this note has already been signed";
/// … when a SAVE's draft is another chart's or another terminal's.
const ELSEWHERE: &str = "this draft belongs to another chart or terminal — nothing was saved";
/// … the same, for a SIGN: nothing was signed, and the editor still holds the text.
const SIGN_ELSEWHERE: &str =
    "this draft belongs to another chart or terminal — nothing was signed; your text is still here";
/// … when no event names the patient on this node.
const NO_CHART: &str = "this patient has no chart on this node";
/// … when nothing but whitespace was typed.
const EMPTY: &str = "there is nothing to sign — the note is empty";
/// Fixture mode's refusal, worded as every other write's (`commands::cease_impl`).
const FIXTURE: &str = "fixture mode: this window is showing mock data and cannot write";

/// Read the chart's notes (over its linked charts, ADR-0076) and open drafts, worded in this
/// machine's zone. Fixture mode has no notes: it reads as an empty pane, never a failure.
pub async fn note_list_impl(state: &AppState, patient_id: &str) -> Result<NotePane, ErrorView> {
    // Bound to the chart the webview is showing: a read for a chart since left must not be drawn
    // under the next one's header.
    let patient = state.displayed_patient(patient_id).await.map_err(refused)?;
    let Some(db) = state.db.as_ref() else {
        return Ok(NotePane {
            notes: vec![],
            withheld_sentence: None,
            drafts: vec![],
        });
    };
    // Not `terminal_of`: a read never waits for the id. Unset, no draft reads as this terminal's.
    let terminal = state.terminal_id.get().map(String::as_str);
    let mut db = db.lock().await?;
    // ONE snapshot (ruling R12): the notes and the drafts are two statements, and a sign
    // committing between them would show the clinician neither the note nor its draft.
    let list = cairn_node::medication::snapshot::in_snapshot(&mut db, async |tx| {
        cairn_node::note::read::list_patient_notes(tx, patient).await
    })
    .await
    .map_err(|e| chart_read_failed(db.explain(e)))?;
    // The window supplies the zone (ruling R10): the node never knows the terminal's.
    let today = Utc::now().with_timezone(&chrono::Local).date_naive();
    Ok(note_pane(list, terminal, &chrono::Local, today))
}

/// Save the text on screen into draft `draft_id` of the chart on screen, held by this terminal.
/// A save that wrote nothing (`spent`, `elsewhere`, `no-chart`) is an `Ok` with its sentence, not
/// an error: the connection worked, and the webview keeps the text either way.
pub async fn note_draft_save_impl(
    state: &AppState,
    patient_id: &str,
    draft_id: &str,
    narrative: &str,
    stated_time: Option<&str>,
) -> Result<SaveView, ErrorView> {
    // The save names the chart the webview is showing, so a debounced save that fires after the
    // clinician switched charts is refused here, never written to the chart now open.
    let patient = state.displayed_patient(patient_id).await.map_err(refused)?;
    if state.is_mock() {
        return Err(refused(FIXTURE));
    }
    let draft: Uuid = draft_id.parse().map_err(|_| refused("not a draft id"))?;
    let stated = parse_stated(stated_time).map_err(refused)?;
    let terminal = terminal_of(state)?;
    let db = state
        .db
        .as_ref()
        .ok_or_else(|| node_cannot_write("no database connection"))?
        .lock()
        .await?;
    let outcome =
        cairn_node::note::draft::save_draft(&*db, draft, patient, terminal, narrative, stated)
            .await
            .map_err(|e| node_failure(&db.explain(e)))?;
    Ok(save_view(outcome))
}

/// Sign the note: the steps of `commands::cease_impl`, for one event. The chart on screen first,
/// then fixture mode, then every refusal that is a verdict on the gesture (empty text, a stated
/// time that cannot be read), and only then the key — a passphrase that signs nothing is a step
/// paper never has. The raw text goes to `sign_note`, which escapes it.
pub async fn note_sign_impl(
    state: &AppState,
    patient_id: &str,
    draft_id: &str,
    narrative: &str,
    stated_time: Option<&str>,
    credential: Option<Credential>,
) -> Result<NoteSignReport, ErrorView> {
    let patient = state.displayed_patient(patient_id).await.map_err(refused)?;
    if state.is_mock() {
        return Err(refused(FIXTURE));
    }
    let (draft, stated) = sign_checks_before_the_key(draft_id, narrative, stated_time)?;
    let terminal = terminal_of(state)?;
    let (human_sk, human_kid) = state
        .signer_for_act(credential, Now::read())
        .await
        .map_err(|r| key_locked(&r))?;
    let node_sk = state
        .node_sk
        .as_ref()
        .ok_or_else(|| node_cannot_write("no node key"))?;

    let started = Instant::now();
    let result = {
        let mut db = state
            .db
            .as_ref()
            .ok_or_else(|| node_cannot_write("no database connection"))?
            .lock()
            .await?;
        let input = cairn_node::note::sign::SignNoteInput {
            draft,
            patient,
            terminal,
            narrative,
            stated,
            node_origin: &state.node_origin,
        };
        let author = cairn_node::medication::AuthorParams {
            human_sk: &human_sk,
            human_kid: &human_kid,
        };
        cairn_node::note::sign::sign_note(&mut db, node_sk, &input, &author)
            .await
            .map_err(|e| db.explain(e))
    };
    // ONE write, so one outcome: signed, refused (nothing written), NOT known (#814), or an
    // ordinary failure returned as the node's own sentence.
    let (text, behind_the_record) = match result {
        Ok(Ok(_signed)) => ("Signed — the note is on the chart.".to_string(), None),
        Ok(Err(refusal)) => return Err(sign_refusal_view(refusal)),
        Err(e) => match cairn_node::db_diagnosis::commit_outcome_unknown_event(&e) {
            Some(event) => (
                format!(
                    "Whether this note was signed is NOT known — no ordinary answer came back \
                     while saving it (event {event}: {}). Once the window can reach the node \
                     again, the chart shows whether the note is there.",
                    cairn_gui_live::cause_of(&e)
                ),
                Some(Recorded::Unknown),
            ),
            None => return Err(node_failure(&e)),
        },
    };
    // A gesture with an unknown outcome is no timing sample (#818 review).
    if behind_the_record.is_none() {
        let elapsed_ms = started.elapsed().as_millis().min(i32::MAX as u128) as i32;
        record_timing(state, "note-sign", 1, elapsed_ms).await;
    }
    Ok(NoteSignReport {
        text,
        behind_the_record,
    })
}

/// This terminal's id, or a refusal while `main`'s setup hook has not yet read it.
///
/// Tauri 2 builds the config-declared window BEFORE it runs the setup hook (`tauri` 2.11.5,
/// `src/app.rs:2524-2532`), and the state is managed at builder time so no command ever meets
/// unmanaged state; the id alone arrives in setup. Nothing in Tauri's contract says a command
/// cannot be served in between, so a WRITE served then says so rather than writing under a blank
/// name. (`note_list` never waits for it: it shows every draft as another terminal's instead.)
pub(crate) fn terminal_of(state: &AppState) -> Result<&str, ErrorView> {
    state
        .terminal_id
        .get()
        .map(String::as_str)
        .ok_or(ErrorView {
            text: "this window is still starting — it has not yet read this terminal's id; \
               nothing was done, try again"
                .to_string(),
            retry: Retry::Now,
        })
}

/// The refusals a sign says before it asks for the key, and the draft id and stated time read
/// from what the webview sent. Pure.
pub(crate) fn sign_checks_before_the_key(
    draft_id: &str,
    narrative: &str,
    stated_time: Option<&str>,
) -> Result<(Uuid, Option<DateTime<Utc>>), ErrorView> {
    // `sign_note` refuses this too; asked here as well so it is said before the passphrase.
    if narrative.trim().is_empty() {
        return Err(sign_refusal_view(SignRefusal::Empty));
    }
    let draft = draft_id.parse().map_err(|_| refused("not a draft id"))?;
    let stated = parse_stated(stated_time).map_err(refused)?;
    Ok((draft, stated))
}

/// A save's answer as the webview reads it. Pure.
pub(crate) fn save_view(outcome: SaveOutcome) -> SaveView {
    let sentence = match outcome {
        SaveOutcome::Saved => None,
        SaveOutcome::Spent => Some(SPENT),
        SaveOutcome::Elsewhere => Some(ELSEWHERE),
        SaveOutcome::NoChart => Some(NO_CHART),
    };
    SaveView {
        saved: sentence.is_none(),
        sentence: sentence.map(str::to_string),
    }
}

/// A sign's refusal as the webview reads it. Pure. Only "already signed" asks for a re-read: the
/// pane on screen is behind the record, and the re-read shows the signed note. The others are
/// verdicts a re-read cannot mend.
pub(crate) fn sign_refusal_view(refusal: SignRefusal) -> ErrorView {
    match refusal {
        SignRefusal::Empty => refused(EMPTY),
        SignRefusal::Spent => stale(SPENT),
        SignRefusal::Elsewhere => refused(SIGN_ELSEWHERE),
        SignRefusal::NoChart => refused(NO_CHART),
    }
}

/// The read as the pane shows it, in zone `tz` with `today` its date. Pure.
pub(crate) fn note_pane<Tz: TimeZone>(
    list: PatientNoteList,
    terminal: Option<&str>,
    tz: &Tz,
    today: NaiveDate,
) -> NotePane
where
    Tz::Offset: std::fmt::Display,
{
    let withheld_sentence = (list.withheld_on_linked > 0).then(|| {
        format!(
            "{} note(s) on this patient's linked charts are not shown: a link in this record is in \
             doubt, so the node cannot yet vouch that they are this patient's. They are shown \
             once a clinician has judged the record's links (\"How these charts are linked\").",
            list.withheld_on_linked
        )
    });
    NotePane {
        notes: list.notes.iter().map(|n| note_row_view(n, tz)).collect(),
        withheld_sentence,
        drafts: list
            .drafts
            .iter()
            .map(|d| draft_view(d, terminal, tz, today))
            .collect(),
    }
}

/// One note's words. `time_words` takes today from the clock itself.
fn note_row_view<Tz: TimeZone>(note: &NoteRow, tz: &Tz) -> NoteRowView
where
    Tz::Offset: std::fmt::Display,
{
    NoteRowView {
        event_id: note.event_id.to_string(),
        text: note.text.clone(),
        byline: grade_words(&note.grade),
        when: time_words(
            &note.effective_basis,
            note.t_effective,
            note.recording_started,
            tz,
        ),
    }
}

/// One draft's words, and whether it is this terminal's to edit. Pure.
pub(crate) fn draft_view<Tz: TimeZone>(
    draft: &NoteDraft,
    terminal: Option<&str>,
    tz: &Tz,
    today: NaiveDate,
) -> DraftView
where
    Tz::Offset: std::fmt::Display,
{
    // `None`: this terminal's id is not read yet, so no draft can be known to be this one's.
    let here = terminal == Some(draft.terminal_id.as_str());
    let mut words = format!(
        "unsigned draft · started {} · last typed {}",
        instant(draft.recording_started, tz, today),
        instant(draft.last_keystroke, tz, today)
    );
    if !here {
        words.push_str(" · being written at another terminal");
    }
    DraftView {
        draft_id: draft.draft_id.to_string(),
        narrative: draft.narrative.clone(),
        stated_time: draft.stated_time.map(|t| datetime_local(t, tz)),
        here,
        words,
    }
}

/// `t` as a `datetime-local` value in `tz`: minutes only when that is exact, else with its seconds
/// (and any fraction), so a reopened draft sends back the instant it holds, never one rounded to
/// the minute. Both forms are valid field values and both parse in [`stated_time_in`]. Pure.
fn datetime_local<Tz: TimeZone>(t: DateTime<Utc>, tz: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    use chrono::Timelike;
    let format = if t.second() == 0 && t.nanosecond() == 0 {
        "%Y-%m-%dT%H:%M"
    } else {
        "%Y-%m-%dT%H:%M:%S%.f"
    };
    t.with_timezone(tz).format(format).to_string()
}

/// The webview's stated-time field: absent or empty is "not stated" (principle 4), else a time.
pub(crate) fn parse_stated(stated_time: Option<&str>) -> Result<Option<DateTime<Utc>>, String> {
    match stated_time {
        None | Some("") => Ok(None),
        Some(s) => stated_time_from_local(s).map(Some),
    }
}

/// A `datetime-local` value (`"2026-11-03T03:00"`, no zone) read in this machine's zone — the
/// zone the clinician reads the wall clock in.
pub fn stated_time_from_local(s: &str) -> Result<DateTime<Utc>, String> {
    stated_time_in(s, &chrono::Local)
}

/// [`stated_time_from_local`] in zone `tz`, so the rule is testable without the machine's zone.
pub(crate) fn stated_time_in<Tz: TimeZone>(s: &str, tz: &Tz) -> Result<DateTime<Utc>, String> {
    // With or without seconds: a browser sends them when the field's step asks for them.
    let naive = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f")
        .or_else(|_| NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M"))
        .map_err(|_| format!("not a time: {s}"))?;
    // At a clock change a wall time can happen twice (clocks go back): `earliest` takes the
    // first time the clock showed it.
    pick_local(s, tz.from_local_datetime(&naive).earliest())
}

/// The instant a wall-clock time `s` means, given the zone's earliest answer for it. Pure.
///
/// `None` is a time that never happened (clocks went forward past it): refused, with a sentence
/// naming it — any instant chosen for it would be a time the clinician never saw on a clock.
pub(crate) fn pick_local<Tz: TimeZone>(
    s: &str,
    earliest: Option<DateTime<Tz>>,
) -> Result<DateTime<Utc>, String> {
    match earliest {
        Some(t) => Ok(t.with_timezone(&Utc)),
        None => Err(format!(
            "{s} never happened on this machine's clock — the clocks moved forward past it; \
             choose a time before or after the change"
        )),
    }
}

#[cfg(test)]
#[path = "notes_tests.rs"]
mod tests;
