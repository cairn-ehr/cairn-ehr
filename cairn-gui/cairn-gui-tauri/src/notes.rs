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
//!   save names (`AppState::terminal_id`, read through `terminal_of`) and answers `moved` or
//!   `elsewhere`;
//! * read who's here when it saves or signs — the writer is the one the editor captured when it
//!   opened (`noteDraft.writer`) and sends with every save, so a debounced save landing after a
//!   switch still names the person who typed it. Only the LIST reads the current claim, to say
//!   which part the editor may open on (`notes_parts.rs`).
use crate::commands::record_timing;
use crate::funnel::view::{ErrorView, Recorded, Retry};
use crate::link::judgement_view::{refused, stale};
use crate::signer::Credential;
use crate::state::{AppState, Now};
use crate::write_failure::{chart_read_failed, key_locked, node_cannot_write, node_failure};
use cairn_node::note::draft::{SaveOutcome, TakeOutcome};
use cairn_node::note::sign::SignRefusal;
use chrono::{DateTime, Utc};
use parts::{note_pane, others_have_text, NotePane};
use std::time::Instant;
use time::{parse_stated, year_the_record_holds};
use uuid::Uuid;

/// Reading the stated time (`notes_time.rs`).
#[path = "notes_time.rs"]
mod time;

/// The pane's views: its notes, and each draft's parts (`notes_parts.rs`).
#[path = "notes_parts.rs"]
pub(crate) mod parts;

/// What a draft save did. `saved: false` always carries the sentence why nothing was written.
#[derive(Debug, PartialEq, Eq, serde::Serialize)]
pub struct SaveView {
    pub saved: bool,
    /// The draft was taken to another terminal: the webview keeps what it had not saved as a NEW
    /// draft of its own, so nothing typed is lost.
    pub moved: bool,
    pub sentence: Option<String>,
}

/// What a "Continue here" did. `taken: false` always carries the sentence why nothing moved.
#[derive(Debug, PartialEq, Eq, serde::Serialize)]
pub struct TakeView {
    pub taken: bool,
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

/// Save the text on screen into its draft as `writer`'s part (the webview calls this, debounced,
/// as the clinician types). `writer` is the editor's captured staff id; null = "Don't know".
#[tauri::command]
pub async fn note_draft_save(
    state: tauri::State<'_, AppState>,
    patient_id: String,
    draft_id: String,
    narrative: String,
    stated_time: Option<String>,
    writer: Option<String>,
) -> Result<SaveView, ErrorView> {
    note_draft_save_impl(
        &state,
        &patient_id,
        &draft_id,
        &narrative,
        stated_time.as_deref(),
        writer.as_deref(),
    )
    .await
}

/// "Continue here": move a draft held by another terminal to this one.
#[tauri::command]
pub async fn note_draft_take(
    state: tauri::State<'_, AppState>,
    patient_id: String,
    draft_id: String,
) -> Result<TakeView, ErrorView> {
    note_draft_take_impl(&state, &patient_id, &draft_id).await
}

/// Sign the note on screen — the text sent WITH Sign, never an older autosave (Review Focus 4).
#[tauri::command]
pub async fn note_sign(
    state: tauri::State<'_, AppState>,
    patient_id: String,
    draft_id: String,
    narrative: String,
    stated_time: Option<String>,
    writer: Option<String>,
    credential: Option<Credential>,
) -> Result<NoteSignReport, ErrorView> {
    note_sign_impl(
        &state,
        &patient_id,
        &draft_id,
        &narrative,
        stated_time.as_deref(),
        writer.as_deref(),
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
/// … when a SIGN finds its draft taken to another terminal (Task 5 review M1): not "another
/// chart" — the clinician is told where the note went.
const SIGN_MOVED: &str = "This note is now being continued at another terminal — nothing was \
     signed; your text is still here.";
/// … when the writer a save names is not an enrolled member (any more).
const NO_SUCH_WRITER: &str =
    "the name at this terminal is not an enrolled member — say who's here again";
/// … when a save finds its draft taken to another terminal.
const MOVED: &str = "This note is now being continued at another terminal. What you typed since \
     the last save is kept here as a new draft.";
/// … when a take finds no such draft (never saved, or discarded).
const GONE: &str = "this draft is no longer open";
/// … the same, for a SIGN: the on-screen writer, or the writer of a part above, is no longer
/// enrolled. Nothing was signed, and the editor still holds the text.
const SIGN_NO_SUCH_WRITER: &str = "a writer of this note is not enrolled on this node — nothing \
     was signed; your text is still here";
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
    // The claim decides only which part the editor may open on; a save never reads it.
    let claim = state.whos_here.lock().await.claim.clone();
    Ok(note_pane(list, terminal, &claim, &chrono::Local, today))
}

/// Save the text on screen as `writer`'s part of draft `draft_id` of the chart on screen, held by
/// this terminal. A save that wrote nothing (`spent`, `moved`, `elsewhere`, `no-chart`,
/// `no-such-writer`) is an `Ok` with its sentence, not an error: the connection worked, and the
/// webview keeps the text either way.
pub async fn note_draft_save_impl(
    state: &AppState,
    patient_id: &str,
    draft_id: &str,
    narrative: &str,
    stated_time: Option<&str>,
    writer: Option<&str>,
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
    let outcome = cairn_node::note::draft::save_part(
        &*db, draft, patient, terminal, writer, narrative, stated,
    )
    .await
    .map_err(|e| node_failure(&db.explain(e)))?;
    Ok(save_view(outcome))
}

/// "Continue here": move draft `draft_id` of the chart on screen to this terminal — one statement
/// on the node. Its old terminal's next save then answers `moved`. Refusals are `Ok` with their
/// sentence, as a save's are.
pub async fn note_draft_take_impl(
    state: &AppState,
    patient_id: &str,
    draft_id: &str,
) -> Result<TakeView, ErrorView> {
    let patient = state.displayed_patient(patient_id).await.map_err(refused)?;
    if state.is_mock() {
        return Err(refused(FIXTURE));
    }
    let draft: Uuid = draft_id.parse().map_err(|_| refused("not a draft id"))?;
    let terminal = terminal_of(state)?;
    let db = state
        .db
        .as_ref()
        .ok_or_else(|| node_cannot_write("no database connection"))?
        .lock()
        .await?;
    let outcome = cairn_node::note::draft::take_draft(&*db, draft, patient, terminal)
        .await
        .map_err(|e| node_failure(&db.explain(e)))?;
    Ok(take_view(outcome))
}

/// Sign the note: the steps of `commands::cease_impl`, for one event. The chart on screen first,
/// then fixture mode, then every refusal that is a verdict on the gesture (empty text, a stated
/// time that cannot be read), and only then the key — a passphrase that signs nothing is a step
/// paper never has. The raw text goes to `sign_note`, which escapes it, with `writer` — the
/// editor's captured writer, so the signer's own part signs `authored`, not `attested`.
pub async fn note_sign_impl(
    state: &AppState,
    patient_id: &str,
    draft_id: &str,
    narrative: &str,
    stated_time: Option<&str>,
    writer: Option<&str>,
    credential: Option<Credential>,
) -> Result<NoteSignReport, ErrorView> {
    let patient = state.displayed_patient(patient_id).await.map_err(refused)?;
    if state.is_mock() {
        return Err(refused(FIXTURE));
    }
    // Asked only of a blank editor: whether a part above holds text the node would sign.
    let others = narrative.trim().is_empty()
        && other_parts_have_text(state, patient, draft_id, writer).await?;
    let (draft, stated) = sign_checks_before_the_key(draft_id, narrative, others, stated_time)?;
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
            writer,
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

/// Whether draft `draft_id`'s stored parts hold text the on-screen text does not replace
/// (`parts::others_have_text`). A draft not yet saved, or an unreadable id, has none.
async fn other_parts_have_text(
    state: &AppState,
    patient: Uuid,
    draft_id: &str,
    writer: Option<&str>,
) -> Result<bool, ErrorView> {
    let Ok(draft) = draft_id.parse::<Uuid>() else {
        return Ok(false);
    };
    let db = state
        .db
        .as_ref()
        .ok_or_else(|| node_cannot_write("no database connection"))?
        .lock()
        .await?;
    let drafts = cairn_node::note::draft::open_drafts(&*db, patient)
        .await
        .map_err(|e| chart_read_failed(db.explain(e)))?;
    Ok(drafts
        .iter()
        .find(|d| d.draft_id == draft)
        .is_some_and(|d| others_have_text(&d.parts, writer)))
}

/// The refusals a sign says before it asks for the key, and the draft id and stated time read
/// from what the webview sent. `others_have_text`: a part the on-screen text does not replace
/// holds text, so a blank editor still signs. Pure.
pub(crate) fn sign_checks_before_the_key(
    draft_id: &str,
    narrative: &str,
    others_have_text: bool,
    stated_time: Option<&str>,
) -> Result<(Uuid, Option<DateTime<Utc>>), ErrorView> {
    // `sign_note` refuses an all-blank draft too; asked here so it is said before the passphrase.
    if narrative.trim().is_empty() && !others_have_text {
        return Err(sign_refusal_view(SignRefusal::Empty));
    }
    let draft = draft_id.parse().map_err(|_| refused("not a draft id"))?;
    let stated = parse_stated(stated_time).map_err(refused)?;
    if let (Some(s), Some(t)) = (stated_time, stated) {
        year_the_record_holds(s, t).map_err(refused)?;
    }
    Ok((draft, stated))
}

/// A save's answer as the webview reads it. Pure.
pub(crate) fn save_view(outcome: SaveOutcome) -> SaveView {
    let sentence = match outcome {
        SaveOutcome::Saved => None,
        SaveOutcome::Spent => Some(SPENT),
        SaveOutcome::Moved => Some(MOVED),
        SaveOutcome::Elsewhere => Some(ELSEWHERE),
        SaveOutcome::NoChart => Some(NO_CHART),
        SaveOutcome::NoSuchWriter => Some(NO_SUCH_WRITER),
    };
    SaveView {
        saved: sentence.is_none(),
        moved: outcome == SaveOutcome::Moved,
        sentence: sentence.map(str::to_string),
    }
}

/// A take's answer as the webview reads it. Pure.
pub(crate) fn take_view(outcome: TakeOutcome) -> TakeView {
    let sentence = match outcome {
        TakeOutcome::Taken => None,
        TakeOutcome::Spent => Some(SPENT),
        TakeOutcome::Elsewhere => Some(ELSEWHERE),
        TakeOutcome::Gone => Some(GONE),
    };
    TakeView {
        taken: sentence.is_none(),
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
        SignRefusal::Moved => refused(SIGN_MOVED),
        SignRefusal::NoChart => refused(NO_CHART),
        SignRefusal::NoSuchWriter => refused(SIGN_NO_SUCH_WRITER),
    }
}

#[cfg(test)]
#[path = "notes_tests.rs"]
mod tests;
