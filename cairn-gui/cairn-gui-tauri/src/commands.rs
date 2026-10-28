//! The chart surface's five commands (the front door's are in `funnel::commands`): three act
//! on a chart (`med_list`, `sign_off`, `cease`) and two on the signing session (`unlock`,
//! `lock_state`). Each one is a thin adapter: it resolves state, calls a `cairn-node` function,
//! and maps the result. No clinical logic lives here — that is all in `cairn-medication-view`
//! and `cairn-gui-tab-medications`, under `cargo test`.
//!
//! The three chart commands are one-line `#[tauri::command]` forwarders onto plain
//! `*_impl(&AppState, …)` functions — the same split `funnel::commands` uses — so that the one
//! rule a wrong-chart defect would break (act only on the chart on SCREEN,
//! `AppState::displayed_patient`) is tested against `AppState::mock` with no Tauri runtime.
//! Since R1 that rule covers a SET: a chart linked to others reads as one combined list, and
//! the two writing commands also name the chart set that list was read over and refuse when it
//! changed (ADR-0076 decision 3, `crate::chart_set`).
//!
//! # Two rules every command in this file follows
//!
//! 1. **Return the underlying error text, never a generic string.** An in-DB floor refusal
//!    is legible on purpose (§9.6); "sign-off failed" throws away the one thing the
//!    clinician needs to act on.
//! 2. **Report partial completion, never imply it** (ADR-0060 decision 2). Every report
//!    type here carries what did NOT happen alongside what did, and the renderer shows it.
use crate::chart_set::{
    cease_plan, chart_pane, check_displayed_set, linked_members, set_change, signed_across_message,
    ChartPane, ChartSetError,
};
use crate::funnel::view::{ErrorView, Recorded};
use crate::link::judgement_view::{refused, stale};
use crate::state::{AppState, Now, SessionKey};
use crate::write_failure::{
    already_stopped_note, cease_outcome, chart_read_failed, key_locked, node_cannot_write,
    node_failure, retry_when_nothing_written, sign_off_outcome, UnknownThread,
};
use cairn_gui_tab_medications::view::{missing_report, withheld_report};
use cairn_medication_view::{elsewhere_report, short_kid, PatientMedicationList};
use std::time::Instant;
use uuid::Uuid;
use zeroize::Zeroizing;

/// Read the chart and build the view model.
///
/// A display never STARTS stale (the reference-shell freshness rule): this always reads
/// fresh from the projections, and nothing on screen is ever replaced without the clinician
/// asking for it.
#[tauri::command]
pub async fn med_list(
    state: tauri::State<'_, AppState>,
    patient_id: String,
    seen_charts: Option<Vec<String>>,
) -> Result<ChartPane, String> {
    med_list_impl(&state, &patient_id, seen_charts).await
}

/// `seen_charts` (JS key `seenCharts`): on a re-read, the window's HELD set — what it last drew,
/// moved on by the clinician's own Link/Unlink/Confirm using its report's `charts_after`
/// (`chart_stale.js`'s `applyOwnAct`) — so their own change is never announced back to them. `None` on a first read. The pane's
/// `set_change` names what joined or left since (#797, `chart_set::set_change`).
pub async fn med_list_impl(
    state: &AppState,
    patient_id: &str,
    seen_charts: Option<Vec<String>>,
) -> Result<ChartPane, String> {
    // Bound to the chart the webview is showing (`AppState::displayed_patient`): a read for a
    // chart the clerk has since left must not be rendered under the next one's header.
    let patient = state.displayed_patient(patient_id).await?;
    // The node resolves the chart SET itself; `list.charts` is what the view hands the webview
    // to send back with every write.
    let list = read_chart_of(state, patient).await?;
    // NOT `?`: a failed read of the member identities must never hide the medication list
    // itself (availability over consistency). `chart_pane` turns it into a warning instead.
    let members = linked_members(state, &list.charts).await;
    // Likewise the links and their doubts: an unread list is worded in the pane, never a
    // failed open. (`read_record_links` takes the db lock itself; nothing here holds it.)
    let links = crate::link::record_links::read_record_links(state, &list.charts).await;
    // The possible-duplicate banner (R5a); every failure inside it is a worded line.
    let dups = crate::duplicates::duplicate_section(
        state,
        patient,
        &list.charts,
        members.as_deref().unwrap_or(&[]),
    )
    .await;
    // What joined or left the record since the set the window had on screen; never a failed
    // open (an unread departed chart is named by id).
    let change = set_change(
        state,
        seen_charts.as_deref(),
        patient,
        &list.charts,
        members.as_deref().unwrap_or(&[]),
    )
    .await;
    Ok(chart_pane(&list, patient, members, links, dups, change))
}

/// Whether a signing key is currently held, and whose.
#[derive(serde::Serialize)]
pub struct LockState {
    pub unlocked: bool,
    /// The short key id of whoever is signed in, when someone is. The window shows this so
    /// a clinician cannot sign a chart believing they are someone else.
    pub kid: Option<String>,
    /// True in fixture mode, where there is nothing to unlock and nothing to write.
    pub mock: bool,
}

/// Unseal the clinician's signing key and hold it for the session.
///
/// One unseal per session is what makes the whole-list gesture cost ONE act — see the
/// module doc on `state.rs` for why holding it is the paper-parity answer and re-locking is
/// the safety answer.
#[tauri::command]
pub async fn unlock(
    state: tauri::State<'_, AppState>,
    passphrase: String,
) -> Result<LockState, String> {
    // Wiped when this function returns, on the success path and on every error path alike.
    // The webview clears its own field either way (see `main.js`); this is the other half —
    // a passphrase left in the heap outlives the failure that put it there, and this process
    // stays alive for a whole clinical session.
    let passphrase = Zeroizing::new(passphrase);

    let path = state
        .attester_key_path
        .as_ref()
        .ok_or("this window is showing fixture data; there is nothing to sign")?;

    // The same routine every CLI verb uses to unseal a human key — not a second
    // implementation, because a second one would eventually accept a key the node refuses.
    let sk = cairn_node::keystore::load(path, Some(passphrase.as_str()))
        .map_err(|e| format!("could not unseal the signing key: {e}"))?;
    let kid = hex::encode(sk.verifying_key().to_bytes());

    // Refuse early and legibly rather than letting the in-DB floor reject every signature
    // later. The floor is still the real enforcement (principle 12) — this only means the
    // clinician learns at unlock time, not after reviewing a whole chart.
    let db = state
        .db
        .as_ref()
        .ok_or("no database connection")?
        .lock()
        .await
        // The key is unsealed by now: say so, or a rejected DATABASE password ("password
        // authentication failed") reads as a wrong passphrase.
        .map_err(|e| format!("Your passphrase was right, but the key was not unlocked: the {e}"))?;
    if !cairn_node::identify::attester_is_enrolled_human(&db, &kid)
        .await
        .map_err(|e| e.to_string())?
    {
        return Err(format!(
            "key {} is not an enrolled human actor on this node — run `cairn-node \
             enroll-human` first",
            short_kid(&kid)
        ));
    }
    drop(db);

    let short = short_kid(&kid).to_string();
    *state.session.lock().await = Some(SessionKey::new(sk, Now::read()));
    Ok(LockState {
        unlocked: true,
        kid: Some(short),
        mock: false,
    })
}

/// Report whether a key is still held.
///
/// The frontend polls this, so the window SHOWS the lock rather than the clinician
/// discovering it when a sign-off is refused — the same "state is ambient, never modal"
/// rule the shell design follows.
///
/// It goes through `key_status`, NOT `live_key`, and that distinction is the whole reason
/// the two exist. This runs on a 10-second timer with nobody at the keyboard; treating it as
/// activity meant the window kept resetting its own idle clock and the key never re-locked
/// at all. See the `state` module doc.
#[tauri::command]
pub async fn lock_state(state: tauri::State<'_, AppState>) -> Result<LockState, String> {
    let held = state.key_status(Now::read()).await;
    Ok(LockState {
        unlocked: held.is_some(),
        kid: held.as_deref().map(|kid| short_kid(kid).to_string()),
        mock: state.is_mock(),
    })
}

/// What one sign-off gesture did — and, just as important, what it did not do.
#[derive(Debug, serde::Serialize)]
pub struct SignOffReport {
    /// Kept on the payload beside `text` (as `CeaseReport` keeps its counts); the webview says
    /// `text`, which already carries these.
    pub signed: usize,
    /// Lines whose own write failed. ADR-0060 decision 7: each attestation commits in its
    /// own transaction, so a failure here means THIS line was not signed and the lines signed
    /// beside it stay signed. Lines a stopped gesture never tried (#818) are not here: `text`
    /// names them.
    pub failed: Vec<String>,
    /// The outcome line the webview says (`write_failure::sign_off_outcome`): signed, NOT signed,
    /// where the gesture stopped early if it did (#818), and the chart's own reports — lines
    /// withheld, groups it could not display, lines elsewhere (#849), the linked charts it read across.
    pub text: String,
    /// `Unknown` when the line it stopped at may be signed: the list on screen may be behind the
    /// record, so the webview's re-read marks it "may be out of date" if it fails (#811).
    pub behind_the_record: Option<Recorded>,
}

/// Sign off every unsigned or stale drug on the chart — the ONE gesture (#288).
#[tauri::command]
pub async fn sign_off(
    state: tauri::State<'_, AppState>,
    patient_id: String,
    charts: Vec<String>,
) -> Result<SignOffReport, ErrorView> {
    sign_off_impl(&state, &patient_id, charts).await
}

/// `charts` is the chart SET the webview displayed (`MedListView::charts`, sent back as-is).
pub async fn sign_off_impl(
    state: &AppState,
    patient_id: &str,
    charts: Vec<String>,
) -> Result<SignOffReport, ErrorView> {
    // FIRST: the chart the clinician was LOOKING AT when they pressed the button, and only if
    // it is still the open one — never "whatever is open now" (see
    // `AppState::displayed_patient`). First so that no other refusal can mask this one, and so
    // it is testable in fixture mode.
    let patient = state.displayed_patient(patient_id).await.map_err(refused)?;
    // SECOND: the set that chart reads over must still be the set whose list was on screen
    // (ADR-0076 decision 3). Also ahead of fixture mode's refusal, so a changed set is reported
    // as exactly that everywhere. This is the window's early snapshot; the orchestrator below
    // compares again against its own first read, and that one is authoritative.
    let read_set = read_chart_of(state, patient)
        .await
        .map_err(chart_read_failed)?
        .charts;
    // A CHANGED set is `Retry::AfterReread` (the screen is behind the record); an unreadable
    // list is a window fault and stays a plain refusal — `ChartSetError::into_view`.
    let shown = check_displayed_set(&read_set, &charts).map_err(ChartSetError::into_view)?;
    if state.is_mock() {
        return Err(refused(
            "fixture mode: this window is showing mock data and cannot write",
        ));
    }
    let (human_sk, human_kid) = state
        .live_key(Now::read())
        .await
        // Not a failure to hide: the clinician needs to know the key re-locked, not to see
        // a gesture silently do nothing.
        .ok_or_else(|| key_locked("your signing key is locked — unlock it to sign off"))?;
    let node_sk = state
        .node_sk
        .as_ref()
        .ok_or_else(|| node_cannot_write("no node key"))?;

    // Timed around the WRITE only. The clinician's reading time is measured by the operator
    // runbook, not here: capturing it would mean timing how long someone spent thinking.
    let started = Instant::now();
    let outcome = {
        let mut db = state
            .db
            .as_ref()
            .ok_or_else(|| node_cannot_write("no database connection"))?
            .lock()
            .await?;
        let params = cairn_node::medication::AttestParams {
            human_sk: &human_sk,
            human_kid: &human_kid,
            basis: None,
            note: None,
        };
        cairn_node::medication::signoff::sign_off_medication_list(
            &mut db,
            node_sk,
            &state.node_origin,
            &params,
            patient,
            // What the webview displayed — the orchestrator re-compares it against its own
            // first read, so the authoritative check is against the screen itself.
            Some(&shown),
        )
        .await
        .map_err(|e| node_failure(&e))?
    };
    let elapsed_ms = started.elapsed().as_millis().min(i32::MAX as u128) as i32;
    // Every field named, none behind `..` (#820): a field added to the outcome breaks this
    // build, so a new fact cannot be dropped from the report by default.
    let cairn_node::medication::signoff::SignOffOutcome {
        attested,
        // The window reports counts and names lines, not attestation ids.
        event_ids: _,
        // The window states no "nothing on this chart" sentence; the chart on screen shows it.
        total_rows: _,
        active_rows: _,
        withheld,
        separation_targets,
        groups_missing_from_chart,
        elsewhere,
        failed,
        interrupted,
        charts,
    } = outcome;

    // A gesture that stopped early (#818) is not a sample of what signing off costs, and its
    // connection is likely dead, so recording it would only log a false "succeeded".
    if interrupted.is_none() {
        record_timing(state, "signoff", attested.len(), elapsed_ms).await;
    }

    // Rendered by the SAME two functions the chart itself uses, so the promise made before the
    // gesture and the report made after it cannot word the same fact differently. The ids come
    // from the outcome — what the orchestrator actually did — not from a fresh read that might
    // already disagree.
    let notes = [
        withheld_report(&withheld, &separation_targets),
        missing_report(&groups_missing_from_chart, &separation_targets),
        elsewhere_report(&elsewhere),
        signed_across_message(&charts),
    ];
    let failed: Vec<String> = failed
        .iter()
        .map(|f| format!("thread {}: {}", f.medication_id, f.error))
        .collect();
    let (text, behind_the_record) =
        sign_off_outcome(attested.len(), &failed, interrupted.as_ref(), &notes);
    Ok(SignOffReport {
        signed: attested.len(),
        failed,
        text,
        behind_the_record,
    })
}

/// What one cease gesture did to a group's member threads.
#[derive(Debug, serde::Serialize)]
pub struct CeaseReport {
    pub ceased: usize,
    /// Member threads this gesture did NOT stop, named individually: those whose cessation
    /// failed, and those deliberately held back on a wrong-chart hazard line
    /// (`chart_set::cease_plan`). ADR-0060 again: one member failing must not un-stop the
    /// others, and it must never pass in silence.
    pub failed: Vec<String>,
    /// The outcome line the webview says: stopped, NOT stopped, and — for a thread whose commit
    /// got no ordinary answer (#814) — NOT known (`write_failure::cease_outcome`).
    pub text: String,
    /// `Unknown` when any thread's outcome is unknown: the list on screen may be behind the
    /// record, so the webview's re-read marks it "may be out of date" if it fails (#811).
    pub behind_the_record: Option<Recorded>,
}

/// Stop a drug — every member thread of the displayed line (on a wrong-chart hazard line,
/// every thread on the opened chart; see `chart_set::cease_plan`).
///
/// # Why this surface demands a reason and an author when the CLI verb does not
///
/// ADR-0060's generating framing: an order may be cancelled only by somebody TAKING
/// OWNERSHIP of the cancellation and GIVING A RATIONALE. `medication-cease` accepts neither
/// today (issue #342), which is a gap in a shipped verb's contract that this slice does not
/// reopen. But the fix #342 records is deliberately **local-authoring only** — a door-level
/// NOT NULL would fork the event set the moment a peer replicated a rationale-less
/// cessation — and this window IS a local-authoring surface. So it supplies both: the
/// unlocked clinician is the author (ADR-0053), and the reason is required here. The verb
/// stays unchanged; the window simply never walks the path #342 is about.
#[tauri::command]
pub async fn cease(
    state: tauri::State<'_, AppState>,
    group_id: String,
    reason: String,
    patient_id: String,
    charts: Vec<String>,
) -> Result<CeaseReport, ErrorView> {
    cease_impl(&state, &patient_id, charts, &group_id, &reason).await
}

/// `charts` is the chart SET the webview displayed, exactly as for [`sign_off_impl`].
pub async fn cease_impl(
    state: &AppState,
    patient_id: &str,
    charts: Vec<String>,
    group_id: &str,
    reason: &str,
) -> Result<CeaseReport, ErrorView> {
    // Asked ONCE, and FIRST, for the whole gesture — and only if it is the chart the clinician
    // was looking at (`AppState::displayed_patient`).
    let patient = state.displayed_patient(patient_id).await.map_err(refused)?;
    // Which threads make up this displayed line. Read rather than trusted from the caller:
    // the webview knows only the group id it was rendered with, and a reconciled group's
    // membership is a clinical fact the node owns. Read BEFORE the other refusals so the set
    // check (ADR-0076 decision 3) comes second, as it does for sign-off.
    let chart = read_chart_of(state, patient)
        .await
        .map_err(chart_read_failed)?;
    // The ONLY set check a cease gets: unlike sign-off, `cease_medication` takes one thread
    // and knows nothing of chart sets, so there is no orchestrator-side second compare. What
    // keeps a cease on the right chart regardless is decision 2 below — each thread is
    // written on the chart its own statement is on, read from this same `chart`.
    check_displayed_set(&chart.charts, &charts).map_err(ChartSetError::into_view)?;
    if state.is_mock() {
        return Err(refused(
            "fixture mode: this window is showing mock data and cannot write",
        ));
    }
    let reason = reason.trim().to_string();
    if reason.is_empty() {
        return Err(refused(
            "stopping a drug needs a reason — the record must say who stopped it \
                    and why (ADR-0060)",
        ));
    }
    let group: Uuid = group_id
        .parse()
        .map_err(|_| refused("not a medication id"))?;
    // Each member thread paired with the chart it LIVES on — a cessation is written to its
    // thread's own chart, never to the chart that happened to be opened (ADR-0076 decision
    // 2) — and, on a line withheld as a wrong-chart hazard, only the opened chart's threads
    // (see `cease_plan`). Held-back threads are reported with the failures: both are threads
    // this gesture did NOT stop, and the report must name every one.
    let row = chart
        .rows
        .iter()
        .find(|row| row.group_id == group)
        .ok_or_else(|| stale("that drug is no longer on this chart — nothing was done"))?;
    let plan = cease_plan(row, patient);
    // Nothing this chart may write: said now, before the key, as a release does — an unlock
    // that writes nothing is a step paper never has — and not timed. On a hazard line the window
    // draws Stop only where `cease_plan` writes (#841), and an ordinary line has a thread unless
    // its read raced a reconcile — so a press reaching here means the list on screen is behind
    // the record: say so, so a failed re-read marks it out of date.
    if plan.write.is_empty() {
        return Ok(CeaseReport {
            ceased: 0,
            text: cease_outcome(0, &plan.held_back, &[]),
            failed: plan.held_back,
            behind_the_record: Some(Recorded::No),
        });
    }
    let (human_sk, human_kid) = state
        .live_key(Now::read())
        .await
        .ok_or_else(|| key_locked("your signing key is locked — unlock it to stop a drug"))?;
    let node_sk = state
        .node_sk
        .as_ref()
        .ok_or_else(|| node_cannot_write("no node key"))?;
    let node_kid = hex::encode(node_sk.verifying_key().to_bytes());

    let started = Instant::now();
    let mut ceased = 0usize;
    let mut failed = plan.held_back;
    // Each ordinary failure's class, for the refusal below when nothing was stopped (#840).
    let mut retries = Vec::new();
    let mut unknown = Vec::new();
    for (medication_id, thread_chart) in plan.write {
        // One member per call, and `cease_medication` opens its own transaction — so a
        // failure on one thread of a reconciled pair leaves the other one stopped rather
        // than rolling both back (ADR-0060 decision 7).
        let db = state
            .db
            .as_ref()
            .ok_or_else(|| node_cannot_write("no database connection"))?;
        let input = cairn_node::medication::CeaseMedicationInput {
            stopped: None,
            stopped_precision: None,
            reason: Some(reason.as_str()),
        };
        let author = cairn_node::medication::AuthorParams {
            human_sk: &human_sk,
            human_kid: &human_kid,
        };
        // A lost connection fails THIS thread only: an earlier one may already be written, so
        // it is reported with the others, never as "nothing was done" (#876).
        let result = match db.lock().await {
            Err(lost) => Err(anyhow::Error::from(lost)),
            Ok(mut db) => {
                cairn_node::medication::cease_medication(
                    &mut db,
                    node_sk,
                    &node_kid,
                    &state.node_origin,
                    thread_chart,
                    medication_id,
                    &input,
                    Some(&author),
                    None,
                )
                .await
            }
        };
        match result {
            Ok(_) => ceased += 1,
            // A commit lost mid-flight may have stopped it (#814): not a failure, not a success.
            Err(e) => match cairn_node::db_diagnosis::commit_outcome_unknown_event(&e) {
                Some(event) => unknown.push(UnknownThread {
                    thread: medication_id,
                    event,
                    cause: e.root_cause().to_string(),
                }),
                None => {
                    retries.push(node_failure(&e).retry);
                    failed.push(format!("thread {medication_id}: {e:#}"))
                }
            },
        }
    }
    // Every write failed and none is unknown: nothing was stopped, so this is a refusal, not a
    // report (#840) — as a report it dropped the failure's class, so a stale refusal left the
    // list unmarked if the re-read after it failed. Not timed: no gesture was made.
    if let Some(retry) = retry_when_nothing_written(ceased, &unknown, &retries) {
        return Err(ErrorView {
            text: cease_outcome(0, &failed, &[]),
            retry,
        });
    }
    let elapsed_ms = started.elapsed().as_millis().min(i32::MAX as u128) as i32;
    // As for a sign-off: a gesture with an unknown outcome is no timing sample (#818 review).
    // `ceased > 0` is belt and braces: a cease that stopped nothing has returned above.
    if unknown.is_empty() && ceased > 0 {
        record_timing(state, "cease", 1, elapsed_ms).await;
    }

    let text = cease_outcome(ceased, &failed, &unknown);
    // Said only when every thread of this press stopped for certain (see `already_stopped_note`).
    let landed_in_full = ceased > 0 && failed.is_empty() && unknown.is_empty();
    let text = match already_stopped_note(row, landed_in_full) {
        Some(note) => format!("{text} {note}"),
        None => text,
    };
    Ok(CeaseReport {
        ceased,
        failed,
        text,
        behind_the_record: (!unknown.is_empty()).then_some(Recorded::Unknown),
    })
}

/// Read one named chart, from the node or from fixtures. Callers resolve WHICH chart first —
/// `AppState::displayed_patient` for the open chart, `AppState::shown` for one being compared
/// (`link::same_person::compare_impl`); this only reads.
pub(crate) async fn read_chart_of(
    state: &AppState,
    patient: Uuid,
) -> Result<PatientMedicationList, String> {
    let Some(db) = state.db.as_ref() else {
        use cairn_gui_data::port::ClinicalData;
        return cairn_gui_data::mock::MockData::with_fixtures()
            .medications(&patient.to_string())
            .map_err(|e| format!("{e:?}"));
    };
    let mut db = db.lock().await.map_err(|e| e.to_string())?;
    // One snapshot for the whole read (#864): no line stopped without its reason.
    cairn_node::medication::snapshot::in_snapshot(&mut db, async |tx| {
        cairn_node::medication::read::list_patient_medications(tx, patient).await
    })
    .await
    .map_err(|e| format!("{e:#}"))
}

/// Record what a gesture cost, and never let that recording fail the gesture.
///
/// Timing is observability. A metric that can turn a successful clinical act into a
/// reported failure is a metric that has been allowed to matter more than the act — so the
/// error goes to stderr and the caller is never told.
pub(crate) async fn record_timing(state: &AppState, kind: &str, items: usize, elapsed_ms: i32) {
    let Some(db) = state.db.as_ref() else { return };
    // Never reopens: a reopen here would spend the attempt the act's own re-read needs (#876).
    let Some(db) = db.lock_if_open().await else {
        eprintln!("gesture timing not recorded: the connection is closed");
        return;
    };
    if let Err(e) = cairn_node::ui_timing::record_gesture(&*db, kind, items, elapsed_ms).await {
        // Not "the act succeeded": a gesture whose every line failed is timed too, and its
        // connection may be the very thing that failed it. The act reports its own outcome.
        eprintln!("gesture timing not recorded (the act reports its own outcome): {e:#}");
    }
}

// `short_kid` is `cairn_medication_view::display::short_kid` — ONE truncation rule, because
// this window shows a key id from the view model (the signature column) and one from its own
// lock state (the "signing as …" line) on the same screen. Two copies of the rule is one edit
// away from rendering the same clinician as two different-looking ids.

#[cfg(test)]
#[path = "commands_tests.rs"]
pub(crate) mod tests;
