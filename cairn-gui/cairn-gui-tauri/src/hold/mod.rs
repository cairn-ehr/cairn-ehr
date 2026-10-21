//! The window's withhold and release gestures (#832): which thread a "W" is written on and
//! which holds a Release ends, decided by pure functions in [`plan`] before anything touches
//! the database; the two commands that act on those plans; and the sentences they report
//! (`outcome`).
//!
//! Both commands follow `commands::cease_impl`'s steps — the wrong-chart refusal first, the same
//! "each thread written on its own chart" rule (ADR-0076 decision 2), the same split of a lost
//! commit from an ordinary failure (#814) — so a reviewer who knows one knows the others. One
//! difference in order: the signing key is checked LAST here, after every refusal that is a
//! verdict on the gesture (a cease checks it before finding the line). The
//! in-DB floor (db/059) is the real enforcement: it refuses a withhold on a ceased line, an empty
//! reason, an end not after the start, and a release naming something that is not a hold, and
//! its legible sentence reaches the clinician through `node_failure`. The window's own checks
//! are defence in depth and earlier, plainer wording — never a second copy of the floor's state
//! rules.
use crate::chart_set::{check_displayed_set, ChartSetError};
use crate::commands::{read_chart_of, record_timing};
use crate::funnel::view::{ErrorView, Recorded};
use crate::link::judgement_view::{refused, stale};
use crate::state::{AppState, Now};
use crate::write_failure::{
    chart_read_failed, key_locked, node_cannot_write, node_failure, UnknownThread,
};
use cairn_event::medication::HoldUntil;
use outcome::{release_outcome, withhold_outcome};
use std::time::Instant;
use uuid::Uuid;

mod outcome;
pub(crate) mod plan;

/// What one Withhold or Release did, as the webview says it.
#[derive(Debug, serde::Serialize)]
pub struct HoldReport {
    /// The outcome line (`outcome::withhold_outcome` / `outcome::release_outcome`): done, NOT
    /// done, and NOT known, each named.
    pub text: String,
    /// `Unknown` when a commit got no ordinary answer: the list on screen may be behind the
    /// record, so the webview's re-read marks it "may be out of date" if it fails (#811).
    pub behind_the_record: Option<Recorded>,
}

/// When a hold ends, as the clinician chose it. Owned (unlike the wire's `HoldUntil`), so it
/// can be parsed before the write and borrowed into `HoldUntil` at the call.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum UntilChoice {
    /// Until a clinician reviews it — the open-ended "W".
    Reviewed,
    /// Until this RFC 3339 instant, passed through exactly as the webview wrote it.
    At(String),
}

/// Read the webview's end-of-hold field. The end is a clinical fact the clinician must choose:
/// the webview preselects "until reviewed" ON SCREEN, where it is seen and changed, and sends
/// `""` for "until a time" with no (or a partly typed) time. That empty end is refused with a
/// sentence saying what to choose — never turned into "until reviewed" here, which would be a
/// hold nobody decided on (principle 4). Only the shape is
/// checked here; that the end is AFTER the start is the floor's rule (db/059).
pub(crate) fn parse_until(until: &str) -> Result<UntilChoice, String> {
    match until {
        "reviewed" => Ok(UntilChoice::Reviewed),
        "" => Err("choose when the hold ends: a time, or until reviewed".to_string()),
        text if chrono::DateTime::parse_from_rfc3339(text).is_ok() => {
            Ok(UntilChoice::At(text.to_string()))
        }
        text => Err(format!("not a time: {text}")),
    }
}

/// Withhold a drug — write a "W" on the displayed line (#832, ADR-0081).
#[tauri::command]
pub async fn withhold(
    state: tauri::State<'_, AppState>,
    group_id: String,
    reason: String,
    until: String,
    patient_id: String,
    charts: Vec<String>,
) -> Result<HoldReport, ErrorView> {
    withhold_impl(&state, &patient_id, charts, &group_id, &reason, &until).await
}

/// The testable body of [`withhold`]; the flow is the module doc's. `charts` is the chart SET
/// the webview displayed, exactly as for `commands::cease_impl`.
pub async fn withhold_impl(
    state: &AppState,
    patient_id: &str,
    charts: Vec<String>,
    group_id: &str,
    reason: &str,
    until: &str,
) -> Result<HoldReport, ErrorView> {
    // FIRST, as for every chart write: only the chart the clinician was LOOKING AT, so no other
    // refusal can mask a wrong-chart one (`AppState::displayed_patient`).
    let patient = state.displayed_patient(patient_id).await.map_err(refused)?;
    // The line's threads are read, never trusted from the caller (as for a cease), and read
    // BEFORE the other refusals so the set check (ADR-0076 decision 3) comes second.
    let chart = read_chart_of(state, patient)
        .await
        .map_err(chart_read_failed)?;
    check_displayed_set(&chart.charts, &charts).map_err(ChartSetError::into_view)?;
    if state.is_mock() {
        return Err(refused(
            "fixture mode: this window is showing mock data and cannot write",
        ));
    }
    // The reason and the end are the clinician's own words; checked before the line lookup so a
    // missing one is named whatever the state of the chart.
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(refused(
            "withholding a drug needs a reason — the chart must say why it is not being given",
        ));
    }
    let until = parse_until(until).map_err(refused)?;
    let group: Uuid = group_id
        .parse()
        .map_err(|_| refused("not a medication id"))?;
    let row = chart
        .rows
        .iter()
        .find(|row| row.group_id == group)
        .ok_or_else(|| stale("that drug is no longer on this chart — nothing was done"))?;
    // Which thread carries the W, and the chart it lives on (`plan::withhold_target`): never
    // the opened chart by assumption, and never another person's drug on a hazard line.
    let (medication_id, thread_chart) = plan::withhold_target(row, patient).map_err(refused)?;
    // The key last (a cease asks for it earlier): a locked key is not a verdict on the gesture,
    // so every refusal that IS one is said first.
    let (human_sk, human_kid) = state
        .live_key(Now::read())
        .await
        .ok_or_else(|| key_locked("your signing key is locked — unlock it to withhold a drug"))?;
    let node_sk = state
        .node_sk
        .as_ref()
        .ok_or_else(|| node_cannot_write("no node key"))?;
    let node_kid = hex::encode(node_sk.verifying_key().to_bytes());

    let started = Instant::now();
    let result = {
        let mut db = state
            .db
            .as_ref()
            .ok_or_else(|| node_cannot_write("no database connection"))?
            .lock()
            .await;
        let input = cairn_node::medication::WithholdMedicationInput {
            reason,
            // "From now": the node stamps the physical now (ADR-0081).
            from: None,
            until: match &until {
                UntilChoice::Reviewed => HoldUntil::Reviewed,
                UntilChoice::At(at) => HoldUntil::At(at),
            },
        };
        let author = cairn_node::medication::AuthorParams {
            human_sk: &human_sk,
            human_kid: &human_kid,
        };
        cairn_node::medication::withhold_medication(
            &mut db,
            node_sk,
            &node_kid,
            &state.node_origin,
            thread_chart,
            medication_id,
            &input,
            Some(&author),
        )
        .await
    };
    // ONE write, so one outcome: landed, NOT known (#814), or an ordinary failure returned as
    // the node's own sentence.
    let unknown = match result {
        Ok(_) => None,
        Err(e) => match cairn_node::db_diagnosis::commit_outcome_unknown_event(&e) {
            Some(event) => Some(UnknownThread {
                thread: medication_id,
                event,
                cause: e.root_cause().to_string(),
            }),
            None => return Err(node_failure(&e)),
        },
    };
    let elapsed_ms = started.elapsed().as_millis().min(i32::MAX as u128) as i32;
    // A gesture with an unknown outcome is no timing sample (#818 review).
    if unknown.is_none() {
        record_timing(state, "withhold", 1, elapsed_ms).await;
    }
    let (text, behind_the_record) = withhold_outcome(unknown.as_ref());
    Ok(HoldReport {
        text,
        behind_the_record,
    })
}

/// Release holds early — end the holds the clinician was SHOWN on the displayed line.
#[tauri::command]
pub async fn release(
    state: tauri::State<'_, AppState>,
    group_id: String,
    withholds: Vec<String>,
    reason: String,
    patient_id: String,
    charts: Vec<String>,
) -> Result<HoldReport, ErrorView> {
    release_impl(&state, &patient_id, charts, &group_id, &withholds, &reason).await
}

/// `withholds` are the hold (withhold event) ids the webview displayed on this line: a release
/// never ends a hold its author was not shown (ADR-0081 decision 4, `plan::release_plan`).
/// `reason` is optional; blank means none.
pub async fn release_impl(
    state: &AppState,
    patient_id: &str,
    charts: Vec<String>,
    group_id: &str,
    withholds: &[String],
    reason: &str,
) -> Result<HoldReport, ErrorView> {
    // The same first three steps as a withhold, for the same reasons.
    let patient = state.displayed_patient(patient_id).await.map_err(refused)?;
    let chart = read_chart_of(state, patient)
        .await
        .map_err(chart_read_failed)?;
    check_displayed_set(&chart.charts, &charts).map_err(ChartSetError::into_view)?;
    if state.is_mock() {
        return Err(refused(
            "fixture mode: this window is showing mock data and cannot write",
        ));
    }
    if withholds.is_empty() {
        return Err(refused("no hold named — nothing was done"));
    }
    let shown = withholds
        .iter()
        .map(|id| id.parse::<Uuid>())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| refused("not a hold id"))?;
    let group: Uuid = group_id
        .parse()
        .map_err(|_| refused("not a medication id"))?;
    let row = chart
        .rows
        .iter()
        .find(|row| row.group_id == group)
        .ok_or_else(|| stale("that drug is no longer on this chart — nothing was done"))?;
    let plan = plan::release_plan(row, &shown, patient);
    // Nothing to write and nothing deliberately held back: every hold shown has already ended,
    // so the screen is behind the record — re-read it, nothing was done.
    if plan.write.is_empty() && plan.held_back.is_empty() {
        return Err(stale(
            "the holds shown are no longer on this line — nothing was done",
        ));
    }
    // Nothing this chart may write, only holds held back: said now, before the key — a locked
    // key would otherwise hide "release it from its own chart" behind an unlock that writes
    // nothing — and no timing sample, as no gesture was made.
    if plan.write.is_empty() {
        return Ok(HoldReport {
            text: release_outcome(0, plan.gone.len(), plan.unseen, &plan.held_back, &[]),
            behind_the_record: None,
        });
    }
    let reason = Some(reason.trim()).filter(|r| !r.is_empty());
    let (human_sk, human_kid) = state
        .live_key(Now::read())
        .await
        .ok_or_else(|| key_locked("your signing key is locked — unlock it to release a hold"))?;
    let node_sk = state
        .node_sk
        .as_ref()
        .ok_or_else(|| node_cannot_write("no node key"))?;
    let node_kid = hex::encode(node_sk.verifying_key().to_bytes());

    let started = Instant::now();
    let mut released = 0usize;
    // Held-back holds are reported with the failures: both are holds this gesture did NOT end.
    let mut failed = plan.held_back;
    let mut unknown = Vec::new();
    for (medication_id, thread_chart, ids) in plan.write {
        // One thread per call, each its own transaction, so one thread failing leaves the
        // others released rather than rolling them back (ADR-0060 decision 7).
        let mut db = state
            .db
            .as_ref()
            .ok_or_else(|| node_cannot_write("no database connection"))?
            .lock()
            .await;
        let author = cairn_node::medication::AuthorParams {
            human_sk: &human_sk,
            human_kid: &human_kid,
        };
        match cairn_node::medication::release_medication_holds(
            &mut db,
            node_sk,
            &node_kid,
            &state.node_origin,
            thread_chart,
            medication_id,
            &ids,
            reason,
            Some(&author),
        )
        .await
        {
            Ok(_) => released += ids.len(),
            // A commit lost mid-flight may have released them (#814): neither done nor failed.
            Err(e) => match cairn_node::db_diagnosis::commit_outcome_unknown_event(&e) {
                Some(event) => unknown.push(UnknownThread {
                    thread: medication_id,
                    event,
                    cause: e.root_cause().to_string(),
                }),
                None => failed.push(format!("thread {medication_id}: {e:#}")),
            },
        }
    }
    let elapsed_ms = started.elapsed().as_millis().min(i32::MAX as u128) as i32;
    // Timed only when it released something and no outcome is unknown (#818 review): a release
    // whose every write failed made no gesture worth a sample.
    if unknown.is_empty() && released > 0 {
        record_timing(state, "release", 1, elapsed_ms).await;
    }
    Ok(HoldReport {
        text: release_outcome(released, plan.gone.len(), plan.unseen, &failed, &unknown),
        behind_the_record: (!unknown.is_empty()).then_some(Recorded::Unknown),
    })
}

#[cfg(test)]
#[path = "command_tests.rs"]
mod command_tests;
