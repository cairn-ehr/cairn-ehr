//! The window's five commands. Each one is a thin adapter: it resolves state, calls a
//! `cairn-node` function, and maps the result. No clinical logic lives here — that is all
//! in `cairn-medication-view` and `cairn-gui-tab-medications`, under `cargo test`.
//!
//! # Two rules every command in this file follows
//!
//! 1. **Return the underlying error text, never a generic string.** An in-DB floor refusal
//!    is legible on purpose (§9.6); "sign-off failed" throws away the one thing the
//!    clinician needs to act on.
//! 2. **Report partial completion, never imply it** (ADR-0060 decision 2). Every report
//!    type here carries what did NOT happen alongside what did, and the renderer shows it.
use crate::state::{AppState, SessionKey};
use cairn_gui_tab_medications::view::{build_view, missing_report, withheld_report, MedListView};
use cairn_medication_view::PatientMedicationList;
use std::time::Instant;
use uuid::Uuid;

/// Read the chart and build the view model.
///
/// A display never STARTS stale (the reference-shell freshness rule): this always reads
/// fresh from the projections, and nothing on screen is ever replaced without the clinician
/// asking for it.
#[tauri::command]
pub async fn med_list(state: tauri::State<'_, AppState>) -> Result<MedListView, String> {
    Ok(build_view(&read_chart(&state).await?))
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
        .await;
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
    *state.session.lock().await = Some(SessionKey::new(sk, Instant::now()));
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
#[tauri::command]
pub async fn lock_state(state: tauri::State<'_, AppState>) -> Result<LockState, String> {
    let held = state.live_key().await;
    Ok(LockState {
        unlocked: held.is_some(),
        kid: held.map(|(_, kid)| short_kid(&kid).to_string()),
        mock: state.is_mock(),
    })
}

/// What one sign-off gesture did — and, just as important, what it did not do.
#[derive(serde::Serialize)]
pub struct SignOffReport {
    pub signed: usize,
    /// Lines that still need a signature and deliberately did not get one.
    pub withheld_message: Option<String>,
    /// Groups this chart could not display at all.
    pub missing_message: Option<String>,
    /// Lines whose own write failed. ADR-0060 decision 7: each attestation commits in its
    /// own transaction, so a failure here means THIS line was not signed and every other
    /// line still was. Naming which and why is what makes that safe rather than merely
    /// convenient.
    pub failed: Vec<String>,
}

/// Sign off every unsigned or stale drug on the chart — the ONE gesture (#288).
#[tauri::command]
pub async fn sign_off(state: tauri::State<'_, AppState>) -> Result<SignOffReport, String> {
    if state.is_mock() {
        return Err("fixture mode: this window is showing mock data and cannot write".into());
    }
    let (human_sk, human_kid) = state
        .live_key()
        .await
        // Not a failure to hide: the clinician needs to know the key re-locked, not to see
        // a gesture silently do nothing.
        .ok_or("your signing key is locked — unlock it to sign off")?;
    let node_sk = state.node_sk.as_ref().ok_or("no node key")?;

    // Timed around the WRITE only. The clinician's reading time is measured by the operator
    // runbook, not here: capturing it would mean timing how long someone spent thinking.
    let started = Instant::now();
    let outcome = {
        let mut db = state
            .db
            .as_ref()
            .ok_or("no database connection")?
            .lock()
            .await;
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
            state.patient,
        )
        .await
        .map_err(|e| format!("{e:#}"))?
    };
    let elapsed_ms = started.elapsed().as_millis().min(i32::MAX as u128) as i32;

    record_timing(&state, "signoff", outcome.attested.len(), elapsed_ms).await;

    Ok(SignOffReport {
        signed: outcome.attested.len(),
        // Rendered by the SAME two functions the chart itself uses, so the promise made
        // before the gesture and the report made after it cannot word the same fact
        // differently. The ids come from the outcome — what the orchestrator actually did —
        // not from a fresh read that might already disagree.
        withheld_message: withheld_report(&outcome.withheld, &outcome.separation_targets),
        missing_message: missing_report(
            &outcome.groups_missing_from_chart,
            &outcome.separation_targets,
        ),
        failed: outcome
            .failed
            .iter()
            .map(|f| format!("thread {}: {}", f.medication_id, f.error))
            .collect(),
    })
}

/// What one cease gesture did to a group's member threads.
#[derive(serde::Serialize)]
pub struct CeaseReport {
    pub ceased: usize,
    /// Member threads whose cessation failed, named individually. ADR-0060 again: one
    /// member failing must not un-stop the others, and it must never pass in silence.
    pub failed: Vec<String>,
}

/// Stop a drug — every member thread of the displayed line.
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
) -> Result<CeaseReport, String> {
    if state.is_mock() {
        return Err("fixture mode: this window is showing mock data and cannot write".into());
    }
    let reason = reason.trim().to_string();
    if reason.is_empty() {
        return Err(
            "stopping a drug needs a reason — the record must say who stopped it \
                    and why (ADR-0060)"
                .into(),
        );
    }
    let group: Uuid = group_id.parse().map_err(|_| "not a medication id")?;
    let (human_sk, human_kid) = state
        .live_key()
        .await
        .ok_or("your signing key is locked — unlock it to stop a drug")?;
    let node_sk = state.node_sk.as_ref().ok_or("no node key")?;
    let node_kid = hex::encode(node_sk.verifying_key().to_bytes());

    // Which threads make up this displayed line. Read rather than trusted from the caller:
    // the webview knows only the group id it was rendered with, and a reconciled group's
    // membership is a clinical fact the node owns.
    let chart = read_chart(&state).await?;
    let members: Vec<Uuid> = chart
        .rows
        .iter()
        .find(|row| row.group_id == group)
        .ok_or("that drug is no longer on this chart — refresh and try again")?
        .members
        .iter()
        .map(|m| m.medication_id)
        .collect();

    let started = Instant::now();
    let mut ceased = 0usize;
    let mut failed = Vec::new();
    for medication_id in members {
        // One member per call, and `cease_medication` opens its own transaction — so a
        // failure on one thread of a reconciled pair leaves the other one stopped rather
        // than rolling both back (ADR-0060 decision 7).
        let mut db = state
            .db
            .as_ref()
            .ok_or("no database connection")?
            .lock()
            .await;
        let input = cairn_node::medication::CeaseMedicationInput {
            stopped: None,
            stopped_precision: None,
            reason: Some(reason.as_str()),
        };
        let author = cairn_node::medication::AuthorParams {
            human_sk: &human_sk,
            human_kid: &human_kid,
        };
        match cairn_node::medication::cease_medication(
            &mut db,
            node_sk,
            &node_kid,
            &state.node_origin,
            state.patient,
            medication_id,
            &input,
            Some(&author),
            None,
        )
        .await
        {
            Ok(_) => ceased += 1,
            Err(e) => failed.push(format!("thread {medication_id}: {e:#}")),
        }
    }
    let elapsed_ms = started.elapsed().as_millis().min(i32::MAX as u128) as i32;
    record_timing(&state, "cease", 1, elapsed_ms).await;

    Ok(CeaseReport { ceased, failed })
}

/// Read the chart, from the node or from fixtures.
async fn read_chart(state: &tauri::State<'_, AppState>) -> Result<PatientMedicationList, String> {
    let Some(db) = state.db.as_ref() else {
        use cairn_gui_data::port::ClinicalData;
        return cairn_gui_data::mock::MockData::with_fixtures()
            .medications(&state.patient.to_string())
            .map_err(|e| format!("{e:?}"));
    };
    let db = db.lock().await;
    cairn_node::medication::read::list_patient_medications(&*db, state.patient)
        .await
        .map_err(|e| format!("{e:#}"))
}

/// Record what a gesture cost, and never let that recording fail the gesture.
///
/// Timing is observability. A metric that can turn a successful clinical act into a
/// reported failure is a metric that has been allowed to matter more than the act — so the
/// error goes to stderr and the caller is never told.
async fn record_timing(
    state: &tauri::State<'_, AppState>,
    kind: &str,
    items: usize,
    elapsed_ms: i32,
) {
    let Some(db) = state.db.as_ref() else { return };
    let db = db.lock().await;
    if let Err(e) = cairn_node::ui_timing::record_gesture(&*db, kind, items, elapsed_ms).await {
        eprintln!("gesture timing not recorded (the clinical act itself succeeded): {e}");
    }
}

/// The first 8 characters of a key id — enough to tell colleagues apart on screen.
///
/// Sliced on a char boundary: this is display code, and a panic over a label would take the
/// window down.
fn short_kid(kid: &str) -> &str {
    match kid.char_indices().nth(8) {
        Some((byte_index, _)) => &kid[..byte_index],
        None => kid,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// The kid shown on screen must be short enough to read and must never panic, whatever
    /// the id turns out to be — including a short one or a non-ASCII one.
    #[test]
    fn the_displayed_kid_is_truncated_without_panicking() {
        assert_eq!(short_kid("abcdef0123456789"), "abcdef01");
        assert_eq!(short_kid("abc"), "abc");
        assert_eq!(short_kid(""), "");
        assert_eq!(short_kid("ααααααααββ"), "αααααααα");
    }

    /// Every field name `main.js` reads off a payload, in source order of appearance.
    ///
    /// Extracted by scanning for `<binding>.<identifier>` rather than by hand, so the guard
    /// below cannot rot into a list nobody updates.
    fn fields_read_by_the_webview(binding: &str) -> BTreeSet<String> {
        let js = include_str!("../src-ui/main.js");
        let needle = format!("{binding}.");
        let mut found = BTreeSet::new();
        for (index, _) in js.match_indices(&needle) {
            // Skip `foo.bar` where `foo` is really the tail of a longer identifier
            // (e.g. `lock` inside `unlock.`), which would attribute a field to the wrong
            // payload.
            let preceded_by_word = js[..index]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_');
            if preceded_by_word {
                continue;
            }
            let rest = &js[index + needle.len()..];
            let field: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !field.is_empty() {
                found.insert(field);
            }
        }
        found
    }

    /// The keys a serialized payload actually offers.
    fn serialized_keys(value: &serde_json::Value) -> BTreeSet<String> {
        value
            .as_object()
            .expect("payload must serialize to an object")
            .keys()
            .cloned()
            .collect()
    }

    /// THE DRIFT GUARD THIS FILE MOST NEEDS.
    ///
    /// The webview is plain JavaScript with no type checking (the deliberate no-bundler
    /// choice), so a Rust field rename does not break the build — it silently produces
    /// `undefined` on screen. For most fields that is a visible cosmetic bug. For
    /// `missing_message` it is not: `undefined` is falsy, the warning element stays hidden,
    /// and the chart claims completeness it does not have. That is the exact ADR-0060
    /// failure this slice exists to prevent, arriving through a typo.
    ///
    /// So: every field the JS reads must exist in the payload the Rust actually sends.
    #[test]
    fn the_webview_reads_no_field_the_backend_does_not_send() {
        let view =
            cairn_gui_tab_medications::build_view(&cairn_medication_view::fixtures::sample_chart());
        let view_json = serde_json::to_value(&view).unwrap();
        let row_json = view_json["rows"][0].clone();

        let sign_off = serde_json::to_value(SignOffReport {
            signed: 0,
            withheld_message: None,
            missing_message: None,
            failed: vec![],
        })
        .unwrap();
        let cease = serde_json::to_value(CeaseReport {
            ceased: 0,
            failed: vec![],
        })
        .unwrap();
        let lock = serde_json::to_value(LockState {
            unlocked: false,
            kid: None,
            mock: false,
        })
        .unwrap();

        // `report` names both outcome payloads in the JS, so the union is the honest
        // comparison: either command may be the one that supplies a given field.
        let mut report_keys = serialized_keys(&sign_off);
        report_keys.extend(serialized_keys(&cease));

        for (binding, available) in [
            ("view", serialized_keys(&view_json)),
            ("row", serialized_keys(&row_json)),
            ("report", report_keys),
            ("lock", serialized_keys(&lock)),
        ] {
            for field in fields_read_by_the_webview(binding) {
                assert!(
                    available.contains(&field),
                    "src-ui/main.js reads `{binding}.{field}`, which the backend does not \
                     send. Available: {available:?}"
                );
            }
        }
    }

    /// The other direction, for the two fields where silence is the dangerous failure.
    ///
    /// A field the backend sends and the JS ignores is usually harmless — but not these
    /// two: dropping either means a chart that is missing a drug, or a line that was
    /// deliberately left unsigned, renders as if everything were fine (ADR-0060 decision 2).
    #[test]
    fn the_webview_reads_both_partial_completion_reports() {
        let read = fields_read_by_the_webview("view");
        assert!(read.contains("missing_message"), "got: {read:?}");
        assert!(read.contains("withheld_message"), "got: {read:?}");

        let reported = fields_read_by_the_webview("report");
        assert!(reported.contains("missing_message"), "got: {reported:?}");
        assert!(reported.contains("withheld_message"), "got: {reported:?}");
        assert!(reported.contains("failed"), "got: {reported:?}");
    }
}
