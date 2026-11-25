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
//! - **A 60 s poll** (`update.js`), [`update_state`] / [`update_poll`], re-reads the stamp on
//!   the window's one shared connection (reopening it if lost) and remembers the verdict and the
//!   stamp; it also starts a release fetch when one is due (`crate::release_fetch`, #909 C2).
//! - **The gate**, [`refuse_another_chart`]: below the floor, opening a DIFFERENT chart and
//!   registering are refused. The open chart's re-reads, Sign, saves and close never come here.
//! - **Before a restart**, [`installed_program`] asks the program now installed at this window's
//!   path for its generation (`--print-schema-generation`, connecting to nothing) and judges it
//!   against the stamp, so the window never restarts into the same program, or one this database
//!   would refuse, or while the node does not answer. Then [`restart_window`] restarts.
use crate::state::AppState;
use cairn_node::db::{
    embedded_schema_hash, embedded_schema_version, read_schema_stamp, SchemaStamp,
};
use cairn_node::window_floor::{
    install_fit, window_verdict, InstallFit, InstalledBuild, WindowVerdict,
};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The flag that makes this program print its generation line and exit (`main.rs`). A contract
/// ACROSS generations: an older window runs the newer program with it, so neither the flag nor
/// [`generation_line`]'s format ever changes, and [`asks_for_generation_line`] reads it before
/// clap parses anything.
pub const PRINT_FLAG: &str = "--print-schema-generation";

/// The flag that makes this program print its signed release version (a decimal `u64`; 0 when
/// built without one) and exit, answered before clap like [`PRINT_FLAG`]. An older program that
/// does not know it fails or prints usage, which reads as release 0.
pub const RELEASE_FLAG: &str = "--print-release-version";

/// This build's signed release version, stamped at build time by `CAIRN_RELEASE_VERSION`;
/// absent or unparsable is 0 (an unreleased build).
pub fn release_version() -> u64 {
    option_env!("CAIRN_RELEASE_VERSION")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

/// The line `--print-release-version` prints.
pub fn release_line(release: u64) -> String {
    release.to_string()
}

/// Was this program started to print its release line? Read BEFORE clap. **Pure.**
pub fn asks_for_release_line(args: impl IntoIterator<Item = std::ffi::OsString>) -> bool {
    args.into_iter().skip(1).any(|arg| arg == RELEASE_FLAG)
}

/// [`release_line`] read back; anything else is `None`. **Pure.**
pub fn parse_release_line(line: &str) -> Option<u64> {
    line.trim().parse().ok()
}

/// How long the installed program may take to print its line. A hung or slow one reads as "not
/// installed" (Review Focus 4): it must never block, or darken, the window asking.
pub const INSTALLED_CHECK_TIMEOUT: Duration = Duration::from_secs(5);

/// Said for a refused open or registration below the floor (the plan's copy, verbatim).
pub const REFUSED_ANOTHER_CHART: &str =
    "This terminal needs its update before it can open another chart.";

/// What the webview shows: `current` (nothing), `waiting` (the calm line) or `below_floor` (the
/// alert, naming `reason` when the stamp gave one). A refusal learned mid-session is
/// `below_floor` too, gated the same, but its alert is the refusal's own sentence (`refusal`).
/// `note` is the courier's line (`crate::release_fetch::installer_view`): fetching, or the
/// installer's refusal; shown whatever the kind.
#[derive(Debug, PartialEq, Serialize)]
pub struct UpdateView {
    pub kind: &'static str,
    pub reason: Option<String>,
    pub refusal: Option<String>,
    pub note: Option<String>,
}

/// What the installed program would be: `same` (nothing newer), `fits` (restart into it),
/// `does_not_fit` (this database would refuse it, or tell it to update), `unreachable` (the node
/// did not answer just now, so no restart: the relaunched window would exit at its first connect)
/// or `unknown_install` (this window cannot name its own program's path, so it cannot check).
#[derive(Debug, PartialEq, Serialize)]
pub struct InstallView {
    pub kind: &'static str,
}

/// The line `--print-schema-generation` prints: `<generation> <64 hex>`.
pub fn generation_line(generation: i32, hash: [u8; 32]) -> String {
    format!("{generation} {}", hex::encode(hash))
}

/// Was this program started to print its generation line? `args` are the raw arguments,
/// `std::env::args_os()`, read BEFORE clap: a required argument added later, or one this build's
/// clap refuses, must never stop an older window's question from being answered. **Pure.**
pub fn asks_for_generation_line(args: impl IntoIterator<Item = std::ffi::OsString>) -> bool {
    args.into_iter().skip(1).any(|arg| arg == PRINT_FLAG)
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
    let (kind, reason, refusal) = match verdict {
        WindowVerdict::Current => ("current", None, None),
        WindowVerdict::UpdateWaiting => ("waiting", None, None),
        WindowVerdict::BelowFloor { reason } => ("below_floor", reason.clone(), None),
        WindowVerdict::Refused(text) => ("below_floor", None, Some(text.clone())),
    };
    UpdateView {
        kind,
        reason,
        refusal,
        note: None,
    }
}

/// Where an install check's stamp came from: only a fresh read may lead to a restart.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StampSource {
    /// Read just now: the node answered, so a relaunched window can connect.
    Fresh,
    /// The last stamp read (at launch or by a poll): the node did not answer this time.
    Cached,
    /// Neither: no stamp has ever been read.
    Unknown,
}

/// The webview's view of an install fit judged on a stamp from `source`. **Pure.** A fit on
/// anything but a fresh stamp, or a misfit on no stamp at all, is `unreachable`: a restart now
/// would relaunch into an outage, and the window would exit at its first connect.
pub fn install_view(fit: InstallFit, source: StampSource) -> InstallView {
    let kind = match (fit, source) {
        (InstallFit::Same, _) => "same",
        (InstallFit::Fits, StampSource::Fresh) => "fits",
        (InstallFit::DoesNotFit, StampSource::Fresh | StampSource::Cached) => "does_not_fit",
        _ => "unreachable",
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

/// One read of the stamp on the shared connection, reopening it if it was lost (#807, with its
/// retry backoff). A remote window's only link runs through the terminal service's gateway, which
/// restarts on an upgrade (exit 75), so without the reopen the window would never learn of the
/// upgrade. `None` is no answer: fixture mode, a reopen that failed, or a read that failed (each
/// said on stderr). `Some(None)` is an answer: the database carries no stamp.
async fn read_stamp(state: &AppState, why: &str) -> Option<Option<SchemaStamp>> {
    // Accepted cost: the poll may spend #807's reopen attempt and start its backoff, so a press
    // within about 2 s of a failed poll reopen tries nothing. The node is unreachable then anyway,
    // and the poll runs at most once a minute.
    let conn = match state.db.as_ref()?.lock().await {
        Ok(conn) => conn,
        Err(lost) => {
            eprintln!("the {why} could not reach the node: {lost}");
            return None;
        }
    };
    match read_schema_stamp(&conn).await {
        Ok(stamp) => Some(stamp),
        Err(e) => {
            eprintln!(
                "the {why} could not read the schema stamp: {:#}",
                conn.explain(e)
            );
            None
        }
    }
}

/// What a poll's read means for the window. **Pure but for `state`'s two cells.** An answer
/// replaces the verdict and the stamp it came from; no answer changes nothing, so the window says
/// what it last knew — a blip must not lift a below-floor alert, nor raise one.
pub fn poll_answer(state: &AppState, read: Option<Option<SchemaStamp>>) -> UpdateView {
    let mut verdict = state.verdict.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(stamp) = read {
        *verdict = window_verdict(stamp.as_ref(), embedded_schema_version());
        *state.last_stamp.lock().unwrap_or_else(|p| p.into_inner()) = stamp;
    }
    update_view(&verdict)
}

/// The stamp an install check judges against, and where it came from: a fresh answer, else the
/// last stamp read (at launch or by a poll), else none.
pub fn install_stamp(
    state: &AppState,
    read: Option<Option<SchemaStamp>>,
) -> (Option<SchemaStamp>, StampSource) {
    if let Some(stamp) = read {
        return (stamp, StampSource::Fresh);
    }
    match state
        .last_stamp
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
    {
        Some(stamp) => (Some(stamp), StampSource::Cached),
        None => (None, StampSource::Unknown),
    }
}

/// The poll (`update.js`, once a minute), and the release fetch it found due
/// (`crate::release_fetch`), which the Tauri forwarder spawns: a fetch takes minutes, and the
/// poll's answer must not wait for it. Fixture mode reads nothing and fetches nothing.
pub async fn update_poll(state: &AppState) -> (UpdateView, Option<crate::release_fetch::Job>) {
    if state.is_mock() {
        return (update_view(&WindowVerdict::Current), None);
    }
    let read = read_stamp(state, "update check").await;
    let fresh = read.clone();
    let view = poll_answer(state, read);
    crate::release_fetch::after_poll(state, view, fresh.as_ref()).await
}

/// This running program as an installed build: what an installed or offered one is judged
/// against.
pub fn running_build() -> InstalledBuild {
    InstalledBuild {
        generation: embedded_schema_version(),
        hash: embedded_schema_hash(),
        release: release_version(),
    }
}

/// A live window that cannot name its own program's path cannot check what is installed there
/// (macOS reads a path through any symlink as none: ADR-0111). Fixture mode never asks. **Pure.**
pub fn path_unknown(live: bool, program: Option<&Path>) -> bool {
    live && program.is_none()
}

/// What the program installed at this window's path would be against the stamp. Fixture mode has
/// no installed path and is always `same`, so it never restarts (Review Focus 5).
pub async fn installed_program_impl(state: &AppState) -> InstallView {
    if path_unknown(!state.is_mock(), state.launched_exe.as_deref()) {
        return InstallView {
            kind: "unknown_install",
        };
    }
    let read = match state.is_mock() {
        true => None,
        false => read_stamp(state, "install check").await,
    };
    let (stamp, source) = install_stamp(state, read);
    let fit = installed_fit(state.launched_exe.clone(), &[PRINT_FLAG], stamp.as_ref()).await;
    install_view(fit, source)
}

/// Run `program args` for its generation line (and `RELEASE_FLAG` for its release, 0 if it
/// cannot answer) and judge it against `stamp`. No program, one
/// that fails, hangs past [`INSTALLED_CHECK_TIMEOUT`] or prints anything else is not installed.
pub async fn installed_fit(
    program: Option<PathBuf>,
    args: &[&str],
    stamp: Option<&SchemaStamp>,
) -> InstallFit {
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    let installed = match program {
        // Off the async runtime: the waits below sleep.
        Some(program) => tokio::task::spawn_blocking(move || {
            let (generation, hash) = generation_of(&program, &args, INSTALLED_CHECK_TIMEOUT)?;
            // An older program cannot answer the release flag: release 0.
            let release = run_for_line(
                &program,
                &[RELEASE_FLAG.to_string()],
                INSTALLED_CHECK_TIMEOUT,
            )
            .and_then(|l| parse_release_line(&l))
            .unwrap_or(0);
            Some(InstalledBuild {
                generation,
                hash,
                release,
            })
        })
        .await
        .ok()
        .flatten(),
        None => None,
    };
    install_fit(installed, running_build(), stamp)
}

/// Run `program args` and read its generation line. Blocking; see [`run_for_line`].
fn generation_of(program: &Path, args: &[String], timeout: Duration) -> Option<(i32, [u8; 32])> {
    parse_generation_line(&run_for_line(program, args, timeout)?)
}

/// Run `program args`, wait at most `timeout`, and return its stdout if it exited successfully.
/// Blocking. A program still running at the deadline is killed, so a hung one leaves nothing
/// behind.
fn run_for_line(program: &Path, args: &[String], timeout: Duration) -> Option<String> {
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
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

// ---- The Tauri forwarders (`update.js` calls all three). ----

/// The poll; a release fetch it found due runs on its own task, through the app's state (a
/// `tauri::State` cannot outlive this call).
#[tauri::command]
pub async fn update_state(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<UpdateView, ()> {
    use tauri::Manager;
    let (view, job) = update_poll(&state).await;
    if let Some(job) = job {
        tauri::async_runtime::spawn(async move {
            let state = app.state::<AppState>();
            crate::release_fetch::fetch(&state, job.offer, &job.dir, job.pinned_root).await;
        });
    }
    Ok(view)
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
