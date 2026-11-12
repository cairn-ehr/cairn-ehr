//! The medication list (batch charting slice 1): the window's commands over a chart's durable,
//! per-writer list of UNSIGNED medication lines (db/078). Sign is `med_list_sign.rs`.
//!
//! - [`med_list_read`]: the writer's lines, each re-checked against the allergies read NOW and
//!   given its default tick (design §3.2). The writer is who's here's claim: a list is shown only
//!   to the person it was typed by (a NULL-writer list only while nobody is named).
//! - [`med_list_save`]: save one line as typed (the window autosaves). The door (db/078) decides
//!   what a hand tick keeps; this sends it the warnings a tick vouches for: those found by a fresh
//!   check NOW that were also drawn on the line (`seen`, ruling R1). The webview's word can only
//!   narrow them, never add one.
//! - [`med_list_delete`]: remove one line, leaving no record (design §9 Q3).
//! - [`med_list_take`]: "Continue here", moving the list from another terminal to this one.
//!
//! Save and delete are written under the writer the list was READ under (`MedListView::writer`,
//! sent back by the webview, as `note_draft_save` takes its writer, ruling R2): a debounced save
//! landing after a who's-here switch still names the person who typed it. The door still refuses
//! a writer who is not a clinical vault member, and Sign checks the writer by key.
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
use crate::med_list_delabel::DelabelOffer;
pub(crate) use crate::med_list_judge::{
    already_on_the_list, fresh_read, judge, tick_holds, Judged,
};
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
#[derive(Debug, Clone, serde::Deserialize)]
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
    /// The warnings the line was DRAWN with (its view's `against`), sent back with a hand tick: a
    /// tick vouches only for warnings both drawn and found by the fresh check at the save (ruling
    /// R1). Missing or empty: the tick vouches for none, and any warning fails it at Sign.
    #[serde(default)]
    pub seen: Vec<WarningRef>,
    /// The clinician unticked a line the default tick would give: their veto, stored (Task 6
    /// review C1). Missing: false. The door clears it when the words change or a hand tick is
    /// given.
    #[serde(default)]
    pub held: bool,
}

/// One warning a line carries, as the wire's override names it: `{"source": "allergy", "ref":
/// "<allergy record event id>"}`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct WarningRef {
    pub source: String,
    #[serde(rename = "ref")]
    pub reference: String,
}

/// The list as the pane draws it. `writer_name`: who it belongs to (`None`: nobody is named, the
/// unnamed list). `elsewhere`: the terminal holding the lines when it is not this one ("Continue
/// here" moves them).
#[derive(Debug, serde::Serialize)]
pub struct MedListView {
    /// The writer's staff id, sent back with every save and delete (the list is written under
    /// the writer it was READ under, never whoever is named when a late save lands).
    pub writer: Option<String>,
    pub writer_name: Option<String>,
    pub elsewhere: Option<String>,
    pub lines: Vec<ListLineView>,
}

/// One line as the pane draws it: what was saved (the [`LineArg`] fields), and what was derived
/// from it on this read. `read` is the directions read into fields (`med_line_read`'s answer);
/// `ticked = (hand_ticked && the hand tick still holds) || (default_ticked && !held)`;
/// `unfinished` the line was never finished (the label "unfinished").
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
    /// Rust's sentence naming the line this one duplicates (`already on the list: {line}`), or
    /// `None` (review I2).
    pub duplicate_of: Option<String>,
    /// The stored veto ([`LineArg::held`]).
    pub held: bool,
    pub unfinished: bool,
    /// The warnings drawn on the line now (its check's allergy marks), for [`LineArg::seen`].
    pub against: Vec<WarningRef>,
    /// The same-drug de-label offers this line may show (empty when none).
    pub delabel_offers: Vec<DelabelOffer>,
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
    writer: Option<String>,
) -> Result<SaveAnswer, ErrorView> {
    med_list_save_impl(&state, &patient_id, &charts, line, writer.as_deref()).await
}

#[tauri::command]
pub async fn med_list_delete(
    state: tauri::State<'_, AppState>,
    patient_id: String,
    line_id: String,
    writer: Option<String>,
) -> Result<SaveAnswer, ErrorView> {
    med_list_delete_impl(&state, &patient_id, &line_id, writer.as_deref()).await
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
        writer,
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
    writer: Option<&str>,
) -> Result<SaveAnswer, ErrorView> {
    let patient = state.displayed_patient(patient_id).await.map_err(refused)?;
    if state.is_mock() {
        return Err(refused(FIXTURE));
    }
    let terminal = terminal_of(state)?.to_string();
    let line_id: Uuid = line.line_id.parse().map_err(|_| refused("not a line id"))?;
    press(&line.charted_as)?;
    // The door keeps the refs of the moment a tick was GIVEN; a later save's are ignored there.
    let ticked_against = match line.hand_ticked {
        true => vouched(
            &warnings_now(state, patient, charts, &line).await,
            &line.seen,
        ),
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
        held: line.held,
    };
    let db = state
        .db
        .as_ref()
        .ok_or_else(|| node_cannot_write("no database connection"))?
        .lock()
        .await?;
    let answer = draft::save_draft_line(&*db, patient, writer, &row)
        .await
        .map_err(|e| node_failure(&db.explain(e)))?;
    Ok(SaveAnswer { answer })
}

/// The warnings on `line` now: a fresh check over the set on screen. Any failure is none, which
/// can only make the tick fail at Sign (it needs every warning seen).
async fn warnings_now(
    state: &AppState,
    patient: Uuid,
    charts: &[String],
    line: &LineArg,
) -> Vec<Uuid> {
    let Ok((term, parts)) = parts_before_the_key(&line.parts, &line.typed_drug) else {
        return vec![];
    };
    let lines = the_line(&line.typed_drug, &term, &parts);
    fresh_check(state, patient, charts, &lines)
        .await
        .map(|check| check.against)
        .unwrap_or_default()
}

/// What a hand tick vouches for (ruling R1): the warnings BOTH found now and drawn on the line
/// when it was ticked (`seen`). One found but never drawn was never seen, so it is left out, and
/// the tick fails at Sign; one drawn but gone now needs no vouching. Pure.
pub(crate) fn vouched(now: &[Uuid], seen: &[WarningRef]) -> Vec<(String, Uuid)> {
    now.iter()
        .filter(|id| {
            seen.iter()
                .any(|w| w.source == ALLERGY_SOURCE && w.reference.parse() == Ok(**id))
        })
        .map(|id| (ALLERGY_SOURCE.to_string(), *id))
        .collect()
}

/// Remove one of the writer's lines, whatever it last saved (the clinician pressed remove on it).
pub async fn med_list_delete_impl(
    state: &AppState,
    patient_id: &str,
    line_id: &str,
    writer: Option<&str>,
) -> Result<SaveAnswer, ErrorView> {
    let patient = state.displayed_patient(patient_id).await.map_err(refused)?;
    if state.is_mock() {
        return Err(refused(FIXTURE));
    }
    let terminal = terminal_of(state)?;
    let line: Uuid = line_id.parse().map_err(|_| refused("not a line id"))?;
    let db = state
        .db
        .as_ref()
        .ok_or_else(|| node_cannot_write("no database connection"))?
        .lock()
        .await?;
    let lines = draft::read_draft_lines(&*db, patient, writer)
        .await
        .map_err(|e| node_failure(&db.explain(e)))?;
    let Some(found) = lines.iter().find(|l| l.line_id == line) else {
        return Ok(SaveAnswer {
            answer: "gone".into(),
        });
    };
    let answer = draft::delete_draft_line(&*db, line, patient, terminal, writer, &found.saved_at)
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
        ticked: (l.hand_ticked && tick_holds(j)) || (j.default_ticked && !l.held),
        duplicate: j.duplicate,
        duplicate_of: j.duplicate_of.as_deref().map(already_on_the_list),
        held: l.held,
        unfinished: !l.finished,
        against: j
            .check
            .against
            .iter()
            .map(|id| WarningRef {
                source: ALLERGY_SOURCE.into(),
                reference: id.to_string(),
            })
            .collect(),
        delabel_offers: j.offers.clone(),
    }
}

#[cfg(test)]
#[path = "med_list_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "med_list_db_tests.rs"]
mod db_tests;

#[cfg(test)]
#[path = "med_list_review_db_tests.rs"]
mod review_db_tests;

#[cfg(test)]
#[path = "med_list_held_db_tests.rs"]
mod held_db_tests;

#[cfg(test)]
#[path = "med_list_delabel_db_tests.rs"]
mod delabel_db_tests;
