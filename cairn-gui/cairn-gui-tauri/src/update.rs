//! An older window against a newer database (#909 slice C1, design §4.4–§4.5).
//!
//! The window is installed apart from the node, so the node's database can be upgraded under a
//! window that is still running. The database's stamp carries a *floor*
//! (`cairn_node::window_floor`): at or above it this window keeps working and only says an update
//! is waiting; below it the window finishes the chart in hand and opens no other.
//!
//! This module is the window's Rust half of that:
//!
//! - **At launch**, [`launch_verdict`] opens the window or refuses it with a sentence.
//! - **A 60 s poll** (`update.js`), [`update_state`], re-reads the stamp on the window's one
//!   shared connection, never reopening it, and remembers the verdict in `AppState::verdict`.
//! - **The gate**, [`refuse_another_chart`]: below the floor, opening a DIFFERENT chart and
//!   registering are refused. The open chart's re-reads, Sign, saves and close never come here.
//! - **Before a restart**, [`installed_program`] asks the program now installed at this window's
//!   path for its generation (`--print-schema-generation`, connecting to nothing) and judges it
//!   against the stamp, so the window never restarts into the same program, or one this database
//!   would refuse. Then [`restart_window`] restarts.
use crate::state::AppState;
use cairn_node::db::{
    embedded_schema_hash, embedded_schema_version, read_schema_stamp, SchemaStamp,
};
use cairn_node::window_floor::{install_fit, window_verdict, InstallFit, WindowVerdict};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The flag that makes this program print its generation line and exit (`main.rs`).
pub const PRINT_FLAG: &str = "--print-schema-generation";

/// How long the installed program may take to print its line. A hung or slow one reads as "not
/// installed" (Review Focus 4): it must never block, or darken, the window asking.
pub const INSTALLED_CHECK_TIMEOUT: Duration = Duration::from_secs(5);

/// Said for a refused open or registration below the floor (the plan's copy, verbatim).
pub const REFUSED_ANOTHER_CHART: &str =
    "This terminal needs its update before it can open another chart.";

/// What the webview shows: `current` (nothing), `waiting` (the calm line) or `below_floor` (the
/// alert, naming `reason` when the stamp gave one).
#[derive(Debug, PartialEq, Serialize)]
pub struct UpdateView {
    pub kind: &'static str,
    pub reason: Option<String>,
}

/// What the installed program would be: `same` (nothing newer), `fits` (restart into it) or
/// `does_not_fit` (this database would refuse it, or tell it to update).
#[derive(Debug, PartialEq, Serialize)]
pub struct InstallView {
    pub kind: &'static str,
}

/// The line `--print-schema-generation` prints: `<generation> <64 hex>`.
pub fn generation_line(generation: i32, hash: [u8; 32]) -> String {
    format!("{generation} {}", hex::encode(hash))
}

/// [`generation_line`] read back. **Pure.** Anything else — an older program's usage text, a
/// half line — is `None`, read as "not installed".
pub fn parse_generation_line(line: &str) -> Option<(i32, [u8; 32])> {
    let (generation, hash) = line.trim().split_once(' ')?;
    let mut bytes = [0u8; 32];
    hex::decode_to_slice(hash, &mut bytes).ok()?;
    Some((generation.parse().ok()?, bytes))
}

/// The installed program's path from this process's own. On Linux a running program whose file
/// was replaced reads as `<path> (deleted)`; the file now at `<path>` is the installed one.
pub fn exe_path(raw: PathBuf) -> PathBuf {
    match raw.to_str().and_then(|s| s.strip_suffix(" (deleted)")) {
        Some(path) => PathBuf::from(path),
        None => raw,
    }
}

/// The launch's decision on the verdict (design §4.5): keep it and open, or refuse with a sentence.
pub fn launch_verdict(verdict: WindowVerdict) -> Result<WindowVerdict, String> {
    match verdict {
        WindowVerdict::Refused(text) => Err(text),
        WindowVerdict::BelowFloor { reason } => {
            let missed = match reason {
                Some(reason) => format!("it does not show {reason}"),
                None => "it may not show newer records correctly".to_string(),
            };
            Err(format!(
                "This window is older than the node's database can show correctly: {missed}. \
                 Install the window that matches the node."
            ))
        }
        fine => Ok(fine),
    }
}

/// Is this verdict below the floor? A refusal learned mid-session (another loader restamped the
/// database, say) counts as below: every doubt resolves toward refusing (design §4.1).
fn below_floor(verdict: &WindowVerdict) -> bool {
    matches!(
        verdict,
        WindowVerdict::BelowFloor { .. } | WindowVerdict::Refused(_)
    )
}

/// The webview's view of a verdict. **Pure.**
pub fn update_view(verdict: &WindowVerdict) -> UpdateView {
    let (kind, reason) = match verdict {
        WindowVerdict::Current => ("current", None),
        WindowVerdict::UpdateWaiting => ("waiting", None),
        WindowVerdict::BelowFloor { reason } => ("below_floor", reason.clone()),
        WindowVerdict::Refused(_) => ("below_floor", None),
    };
    UpdateView { kind, reason }
}

/// The webview's view of an install fit. **Pure.**
pub fn install_view(fit: InstallFit) -> InstallView {
    let kind = match fit {
        InstallFit::Same => "same",
        InstallFit::Fits => "fits",
        InstallFit::DoesNotFit => "does_not_fit",
    };
    InstallView { kind }
}

/// The gate (design §4.5): below the floor, refuse to open `patient` unless it is the chart
/// already open, and refuse every registration (`None`). Above it, nothing is refused.
pub async fn refuse_another_chart(
    state: &AppState,
    patient: Option<uuid::Uuid>,
) -> Result<(), String> {
    // Read and released before the chart lock: a std mutex is never held across an await.
    let below = below_floor(&state.verdict.lock().unwrap_or_else(|p| p.into_inner()));
    if !below {
        return Ok(());
    }
    let open = state.chart.lock().await.as_ref().map(|c| c.patient);
    match patient {
        Some(patient) if open == Some(patient) => Ok(()),
        _ => Err(REFUSED_ANOTHER_CHART.to_string()),
    }
}

/// The poll: re-read the stamp and remember the verdict. Fixture mode has no database and is
/// always `current`. A lost connection is never reopened here (the next act reopens it), and a
/// read that fails changes nothing: the window says what it last knew, which is `current` unless
/// the floor was already learned — a blip must not lift a below-floor alert, nor raise one.
pub async fn update_state_impl(state: &AppState) -> UpdateView {
    let last = || update_view(&state.verdict.lock().unwrap_or_else(|p| p.into_inner()));
    let Some(db) = state.db.as_ref() else {
        return update_view(&WindowVerdict::Current);
    };
    let Some(conn) = db.lock_if_open().await else {
        return last();
    };
    let stamp = match read_schema_stamp(&conn).await {
        Ok(stamp) => stamp,
        Err(e) => {
            eprintln!(
                "the update check could not read the schema stamp: {:#}",
                conn.explain(e)
            );
            return last();
        }
    };
    drop(conn);
    let verdict = window_verdict(stamp.as_ref(), embedded_schema_version());
    let view = update_view(&verdict);
    *state.verdict.lock().unwrap_or_else(|p| p.into_inner()) = verdict;
    view
}

/// What the program installed at this window's path would be against the stamp read now. Fixture
/// mode has no installed path and is always `same`, so it never restarts (Review Focus 5). With
/// no open connection the stamp is unknown, and an installed program then reads as not fitting.
pub async fn installed_program_impl(state: &AppState) -> InstallView {
    let stamp = match state.db.as_ref() {
        None => None,
        Some(db) => match db.lock_if_open().await {
            None => None,
            Some(conn) => read_schema_stamp(&conn).await.unwrap_or_else(|e| {
                eprintln!(
                    "the install check could not read the schema stamp: {:#}",
                    conn.explain(e)
                );
                None
            }),
        },
    };
    install_view(installed_fit(state.launched_exe.clone(), &[PRINT_FLAG], stamp.as_ref()).await)
}

/// Run `program args` for its generation line and judge it against `stamp`. No program, one
/// that fails, hangs past [`INSTALLED_CHECK_TIMEOUT`] or prints anything else is not installed.
pub async fn installed_fit(
    program: Option<PathBuf>,
    args: &[&str],
    stamp: Option<&SchemaStamp>,
) -> InstallFit {
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    let installed = match program {
        // Off the async runtime: the wait below sleeps.
        Some(program) => tokio::task::spawn_blocking(move || {
            generation_of(&program, &args, INSTALLED_CHECK_TIMEOUT)
        })
        .await
        .ok()
        .flatten(),
        None => None,
    };
    let running = (embedded_schema_version(), embedded_schema_hash());
    install_fit(installed, running, stamp)
}

/// Run `program args`, wait at most `timeout`, and read its line. Blocking. A program still
/// running at the deadline is killed, so a hung one leaves nothing behind.
fn generation_of(program: &Path, args: &[String], timeout: Duration) -> Option<(i32, [u8; 32])> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + timeout;
    // shortcut: polls every 50 ms (std has no wait-with-timeout); fine for one short-lived child.
    while child.try_wait().ok()?.is_none() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let output = child.wait_with_output().ok()?;
    output.status.success().then_some(())?;
    parse_generation_line(&String::from_utf8_lossy(&output.stdout))
}

// ---- The Tauri forwarders (`update.js` calls all three). ----

#[tauri::command]
pub async fn update_state(state: tauri::State<'_, AppState>) -> Result<UpdateView, ()> {
    Ok(update_state_impl(&state).await)
}

#[tauri::command]
pub async fn installed_program(state: tauri::State<'_, AppState>) -> Result<InstallView, ()> {
    Ok(installed_program_impl(&state).await)
}

/// Restart into the installed program. `update.js` calls it only after `installed_program`
/// said `fits` and the chart closed; fixture mode never restarts, whatever asks.
#[tauri::command]
pub async fn restart_window(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), ()> {
    if state.is_mock() {
        return Ok(());
    }
    app.restart()
}

#[cfg(test)]
#[path = "update_tests.rs"]
mod tests;
