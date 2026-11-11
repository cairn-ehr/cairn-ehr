//! The medication list (batch charting slice 1): the window's commands over a chart's durable,
//! per-writer list of UNSIGNED medication lines (db/078). Sign is `med_list_sign.rs`.
//!
//! - [`med_list_read`]: the writer's lines, each re-checked against the allergies read NOW and
//!   given its default tick (design §3.2). The writer is who's here's claim: a list is shown only
//!   to the person it was typed by (a NULL-writer list only while nobody is named).
//! - [`med_list_save`]: save one line as typed (the window autosaves). The door (db/078) decides
//!   what a hand tick keeps; this sends it the warnings on the line NOW, built here from a fresh
//!   check, never taken from the webview.
//! - [`med_list_delete`]: remove one line, leaving no record (design §9 Q3).
//! - [`med_list_take`]: "Continue here", moving the list from another terminal to this one.
//!
//! WHY the ticks are derived here and not in the webview: the default tick is never stored, and
//! Sign re-derives every tick from a fresh read in Rust (`med_list_sign.rs`). The read and the Sign
//! share `med_list_judge.rs`'s `fresh_read` and `judge`, so the screen and the Sign judge a line by the same rules.
//!
//! The answers' JSON shapes are the structs below, as `serde` writes them (snake_case keys).
use crate::chart_set::check_displayed_set;
use crate::funnel::view::ErrorView;
use crate::link::judgement_view::refused;
use crate::med_chart::{press, view, LineCheckView, PartArg};
use crate::med_line_sign::{fresh_check, parts_before_the_key, the_line};
pub(crate) use crate::med_list_judge::{fresh_read, judge, Judged};
use crate::notes::terminal_of;
use crate::state::AppState;
use crate::whos_here::Claim;
use crate::write_failure::{node_cannot_write, node_failure};
use cairn_event::medication::ALLERGY_SOURCE;
use cairn_node::medication::draft::{self, DraftLine};
use cairn_node::medication::line::Directions;
use uuid::Uuid;

/// What a write in fixture mode answers (as the note pane's).
const FIXTURE: &str = "fixture mode: this window is showing mock data and cannot write";

/// One line as the webview saves it. `line_text` is the whole line as typed; `typed_drug` and
/// `directions` are the window's split of it (only the window can split it as typed); `parts`
/// the drug parts with their picks; `charted_as` the press (`started-here` | `pre-existing`).
#[derive(Debug, serde::Deserialize)]
pub struct LineArg {
    pub line_id: String,
    pub charted_as: String,
    pub line_text: String,
    pub parts: Vec<PartArg>,
    pub typed_drug: String,
    pub directions: String,
    pub reason: Option<String>,
    pub hand_ticked: bool,
    pub finished: bool,
}

/// The list as the pane draws it. `writer_name`: who it belongs to (`None`: nobody is named, the
/// unnamed list). `elsewhere`: the terminal holding the lines when it is not this one ("Continue
/// here" moves them).
#[derive(Debug, serde::Serialize)]
pub struct MedListView {
    pub writer_name: Option<String>,
    pub elsewhere: Option<String>,
    pub lines: Vec<ListLineView>,
}

/// One line as the pane draws it: what was saved (the [`LineArg`] fields), and what was derived
/// from it on this read. `read` is the directions read into fields (`med_line_read`'s answer);
/// `ticked = hand_ticked || default_ticked`; `unfinished` the line was never finished (the label
/// "unfinished").
#[derive(Debug, serde::Serialize)]
pub struct ListLineView {
    pub line_id: String,
    pub charted_as: String,
    pub line_text: String,
    pub parts: Vec<PartArg>,
    pub typed_drug: String,
    pub directions: String,
    pub reason: Option<String>,
    pub hand_ticked: bool,
    pub finished: bool,
    pub check: LineCheckView,
    pub read: Directions,
    pub default_ticked: bool,
    pub ticked: bool,
    pub duplicate: bool,
    pub unfinished: bool,
}

/// What a save or a delete answers: the door's one word (`saved`, `moved`, `deleted`, …).
#[derive(Debug, serde::Serialize)]
pub struct SaveAnswer {
    pub answer: String,
}

#[tauri::command]
pub async fn med_list_read(
    state: tauri::State<'_, AppState>,
    patient_id: String,
    charts: Vec<String>,
) -> Result<MedListView, ErrorView> {
    med_list_read_impl(&state, &patient_id, Some(&charts)).await
}

#[tauri::command]
pub async fn med_list_save(
    state: tauri::State<'_, AppState>,
    patient_id: String,
    charts: Vec<String>,
    line: LineArg,
) -> Result<SaveAnswer, ErrorView> {
    med_list_save_impl(&state, &patient_id, &charts, line).await
}

#[tauri::command]
pub async fn med_list_delete(
    state: tauri::State<'_, AppState>,
    patient_id: String,
    line_id: String,
) -> Result<SaveAnswer, ErrorView> {
    med_list_delete_impl(&state, &patient_id, &line_id).await
}

#[tauri::command]
pub async fn med_list_take(
    state: tauri::State<'_, AppState>,
    patient_id: String,
) -> Result<MedListView, ErrorView> {
    med_list_take_impl(&state, &patient_id).await
}

/// The list's writer as who's here names them: `(staff id, display name)`, or `(None, None)`.
pub(crate) async fn claim_writer(state: &AppState) -> (Option<String>, Option<String>) {
    match state.whos_here.lock().await.claim() {
        Claim::Named {
            staff_id,
            display_name,
        } => (Some(staff_id.clone()), Some(display_name.clone())),
        Claim::NotAsked | Claim::Unknown => (None, None),
    }
}

/// Read the list for the chart on screen. `charts`: the set the window showed (`None`: take the
/// set read now, as after "Continue here"); a set that changed since leaves every line "not
/// checked" on screen, and Sign refuses it.
pub async fn med_list_read_impl(
    state: &AppState,
    patient_id: &str,
    charts: Option<&[String]>,
) -> Result<MedListView, ErrorView> {
    let patient = state.displayed_patient(patient_id).await.map_err(refused)?;
    let (writer, writer_name) = claim_writer(state).await;
    let fresh = fresh_read(state, patient, writer.as_deref()).await?;
    let same_set = charts.is_none_or(|c| check_displayed_set(&fresh.set, c).is_ok());
    let judged = judge(state, fresh, same_set, writer.is_some()).await;
    let terminal = state.terminal_id.get().map(String::as_str);
    Ok(MedListView {
        writer_name,
        elsewhere: elsewhere(&judged, terminal),
        lines: judged.iter().map(line_view).collect(),
    })
}

/// Save one line (module doc). A save that wrote nothing is an `Ok` with the door's word: the
/// connection worked, and the webview keeps the text either way.
pub async fn med_list_save_impl(
    state: &AppState,
    patient_id: &str,
    charts: &[String],
    line: LineArg,
) -> Result<SaveAnswer, ErrorView> {
    let patient = state.displayed_patient(patient_id).await.map_err(refused)?;
    if state.is_mock() {
        return Err(refused(FIXTURE));
    }
    let terminal = terminal_of(state)?.to_string();
    let line_id: Uuid = line.line_id.parse().map_err(|_| refused("not a line id"))?;
    press(&line.charted_as)?;
    let (writer, _) = claim_writer(state).await;
    // The door keeps the refs of the moment a tick was GIVEN; a later save's are ignored there.
    let ticked_against = match line.hand_ticked {
        true => warnings_now(state, patient, charts, &line).await,
        false => vec![],
    };
    let row = DraftLine {
        line_id,
        terminal_id: terminal,
        seq: 0,
        charted_as: line.charted_as,
        line_text: line.line_text,
        parts: serde_json::to_value(&line.parts).map_err(|e| refused(e.to_string()))?,
        reason: line.reason,
        hand_ticked: line.hand_ticked,
        ticked_against,
        finished: line.finished,
        saved_at: String::new(),
        typed_drug: line.typed_drug,
        directions: line.directions,
    };
    let db = state
        .db
        .as_ref()
        .ok_or_else(|| node_cannot_write("no database connection"))?
        .lock()
        .await?;
    let answer = draft::save_draft_line(&*db, patient, writer.as_deref(), &row)
        .await
        .map_err(|e| node_failure(&db.explain(e)))?;
    Ok(SaveAnswer { answer })
}

/// The warnings on `line` now, as a tick names them: a fresh check over the set on screen. Any
/// failure is no refs, which can only make the tick fail at Sign (it needs every warning seen).
async fn warnings_now(
    state: &AppState,
    patient: Uuid,
    charts: &[String],
    line: &LineArg,
) -> Vec<(String, Uuid)> {
    let Ok((term, parts)) = parts_before_the_key(&line.parts, &line.typed_drug) else {
        return vec![];
    };
    let lines = the_line(&line.typed_drug, &term, &parts);
    match fresh_check(state, patient, charts, &lines).await {
        Ok(check) => check
            .against
            .into_iter()
            .map(|id| (ALLERGY_SOURCE.to_string(), id))
            .collect(),
        Err(_) => vec![],
    }
}

/// Remove one of the writer's lines, whatever it last saved (the clinician pressed remove on it).
pub async fn med_list_delete_impl(
    state: &AppState,
    patient_id: &str,
    line_id: &str,
) -> Result<SaveAnswer, ErrorView> {
    let patient = state.displayed_patient(patient_id).await.map_err(refused)?;
    if state.is_mock() {
        return Err(refused(FIXTURE));
    }
    let terminal = terminal_of(state)?;
    let line: Uuid = line_id.parse().map_err(|_| refused("not a line id"))?;
    let (writer, _) = claim_writer(state).await;
    let db = state
        .db
        .as_ref()
        .ok_or_else(|| node_cannot_write("no database connection"))?
        .lock()
        .await?;
    let lines = draft::read_draft_lines(&*db, patient, writer.as_deref())
        .await
        .map_err(|e| node_failure(&db.explain(e)))?;
    let Some(found) = lines.iter().find(|l| l.line_id == line) else {
        return Ok(SaveAnswer {
            answer: "gone".into(),
        });
    };
    let answer = draft::delete_draft_line(
        &*db,
        line,
        patient,
        terminal,
        writer.as_deref(),
        &found.saved_at,
    )
    .await
    .map_err(|e| node_failure(&db.explain(e)))?;
    Ok(SaveAnswer { answer })
}

/// "Continue here": move the writer's list to this terminal, then read it.
pub async fn med_list_take_impl(
    state: &AppState,
    patient_id: &str,
) -> Result<MedListView, ErrorView> {
    let patient = state.displayed_patient(patient_id).await.map_err(refused)?;
    if state.is_mock() {
        return Err(refused(FIXTURE));
    }
    let terminal = terminal_of(state)?;
    let (writer, _) = claim_writer(state).await;
    {
        let db = state
            .db
            .as_ref()
            .ok_or_else(|| node_cannot_write("no database connection"))?
            .lock()
            .await?;
        draft::take_draft_lines(&*db, patient, writer.as_deref(), terminal)
            .await
            .map_err(|e| node_failure(&db.explain(e)))?;
    }
    med_list_read_impl(state, patient_id, None).await
}

/// The terminal holding the list when it is not this one (`terminal`; `None` while the id is not
/// yet read: then no line is this terminal's). Pure.
pub(crate) fn elsewhere(judged: &[Judged], terminal: Option<&str>) -> Option<String> {
    judged
        .iter()
        .map(|j| &j.line.terminal_id)
        .find(|t| Some(t.as_str()) != terminal)
        .cloned()
}

/// One judged line as the pane draws it. Pure.
pub(crate) fn line_view(j: &Judged) -> ListLineView {
    let l = &j.line;
    ListLineView {
        line_id: l.line_id.to_string(),
        charted_as: l.charted_as.clone(),
        line_text: l.line_text.clone(),
        parts: j.parts.clone(),
        typed_drug: l.typed_drug.clone(),
        directions: l.directions.clone(),
        reason: l.reason.clone(),
        hand_ticked: l.hand_ticked,
        finished: l.finished,
        check: view(j.check.clone()),
        read: j.read.clone(),
        default_ticked: j.default_ticked,
        ticked: l.hand_ticked || j.default_ticked,
        duplicate: j.duplicate,
        unfinished: !l.finished,
    }
}

#[cfg(test)]
#[path = "med_list_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "med_list_db_tests.rs"]
mod db_tests;
