//! The front door's seven commands.
//!
//! Each `#[tauri::command]` is a one-line forwarder onto a plain `*_impl(&AppState, …)`
//! function, so the whole walk — browse, pick, register, prompt, retry — runs under `cargo
//! test` against `AppState::mock` with no Tauri runtime. The same two rules `commands.rs` states
//! hold here: the underlying error text reaches the screen (through `view`'s sentences), and
//! partial completion is reported, never implied (`incomplete_reason` travels with every list).
//!
//! # The walk, as the webview drives it
//!
//! ```text
//! browse(form) ------------------------------> list (scrolls; attests nothing)
//!   └ open_chart(id of a listed candidate) --> chart
//! form_edited(revision)                        (every keystroke in the register form)
//! prompt_search(form, force=false) ----------> waiting sentence, or bounded prompt + token
//!   └ open_chart(id of a prompt candidate) --> chart           ("this is them")
//!   └ register(token) -----------------------> new chart        ("none of these")
//! register clicked with no token yet --------> prompt_search(form, force=true) first
//! ```
use crate::funnel::rows::{person_row_view, PersonRowView};
use crate::funnel::view::{
    browse_summary, header_from_candidate, header_from_registration, prompt_summary,
    register_behind_open_chart_view, register_error_view, registered_behind_open_chart_view,
    search_error_view, token_error_view, waiting_sentence, ChartHeaderView, ErrorView,
};
use crate::funnel::window::OpenChart;
use crate::state::AppState;
use cairn_gui_data::port::DataError;
use cairn_gui_funnel::{
    bound_for_prompt, node_reason, trigger_state, FormSnapshot, Recorded, Restored, SearchToken,
};
use cairn_patient_search::CandidateList;
use serde::Serialize;

/// What the front door needs at launch, and after any reload.
#[derive(Debug, Serialize)]
pub struct FunnelStatus {
    pub mock: bool,
    /// The launch probe's sentence when this node may not write (#654 option 2).
    pub provisioning: Option<String>,
    /// The chart already open (`--patient`), if any.
    pub chart: Option<ChartHeaderView>,
    /// The highest register-form revision the backend has seen — a reloaded webview resumes
    /// from it (see `FunnelSession::revision`).
    pub revision: u64,
}

/// One browse answer. `revision` echoes the request's, so the webview drops a late answer.
#[derive(Debug, Serialize)]
pub struct BrowseView {
    pub revision: u64,
    /// One entry per PERSON (R3): linked charts share a row, each member its own open target.
    pub people: Vec<PersonRowView>,
    /// The announcement (`view::browse_summary`) — from Rust, so "nobody matched" and "the
    /// search did not finish" cannot be confused by a sentence no test pins.
    pub summary: String,
    pub incomplete_reason: Option<String>,
}

/// A browse answer as the webview renders it — pure, so the incompleteness rule is testable
/// without a backend.
///
/// The reason goes through `node_reason`, exactly as the prompt's does: a node that set the
/// `incomplete` flag with no prose must still reach the clerk as partial, never as whole.
pub fn browse_view(revision: u64, list: &CandidateList) -> BrowseView {
    let incomplete_reason = node_reason(list).map(str::to_string);
    BrowseView {
        revision,
        people: list.people.iter().map(person_row_view).collect(),
        summary: browse_summary(
            list.people.len(),
            list.charts().count(),
            incomplete_reason.is_some(),
        ),
        incomplete_reason,
    }
}

/// One step-3 answer.
#[derive(Debug, Serialize)]
pub struct PromptView {
    pub revision: u64,
    /// Set instead of a search when the (advisory) trigger is still waiting, or when the form
    /// has nothing to search on at all.
    pub waiting: Option<String>,
    /// The form moved on while this search ran; nothing was recorded. The webview ignores it.
    pub stale: bool,
    /// What `register` must be handed to attest THIS prompt. `None` means Register may not use it.
    pub token: Option<SearchToken>,
    /// Exactly the bounded list a registration would attest — what is shown IS what is signed.
    pub people: Vec<PersonRowView>,
    /// The sentence announcing this prompt, present whenever `token` is (`view::prompt_summary`).
    pub summary: Option<String>,
    pub incomplete_reason: Option<String>,
}

impl PromptView {
    /// A prompt carrying only a sentence: no search result, nothing to register with.
    fn saying(revision: u64, sentence: String) -> Self {
        PromptView {
            revision,
            waiting: Some(sentence),
            stale: false,
            token: None,
            people: vec![],
            summary: None,
            incomplete_reason: None,
        }
    }
}

/// Remember every chart that is now on screen, so `open_chart` will open them.
async fn remember_shown(state: &AppState, list: &CandidateList) {
    let mut shown = state.shown.lock().await;
    for c in list.charts() {
        shown.insert(c.patient_id, c.clone());
    }
}

/// Open a chart: set it, forget the lists that led here.
///
/// Known imprecision: `chart` and `shown` are two locks, so a browse still in flight can
/// re-populate `shown` just after the clear. Bounded harm — opening a chart attests nothing,
/// and `open_chart` still only opens a row some list really returned.
async fn open(state: &AppState, chart: OpenChart) -> ChartHeaderView {
    let header = chart.header.clone();
    *state.chart.lock().await = Some(chart);
    state.shown.lock().await.clear();
    header
}

/// Open a chart that was just REGISTERED — unless the clerk opened another one while it saved.
///
/// Check-and-set under one hold of the chart lock, so nothing can open a chart between the
/// check and the set. If a chart is already open, the clerk's last act was choosing it ("This
/// is them" clicked during "Saving…"), and silently replacing it with the new chart — same typed
/// name, easy to miss — would put them on a probable duplicate. So the window stays put and the
/// refusal names the new chart (PR #674 review).
async fn open_after_registering(
    state: &AppState,
    patient: uuid::Uuid,
    header: ChartHeaderView,
) -> Result<ChartHeaderView, ErrorView> {
    let mut chart = state.chart.lock().await;
    if let Some(open) = chart.as_ref() {
        return Err(registered_behind_open_chart_view(&header, &open.header));
    }
    *chart = Some(OpenChart {
        patient,
        header: header.clone(),
    });
    drop(chart);
    state.shown.lock().await.clear();
    Ok(header)
}

pub async fn funnel_status_impl(state: &AppState) -> FunnelStatus {
    FunnelStatus {
        mock: state.is_mock(),
        provisioning: state.provisioning.clone(),
        chart: state.chart.lock().await.as_ref().map(|c| c.header.clone()),
        revision: state.funnel.lock().await.revision(),
    }
}

pub async fn form_edited_impl(state: &AppState, revision: u64) {
    state.funnel.lock().await.edited(revision);
}

/// Step 1: search on a fragment. The list SCROLLS — it carries no signed claim, so it is not
/// bounded; and a failure is an error, never an empty list.
pub async fn browse_impl(state: &AppState, form: FormSnapshot) -> Result<BrowseView, ErrorView> {
    let list = state
        .funnel_backend
        .search(&form.query())
        .await
        .map_err(|e| search_error_view(&e))?;
    remember_shown(state, &list).await;
    Ok(browse_view(form.revision, &list))
}

/// Step 3: the registration search, over what the register form holds.
///
/// Without `force`, it runs only once the advisory trigger is satisfied; with it (Register
/// clicked before then — a mononymous patient, an unknown date of birth) it runs on whatever is
/// typed, because a registration without its due-diligence search is what ADR-0061 forbids.
pub async fn prompt_search_impl(
    state: &AppState,
    form: FormSnapshot,
    force: bool,
) -> Result<PromptView, ErrorView> {
    if !force {
        if let Some(sentence) = waiting_sentence(&trigger_state(&form.raw_name, &form.birth_date)) {
            return Ok(PromptView::saying(form.revision, sentence));
        }
    }
    let list = state
        .funnel_backend
        .search(&form.query())
        .await
        .map_err(|e| search_error_view(&e))?;
    let prompt = bound_for_prompt(&list);
    // Rendered from the BOUNDED list, before `record` takes it: the rows on screen are exactly
    // the rows a registration will swear were displayed. The counts are read now for the same
    // reason — `record` moves the prompt — and their `withheld` is shown, never signed
    // (ADR-0075).
    let bounded = prompt.as_list().clone();
    let counts = prompt.counts();
    let recorded = state.funnel.lock().await.record(&form, prompt);
    match recorded {
        Ok(Recorded::Current(token)) => {
            remember_shown(state, &bounded).await;
            Ok(PromptView {
                revision: form.revision,
                waiting: None,
                stale: false,
                token: Some(token),
                summary: Some(prompt_summary(&counts)),
                people: bounded.people.iter().map(person_row_view).collect(),
                incomplete_reason: bounded.incomplete_reason,
            })
        }
        Ok(Recorded::Stale) => Ok(PromptView {
            stale: true,
            waiting: None,
            ..PromptView::saying(form.revision, String::new())
        }),
        Err(refusal) => Ok(PromptView::saying(
            form.revision,
            token_error_view(refusal).text,
        )),
    }
}

/// "None of these — register a new patient."
///
/// Order is load-bearing:
///
/// 1. **No chart may be open.** Register is a front-door act; if a chart is open, the clerk
///    recognised an existing patient ("This is them") while this was on its way, and nothing
///    is written.
/// 2. **The provisioning check** runs BEFORE the attestation leaves the store, so an
///    unprovisioned node refuses with the search intact.
/// 3. `take` and `settle` each hold the session lock briefly and never across the write, so a
///    background prompt search can still land mid-write (the case `TokenStore::commit`'s
///    invalidate exists for).
/// 4. The new chart opens only if the front door is still showing (`open_after_registering`).
///
/// ⚠️ Cancellation-unsafe (#649, #669): nothing may race this against a timeout or `select!`.
pub async fn register_impl(
    state: &AppState,
    token: SearchToken,
) -> Result<ChartHeaderView, ErrorView> {
    if let Some(open) = state.chart.lock().await.as_ref() {
        return Err(register_behind_open_chart_view(&open.header));
    }
    state
        .funnel_backend
        .require_provisioned()
        .await
        .map_err(|e| register_error_view(&e, Restored::Kept))?;
    let taken = state
        .funnel
        .lock()
        .await
        .take_for_register(token)
        .map_err(token_error_view)?;
    let name = taken.raw_name().to_string();
    let birth_date = taken.query().birth_date.clone();
    let outcome = state.funnel_backend.register(taken).await;
    let settled = {
        let mut funnel = state.funnel.lock().await;
        match outcome {
            // #815: the chart may exist, so the search is spent as a success spends it — never
            // restored, or the next press registers the same person a second time.
            Err((error @ DataError::OutcomeUnknown(_), attested)) => {
                Err((error, funnel.settle_unknown(attested)))
            }
            other => funnel.settle(other),
        }
    };
    match settled {
        Ok(patient) => {
            let header = header_from_registration(patient, &name, birth_date.as_deref());
            open_after_registering(state, patient, header).await
        }
        Err((error, restored)) => Err(register_error_view(&error, restored)),
    }
}

/// Open a chart the clerk picked — only one some list on screen actually showed.
pub async fn open_chart_impl(
    state: &AppState,
    patient_id: &str,
) -> Result<ChartHeaderView, String> {
    let refusal = || "that chart was not in a list on screen — search again".to_string();
    // Only a webview bug sends an unparseable id; say THAT, so whoever debugs it is not sent
    // looking for a list problem.
    let patient: uuid::Uuid = patient_id.parse().map_err(|_| {
        format!("the window could not read the chart id {patient_id:?} — search again")
    })?;
    let candidate = state
        .shown
        .lock()
        .await
        .get(&patient)
        .cloned()
        .ok_or_else(refusal)?;
    Ok(open(
        state,
        OpenChart {
            patient,
            header: header_from_candidate(&candidate),
        },
    )
    .await)
}

/// Back to the front door. Discards any held search: the form it described is being reset.
pub async fn close_chart_impl(state: &AppState) {
    *state.chart.lock().await = None;
    state.shown.lock().await.clear();
    state.funnel.lock().await.discard();
}

// ---- The Tauri forwarders. Tauri maps `funnel.js`'s camelCase keys (`patientId`) onto these
// snake_case parameters (`patient_id`). ----

#[tauri::command]
pub async fn funnel_status(state: tauri::State<'_, AppState>) -> Result<FunnelStatus, ()> {
    Ok(funnel_status_impl(&state).await)
}

#[tauri::command]
pub async fn form_edited(state: tauri::State<'_, AppState>, revision: u64) -> Result<(), ()> {
    form_edited_impl(&state, revision).await;
    Ok(())
}

#[tauri::command]
pub async fn browse(
    state: tauri::State<'_, AppState>,
    form: FormSnapshot,
) -> Result<BrowseView, ErrorView> {
    browse_impl(&state, form).await
}

#[tauri::command]
pub async fn prompt_search(
    state: tauri::State<'_, AppState>,
    form: FormSnapshot,
    force: bool,
) -> Result<PromptView, ErrorView> {
    prompt_search_impl(&state, form, force).await
}

#[tauri::command]
pub async fn register(
    state: tauri::State<'_, AppState>,
    token: SearchToken,
) -> Result<ChartHeaderView, ErrorView> {
    register_impl(&state, token).await
}

#[tauri::command]
pub async fn open_chart(
    state: tauri::State<'_, AppState>,
    patient_id: String,
) -> Result<ChartHeaderView, String> {
    open_chart_impl(&state, &patient_id).await
}

#[tauri::command]
pub async fn close_chart(state: tauri::State<'_, AppState>) -> Result<(), ()> {
    close_chart_impl(&state).await;
    Ok(())
}

#[cfg(test)]
#[path = "commands_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "commands_register_tests.rs"]
mod register_tests;
