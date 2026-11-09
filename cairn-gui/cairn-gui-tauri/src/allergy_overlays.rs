//! Allergy slice 2's two new acts on a line of the allergy list: **De-label** ("challenged:
//! tolerated …", `clinical.allergy-challenge.asserted`) and **Entered in error** (a withdrawal,
//! `clinical.allergy-retraction.asserted`). The third, **Correct**, is `allergy_record` sent with
//! `corrects` (`allergies.rs`), because a correction IS a new entry.
//!
//! Each runs `allergies.rs`'s steps in its order: the chart on screen and its set, fixture mode
//! (`on_screen_and_writable`), every verdict on what was typed, then ONE step slice 1 does not
//! have — [`target_before_the_key`]: the line the act names must still be on a FRESH read of the
//! list over the same chart set (another terminal may have corrected or withdrawn it since the
//! window drew it), and the event is filed under that line's own chart (plan D2: a linked chart's
//! entry is overlaid on its chart, which is the only chart the floor admits it on). Only then the
//! key, the one signed event, its timing sample and the list read again (`sign_then_reread`,
//! rulings R11 and #814 unchanged).
use crate::allergies::{
    local_today, on_screen_and_writable, parse_onset, read_list, sign_then_reread, Act,
    AllergyWrite,
};
use crate::chart_set::{check_displayed_set, ChartSetError};
use crate::funnel::view::{ErrorView, Recorded, Retry};
use crate::link::judgement_view::{refused, stale};
use crate::signer::Credential;
use crate::state::AppState;
use cairn_event::allergy_overlay::{AllergyChallenge, AllergyRetraction};
use cairn_node::allergy::read::PatientAllergies;
use cairn_node::allergy::view::{build_view, AllergyView};
use chrono::Offset;
use uuid::Uuid;

/// De-label one line: what was tolerated, when (typed as an onset is; empty = not known), and an
/// optional note. Answers with the list read again.
#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri passes each argument the webview sends by name.
pub async fn allergy_challenge(
    state: tauri::State<'_, AppState>,
    patient_id: String,
    charts: Vec<String>,
    target: String,
    tolerated: String,
    date: Option<String>,
    text: Option<String>,
    credential: Option<Credential>,
) -> Result<AllergyWrite, ErrorView> {
    allergy_challenge_impl(
        &state,
        &patient_id,
        charts,
        &target,
        &tolerated,
        date.as_deref(),
        text.as_deref(),
        credential,
    )
    .await
}

/// Mark a line, a "none known" statement or a de-label as entered in error, with an optional
/// reason. Answers with the list read again.
#[tauri::command]
pub async fn allergy_retract(
    state: tauri::State<'_, AppState>,
    patient_id: String,
    charts: Vec<String>,
    target: String,
    text: Option<String>,
    credential: Option<Credential>,
) -> Result<AllergyWrite, ErrorView> {
    allergy_retract_impl(
        &state,
        &patient_id,
        charts,
        &target,
        text.as_deref(),
        credential,
    )
    .await
}

/// De-label `target` (refusal order: module doc). `tolerated` is trimmed and must name
/// something; `date` reads as an onset does (`parse_onset`); a blank note is none.
#[allow(clippy::too_many_arguments)]
pub async fn allergy_challenge_impl(
    state: &AppState,
    patient_id: &str,
    charts: Vec<String>,
    target: &str,
    tolerated: &str,
    date: Option<&str>,
    text: Option<&str>,
    credential: Option<Credential>,
) -> Result<AllergyWrite, ErrorView> {
    let patient = on_screen_and_writable(state, patient_id, &charts).await?;
    let tolerated = tolerated.trim();
    if tolerated.is_empty() {
        return Err(refused("Name what was tolerated."));
    }
    let date = parse_onset(date, local_today()).map_err(refused)?;
    let found = target_before_the_key(state, patient, &charts, target, Names::Entry).await?;
    let target = found.id.to_string();
    let act = Act::Challenge(AllergyChallenge {
        target: &target,
        tolerated,
        date,
        text: note(text),
    });
    sign_then_reread(state, patient, found.chart, act, credential).await
}

/// Mark `target` entered in error (refusal order: module doc); a blank reason is none.
pub async fn allergy_retract_impl(
    state: &AppState,
    patient_id: &str,
    charts: Vec<String>,
    target: &str,
    text: Option<&str>,
    credential: Option<Credential>,
) -> Result<AllergyWrite, ErrorView> {
    let patient = on_screen_and_writable(state, patient_id, &charts).await?;
    let found = target_before_the_key(state, patient, &charts, target, Names::Withdrawable).await?;
    let target = found.id.to_string();
    let act = Act::Retraction(AllergyRetraction {
        target: &target,
        text: note(text),
    });
    sign_then_reread(state, patient, found.chart, act, credential).await
}

/// Free text as signed: trimmed, and blank is none. Pure.
fn note(text: Option<&str>) -> Option<&str> {
    text.map(str::trim).filter(|t| !t.is_empty())
}

/// What an act may name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Names {
    /// A live line of the drug, other or adverse-effect list (Correct, De-label).
    Entry,
    /// A live line, a standing or previous "none known" line, or a historical line's de-label
    /// (Withdraw de-label) or its own entry (Entered in error: one act strikes it, ADR-0097 decision 19).
    Withdrawable,
}

/// The line an act names, as found on the fresh read: its event and the chart it is filed under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Target {
    pub id: Uuid,
    pub chart: Uuid,
}

/// Read the list again over the chart set the window sent, in one snapshot, and find `target`
/// on it (`target_on`). Said before the key: a passphrase that writes nothing is a step paper
/// never has.
///
/// The gap between this read and the sign is DELIBERATE, and safe — do not "fix" it with a lock.
/// No human step sits in it except the key (and locking the list across the key's KDF would
/// serialise the terminal behind one clinician's typing). Whatever another terminal or a peer
/// signs in it — a correction, de-label or withdrawal of the same line — is a pair of acts a
/// partitioned peer produces anyway, and the view absorbs it: an overlay covers only what it
/// names, so every order over-warns (ADR-0097 decision 14 and "not guaranteed" (i)). The floor
/// still checks the target's type and chart.
pub(crate) async fn target_before_the_key(
    state: &AppState,
    patient: Uuid,
    charts: &[String],
    target: &str,
    names: Names,
) -> Result<Target, ErrorView> {
    let (set, a) = read_list(state, patient).await?;
    check_displayed_set(&set, charts).map_err(ChartSetError::into_view)?;
    let v = build_view(&a, chrono::Local::now().offset().fix());
    target_on(&a, &v, target, names)
}

/// Said when the act's target is on a chart whose link to this one is in doubt (review C1).
pub(crate) const ON_A_DOUBTED_CHART: &str = "This line is on a chart whose link to this one is \
     in doubt — open that chart, or settle the link, to change it.";

/// Whether `target` names a line `names` allows on view `v` of rows `a`, and that row's chart.
/// A target not on the view — corrected, de-labelled or withdrawn since the window drew it, or
/// never a line at all — is refused with a re-read: the list on screen is behind the record.
///
/// A target on a chart whose link to this one is in DOUBT is refused too (review C1): that line
/// may be a stranger's, shown here only because over-warning is the safe error, so no act from
/// this chart may strike, de-label or correct it — that would take a possible stranger's warning
/// off their own chart. The window draws no button on such a line; this refusal (with a re-read,
/// which takes a button drawn before the doubt away) is the backstop. Pure.
pub(crate) fn target_on(
    a: &PatientAllergies,
    v: &AllergyView,
    target: &str,
    names: Names,
) -> Result<Target, ErrorView> {
    let found = find_target(a, v, target, names)?;
    if a.doubted_charts.contains(&found.chart) {
        return Err(stale(ON_A_DOUBTED_CHART));
    }
    Ok(found)
}

/// `target_on` without the doubt rule: the line `target` names on `v`, and its row's chart.
fn find_target(
    a: &PatientAllergies,
    v: &AllergyView,
    target: &str,
    names: Names,
) -> Result<Target, ErrorView> {
    let gone = || ErrorView {
        text: "That allergy line is no longer on this chart — read it again.".into(),
        retry: Retry::AfterReread(Recorded::No),
    };
    let id = Uuid::parse_str(target).map_err(|_| gone())?;
    let live = v
        .drug_allergies
        .iter()
        .chain(&v.other_allergies)
        .chain(&v.adverse_effects)
        .find(|l| l.event_id == id);
    if let Some(l) = live {
        return Ok(Target { id, chart: l.chart });
    }
    if names == Names::Entry {
        return Err(gone());
    }
    let shown = v
        .standing_none
        .iter()
        .chain(&v.previously_none)
        .any(|n| n.event_id == id)
        || v.historical
            .iter()
            .any(|l| l.challenge == Some(id) || l.event_id == id);
    // The view names the line; its chart is on the row it reads.
    let chart = a
        .none
        .iter()
        .map(|n| (n.event_id, n.chart))
        .chain(a.challenges.iter().map(|c| (c.event_id, c.chart)))
        .chain(a.entries.iter().map(|e| (e.event_id, e.chart)))
        .find(|(e, _)| *e == id)
        .map(|(_, chart)| chart);
    match chart {
        Some(chart) if shown => Ok(Target { id, chart }),
        _ => Err(gone()),
    }
}

#[cfg(test)]
#[path = "allergy_overlays_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "allergy_overlays_pure_tests.rs"]
mod pure_tests;
