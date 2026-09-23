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
use crate::funnel::view::{
    candidate_view, header_from_candidate, header_from_registration, prompt_summary,
    register_error_view, search_error_view, token_error_view, waiting_sentence, CandidateView,
    ChartHeaderView, ErrorView,
};
use crate::funnel::window::OpenChart;
use crate::state::AppState;
use cairn_gui_funnel::{
    bound_for_prompt, trigger_state, FormSnapshot, Recorded, Restored, SearchToken,
};
use cairn_patient_search::Candidate;
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
    /// above it (see `FunnelSession::revision`).
    pub revision: u64,
}

/// One browse answer. `revision` echoes the request's, so the webview drops a late answer.
#[derive(Debug, Serialize)]
pub struct BrowseView {
    pub revision: u64,
    pub candidates: Vec<CandidateView>,
    pub incomplete_reason: Option<String>,
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
    pub candidates: Vec<CandidateView>,
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
            candidates: vec![],
            summary: None,
            incomplete_reason: None,
        }
    }
}

/// Remember candidates that are now on screen, so `open_chart` will open them.
async fn remember_shown(state: &AppState, candidates: &[Candidate]) {
    let mut shown = state.shown.lock().await;
    for c in candidates {
        shown.insert(c.patient_id, c.clone());
    }
}

/// Open a chart: set it, forget the lists that led here.
async fn open(state: &AppState, chart: OpenChart) -> ChartHeaderView {
    let header = chart.header.clone();
    *state.chart.lock().await = Some(chart);
    state.shown.lock().await.clear();
    header
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
    remember_shown(state, &list.candidates).await;
    Ok(BrowseView {
        revision: form.revision,
        candidates: list.candidates.iter().map(candidate_view).collect(),
        incomplete_reason: list.incomplete_reason.clone(),
    })
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
    // the rows a registration will swear were displayed.
    let bounded = prompt.as_list().clone();
    let recorded = state.funnel.lock().await.record(&form, prompt);
    match recorded {
        Ok(Recorded::Current(token)) => {
            remember_shown(state, &bounded.candidates).await;
            Ok(PromptView {
                revision: form.revision,
                waiting: None,
                stale: false,
                token: Some(token),
                summary: Some(prompt_summary(bounded.candidates.len())),
                candidates: bounded.candidates.iter().map(candidate_view).collect(),
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
/// Order is load-bearing: the provisioning check runs BEFORE the attestation leaves the store,
/// so an unprovisioned node refuses with the search intact; `take` and `settle` each hold the
/// session lock briefly and never across the write, so a background prompt search can still
/// land mid-write (the case `TokenStore::commit`'s invalidate exists for).
///
/// ⚠️ Cancellation-unsafe (#649, #669): nothing may race this against a timeout or `select!`.
pub async fn register_impl(
    state: &AppState,
    token: SearchToken,
) -> Result<ChartHeaderView, ErrorView> {
    state
        .funnel_backend
        .require_provisioned()
        .await
        .map_err(|e| register_error_view(&e, Restored::Kept))?;
    let (attested, name) = state
        .funnel
        .lock()
        .await
        .take_for_register(token)
        .map_err(token_error_view)?;
    let birth_date = attested.query().birth_date.clone();
    let outcome = state.funnel_backend.register(attested, &name).await;
    let settled = state.funnel.lock().await.settle(outcome);
    match settled {
        Ok(patient) => {
            let header = header_from_registration(patient, &name, birth_date.as_deref());
            Ok(open(state, OpenChart { patient, header }).await)
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
    let patient: uuid::Uuid = patient_id.parse().map_err(|_| refusal())?;
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
    // Revision 0 never raises the floor; it only discards.
    state.funnel.lock().await.edited(0);
}

// ---- The Tauri forwarders. Argument names are what `funnel.js` sends (camelCase). ----

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
mod tests {
    use super::*;
    use crate::funnel::view::Retry;
    use cairn_gui_data::port::DataError;

    use crate::commands::tests::fields_read_in;
    use crate::funnel::view::tests::sample_candidate;
    use crate::funnel::view::{candidate_view, header_opened_by_id};
    use std::collections::BTreeSet;

    /// THE FRONT DOOR'S DRIFT GUARD — the same guard `commands.rs` keeps for `main.js`.
    ///
    /// `funnel.js` is untyped, so a Rust field rename does not break the build: it renders
    /// `undefined`. On this screen that is not cosmetic. `incomplete_reason` undefined is a
    /// partial list shown as whole; `retry` undefined is a Register button left live on a
    /// verdict. So every field the JS reads must exist in the payload the Rust sends.
    #[test]
    fn funnel_js_reads_no_field_the_backend_does_not_send() {
        let js = include_str!("../../src-ui/funnel.js");
        let header = header_opened_by_id(uuid::Uuid::nil());
        let cand = candidate_view(&sample_candidate());
        let payloads: [(&str, serde_json::Value); 6] = [
            (
                "status",
                serde_json::to_value(FunnelStatus {
                    mock: true,
                    provisioning: None,
                    chart: Some(header.clone()),
                    revision: 0,
                })
                .unwrap(),
            ),
            ("header", serde_json::to_value(&header).unwrap()),
            ("cand", serde_json::to_value(&cand).unwrap()),
            (
                "browseView",
                serde_json::to_value(BrowseView {
                    revision: 0,
                    candidates: vec![],
                    incomplete_reason: None,
                })
                .unwrap(),
            ),
            (
                "prompt",
                serde_json::to_value(PromptView::saying(0, String::new())).unwrap(),
            ),
            (
                "failure",
                serde_json::to_value(ErrorView {
                    text: String::new(),
                    retry: Retry::Now,
                })
                .unwrap(),
            ),
        ];
        for (binding, value) in payloads {
            let available: BTreeSet<String> = value.as_object().unwrap().keys().cloned().collect();
            let read = fields_read_in(js, binding);
            assert!(!read.is_empty(), "funnel.js no longer reads `{binding}` at all — rename the binding in this guard, don't delete it");
            for field in read {
                assert!(
                    available.contains(&field),
                    "funnel.js reads `{binding}.{field}`, which the backend does not send. \
                     Available: {available:?}"
                );
            }
        }
    }

    /// The other direction, for the fields whose SILENCE is the dangerous failure.
    #[test]
    fn funnel_js_reads_both_incompleteness_reports_and_the_retry_advice() {
        let js = include_str!("../../src-ui/funnel.js");
        assert!(fields_read_in(js, "browseView").contains("incomplete_reason"));
        assert!(fields_read_in(js, "prompt").contains("incomplete_reason"));
        assert!(fields_read_in(js, "prompt").contains("stale"));
        assert!(fields_read_in(js, "failure").contains("retry"));
        assert!(fields_read_in(js, "status").contains("provisioning"));
    }

    fn f(rev: u64, name: &str, dob: &str) -> FormSnapshot {
        FormSnapshot {
            revision: rev,
            raw_name: name.into(),
            birth_date: dob.into(),
        }
    }

    #[tokio::test]
    async fn browse_then_open_a_displayed_candidate_opens_that_chart() {
        let state = AppState::mock(None);
        let list = browse_impl(&state, f(1, "mich", "")).await.unwrap();
        let id = list.candidates[0].patient_id.clone();
        let header = open_chart_impl(&state, &id).await.unwrap();
        assert_eq!(header.patient_id, id);
        assert_eq!(state.open_patient().await.unwrap().to_string(), id);
    }

    /// Review Focus 3: only a candidate from a list the backend itself returned may be opened.
    #[tokio::test]
    async fn only_a_displayed_candidate_can_be_opened() {
        let state = AppState::mock(None);
        let never_shown = uuid::Uuid::from_u128(424_242).to_string();
        assert!(open_chart_impl(&state, &never_shown).await.is_err());
        assert!(state.open_patient().await.is_err());
    }

    /// A prompt candidate ("this is them") stays openable after a later browse.
    #[tokio::test]
    async fn a_prompt_candidate_survives_a_later_browse() {
        let state = AppState::mock(None);
        let p = prompt_search_impl(&state, f(1, "Samantha Michaelowski", "1975"), true)
            .await
            .unwrap();
        let id = p.candidates[0].patient_id.clone();
        browse_impl(&state, f(1, "zzzz-nobody", "")).await.unwrap();
        open_chart_impl(&state, &id).await.expect("still on screen");
    }

    #[tokio::test]
    async fn the_whole_register_walk_opens_the_new_chart_and_the_next_browse_finds_it() {
        let state = AppState::mock(None);
        let p = prompt_search_impl(&state, f(1, "Zebedee Quixote", "1990-05-05"), false)
            .await
            .unwrap();
        assert!(p.waiting.is_none(), "{p:?}");
        let token = p.token.expect("a searchable form mints a token");
        let header = register_impl(&state, token).await.unwrap();
        assert_eq!(header.name, "Zebedee Quixote");
        assert_eq!(header.born, "born 1990-05-05");
        assert_eq!(
            state.open_patient().await.unwrap().to_string(),
            header.patient_id
        );
        close_chart_impl(&state).await;
        let found = browse_impl(&state, f(2, "quixote", "")).await.unwrap();
        assert!(found
            .candidates
            .iter()
            .any(|c| c.patient_id == header.patient_id));
    }

    /// Review Focus 2: a mononymous patient never trips the trigger, and must still be
    /// registrable — the forced search runs on whatever is typed.
    #[tokio::test]
    async fn a_forced_search_runs_even_when_the_trigger_is_waiting() {
        let state = AppState::mock(None);
        let waiting = prompt_search_impl(&state, f(1, "Cher", ""), false)
            .await
            .unwrap();
        assert!(waiting.waiting.is_some() && waiting.token.is_none());
        let forced = prompt_search_impl(&state, f(1, "Cher", ""), true)
            .await
            .unwrap();
        let token = forced.token.expect("Register must still be reachable");
        register_impl(&state, token).await.unwrap();
    }

    /// An empty form is refused with the token store's own sentence, not searched.
    #[tokio::test]
    async fn an_empty_form_mints_no_token_and_says_why() {
        let state = AppState::mock(None);
        let p = prompt_search_impl(&state, f(1, " ", ""), true)
            .await
            .unwrap();
        assert!(p.token.is_none());
        assert!(p.waiting.unwrap().contains("nothing to search on"));
    }

    #[tokio::test]
    async fn a_stale_prompt_search_is_reported_stale_and_mints_nothing() {
        let state = AppState::mock(None);
        form_edited_impl(&state, 5).await;
        let p = prompt_search_impl(&state, f(4, "Jon Smith", "1980"), true)
            .await
            .unwrap();
        assert!(p.stale && p.token.is_none(), "{p:?}");
    }

    #[tokio::test]
    async fn a_second_register_with_the_same_token_is_refused() {
        let state = AppState::mock(None);
        let t = prompt_search_impl(&state, f(1, "Ada Byron", "1815-12-10"), false)
            .await
            .unwrap()
            .token
            .unwrap();
        register_impl(&state, t).await.unwrap();
        assert!(
            register_impl(&state, t).await.is_err(),
            "one search, one chart"
        );
    }

    #[tokio::test]
    async fn an_edit_after_the_prompt_makes_its_token_unredeemable() {
        let state = AppState::mock(None);
        let t = prompt_search_impl(&state, f(1, "Jon Smith", "1980"), false)
            .await
            .unwrap()
            .token
            .unwrap();
        form_edited_impl(&state, 2).await;
        assert!(register_impl(&state, t).await.is_err());
    }

    #[tokio::test]
    async fn a_failed_registration_keeps_the_search_and_the_retry_works() {
        let state = AppState::mock(None);
        let t = prompt_search_impl(&state, f(1, "Ada Byron", "1815"), false)
            .await
            .unwrap()
            .token
            .unwrap();
        state
            .mock_data()
            .unwrap()
            .fail_next(DataError::Unavailable("disk full".into()));
        let err = register_impl(&state, t).await.unwrap_err();
        assert_eq!(err.retry, Retry::Now);
        assert!(state.open_patient().await.is_err(), "nothing was opened");
        register_impl(&state, t)
            .await
            .expect("the Kept search registers on retry");
    }

    #[tokio::test]
    async fn a_refused_registration_withholds_the_retry() {
        let state = AppState::mock(None);
        let t = prompt_search_impl(&state, f(1, "Ada Byron", "1815"), false)
            .await
            .unwrap()
            .token
            .unwrap();
        state
            .mock_data()
            .unwrap()
            .fail_next(DataError::Refused("bad date".into()));
        let err = register_impl(&state, t).await.unwrap_err();
        assert_eq!(err.retry, Retry::Never);
        assert!(err.text.contains("bad date"));
    }

    /// A failed browse must never reach the clerk as an empty list.
    #[tokio::test]
    async fn a_failed_browse_is_an_error_not_an_empty_list() {
        let state = AppState::mock(None);
        state
            .mock_data()
            .unwrap()
            .fail_next(DataError::Unavailable("gone".into()));
        let err = browse_impl(&state, f(1, "mich", "")).await.unwrap_err();
        assert!(err.text.contains("NOT"), "{}", err.text);
    }

    /// Review Focus 5.
    #[tokio::test]
    async fn chart_commands_refuse_when_no_chart_is_open() {
        let state = AppState::mock(None);
        let err = state.open_patient().await.unwrap_err();
        assert!(err.contains("no chart is open"), "{err}");
    }

    #[tokio::test]
    async fn a_patient_given_at_launch_opens_straight_on_the_chart() {
        let id = uuid::Uuid::from_u128(7);
        let state = AppState::mock(Some(id));
        assert_eq!(state.open_patient().await.unwrap(), id);
        let status = funnel_status_impl(&state).await;
        assert!(status.mock);
        assert_eq!(status.chart.unwrap().patient_id, id.to_string());
    }

    /// Final review #3: a reloaded webview restarts its revision counter at 0 while the backend's
    /// floor survives, so every search would come back stale and Register would go dead with no
    /// word. The status reports the floor so the webview can resume above it.
    #[tokio::test]
    async fn the_status_reports_the_revision_floor_so_a_reload_can_resume() {
        let state = AppState::mock(None);
        form_edited_impl(&state, 7).await;
        let status = funnel_status_impl(&state).await;
        assert_eq!(status.revision, 7);
        let p = prompt_search_impl(&state, f(status.revision + 1, "Ada Byron", "1815"), false)
            .await
            .unwrap();
        assert!(!p.stale && p.token.is_some(), "{p:?}");
    }

    /// Final review #6, the command half: a minted prompt carries its announcement.
    #[tokio::test]
    async fn a_minted_prompt_carries_its_summary() {
        let state = AppState::mock(None);
        let p = prompt_search_impl(&state, f(1, "Samantha Michaelowski", "1975"), false)
            .await
            .unwrap();
        assert!(p.summary.unwrap().contains("none of these"));
    }

    #[tokio::test]
    async fn closing_the_chart_returns_to_the_front_door() {
        let state = AppState::mock(Some(uuid::Uuid::from_u128(7)));
        close_chart_impl(&state).await;
        assert!(state.open_patient().await.is_err());
        assert!(funnel_status_impl(&state).await.chart.is_none());
    }
}
