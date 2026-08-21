//! #457 — spawning `cairn-sync serve` for a test, and telling "slow" from "dead".
//!
//! # The defect this module exists to remove
//!
//! Twelve tests in `clinical_pull.rs` spawn the real `serve` binary and then wait for it to
//! accept TCP. The wait used to be this:
//!
//! ```ignore
//! fn wait_listening(addr: &str) {
//!     for _ in 0..600 {
//!         if std::net::TcpStream::connect(addr).is_ok() { return; }
//!         std::thread::sleep(std::time::Duration::from_millis(100));
//!     }
//!     panic!("serve did not start listening on {addr} within 60s");
//! }
//! ```
//!
//! It polls a **port** and never looks at the **child**. Every possible cause — `EADDRINUSE`, a
//! `--key` path that does not exist, a panic before `bind` — produced that one message, which
//! names startup latency and nothing else. Two rounds of fixes were aimed at latency as a result
//! (#238 raised the ceiling 5 s → 60 s, #263 moved every port below the ephemeral floor) and the
//! flake outlived both, because **nobody had ever seen why a child failed**: the harness threw the
//! evidence away and then spent a full minute reporting the wrong cause.
//!
//! So the fix is not a third ceiling. It is to make the harness capable of distinguishing the
//! cases at all, and to say which one it saw:
//!
//! 1. **the child exited** — report it immediately, with its exit status and its own stderr;
//! 2. **the child is alive and has not bound** — report *that*, with the pid, so it can be
//!    inspected while it is still stalled;
//! 3. **the socket accepts** — the only success.
//!
//! # What the harness now knows about `serve`'s own startup
//!
//! `cmd_serve` in `crates/cairn-sync/src/main.rs` loads `--key` and then calls
//! `TcpListener::bind(listen)` as its *first* action, printing `serving on <addr>` to stderr
//! immediately afterwards. `--conn` is not touched until the first inbound connection arrives.
//! Two consequences a reader should hold:
//!
//! * a bad `--key` kills the child **before** the port is ever bound — the case the end-to-end
//!   test in `serve_readiness_shared.rs` drives, and one that needs no database;
//! * a bad `--conn` does **not** stop the child from binding, so a connection-string mistake
//!   shows up later as a per-connection error, never as a readiness failure.
//!
//! # The remaining hypothesis, left visible on purpose
//!
//! The observed flake is *always exactly three of twelve, always the full ceiling*, on macOS
//! under a loaded parallel sweep — and serialised (`--test-threads=2`) it is 12/12 in seconds.
//! A child that is **alive but has not reached `main`** fits that shape: this project has hit
//! macOS `_dyld_start` loader stalls between test binaries before. This module cannot fix that,
//! but it can now name it — a stall reports as `TimedOut` with a live pid, which is a different
//! sentence from a crash and points at `sample <pid>` rather than at the ceiling.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// How long a *live* child is given to bind before the wait gives up.
///
/// Unchanged from #238 on purpose. The #457 fix is that a **dead** child no longer waits at all,
/// which is the case that was burning the whole ceiling; whether a live child still needs sixty
/// seconds is a separate question, and it can only be answered once the diagnostics below have
/// named a real stall. Changing both at once would destroy that evidence.
const READY_CEILING: Duration = Duration::from_secs(60);

/// Gap between readiness polls. Small enough that the happy path returns promptly, large enough
/// that the poll loop is not itself a busy spin competing with the child for CPU.
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Distinguishes capture files when several guards are alive at once.
///
/// Cargo runs the tests inside one binary on parallel threads, so two guards can exist
/// simultaneously — and if they shared a capture path, one guard's `Drop` would delete the file
/// the other is still reading. Attributing one child's evidence to another test is a fresh way of
/// being told the wrong cause, which is the very defect this module removes.
static CAPTURE_SEQ: AtomicU64 = AtomicU64::new(0);

/// What ONE readiness poll observed.
///
/// Four outcomes rather than the old two (returned / kept looping), because the two failures have
/// **different diagnoses** and must never share a sentence.
#[derive(Debug, PartialEq, Eq)]
pub enum Readiness {
    /// The socket accepted and the child is alive — `serve` is up.
    Listening,
    /// The child has exited. No amount of further polling can succeed. Carries the rendered
    /// `ExitStatus` (`"exit status: 1"`, `"signal: 9 (SIGKILL)"`).
    ChildExited(String),
    /// Neither yet, and the budget has polls left — try again.
    Starting,
    /// The budget is spent and the child is **still running**. A stall, not a crash.
    TimedOut,
}

/// Decide what one poll means. Pure: the two observations in, one verdict out.
///
/// # The ordering is the point
///
/// Liveness is tested **first**, before both the port and the remaining budget:
///
/// * before the **port**, because an exited child cannot be the thing answering. If something
///   accepts on our address after the child is gone, it is a stranger — exactly issue #263's
///   failure mode — and treating that as success hands the test a peer that is not `serve`.
/// * before the **budget**, because a child that dies on the last poll still has an exit status
///   worth reporting, and the obvious `if polls_remaining == 0 { TimedOut }` early return would
///   throw it away.
///
/// Both orderings are pinned by tests in `serve_readiness_shared.rs`, because both are the kind
/// of thing a later reader reorders for readability.
pub fn classify_readiness(
    child_status: Option<&str>,
    port_accepts: bool,
    polls_remaining: u32,
) -> Readiness {
    match (child_status, port_accepts, polls_remaining) {
        (Some(status), _, _) => Readiness::ChildExited(status.to_string()),
        (None, true, _) => Readiness::Listening,
        (None, false, 0) => Readiness::TimedOut,
        (None, false, _) => Readiness::Starting,
    }
}

/// Render the child's stderr for a failure message.
///
/// An empty capture is **declared**, never omitted. "The child said nothing" and "this harness
/// did not capture what the child said" look identical when the section is simply absent, and
/// they mean opposite things — the first is evidence, the second is the #457 defect wearing a
/// fix's clothes.
fn stderr_section(stderr: &str) -> String {
    if stderr.trim().is_empty() {
        "the child printed nothing to stderr".to_string()
    } else {
        format!(
            "--- the child's stderr ---\n{}\n--- end of stderr ---",
            stderr.trim_end()
        )
    }
}

/// Build the text a readiness failure panics with. Pure, so its wording is testable.
///
/// One function, but deliberately **not** one sentence: each failure gets its own diagnosis and
/// its own next step. Collapsing them back into a shared summary line is the regression — it is
/// literally what the old harness did.
pub fn readiness_failure(
    addr: &str,
    pid: u32,
    verdict: &Readiness,
    waited: Duration,
    stderr: &str,
) -> String {
    let cause = match verdict {
        Readiness::ChildExited(status) => format!(
            "serve on {addr} (pid {pid}) EXITED after {:.1?} without ever binding the port — {status}.\n\
             No amount of waiting could have helped, so this is not a startup-latency problem: \
             read the child's own output below for the cause.",
            waited
        ),
        Readiness::TimedOut => format!(
            "serve on {addr} (pid {pid}) is still running after {:.1?} and has not bound the port.\n\
             The child is alive, so this is a stall rather than a crash. Inspect it while it is \
             stalled — `sample {pid}` on macOS, `gdb -p {pid}` or /proc/{pid}/stack on Linux — \
             and see the #457 notes in tests/common/serve.rs on the macOS loader-stall hypothesis.",
            waited
        ),
        // Not failures. Named explicitly so the match stays exhaustive without inventing a third
        // message shape that could then be produced by accident.
        Readiness::Listening | Readiness::Starting => format!(
            "harness bug: readiness_failure called for a non-failure verdict ({verdict:?}) \
             on {addr} (pid {pid})"
        ),
    };
    format!("{cause}\n{}", stderr_section(stderr))
}

/// A spawned `serve` child, its captured stderr, and the promise to clean both up.
///
/// `Drop` kills the child and removes the capture file, so a leaked listener can never wedge a
/// later run on the fixed port and a stale capture can never be read as a live child's evidence.
pub struct ServeGuard {
    child: Child,
    /// Captured at spawn: after `try_wait` reaps the child, `Child::id` is no longer a handle to
    /// anything, and the pid is the one thing a stalled-child message most needs.
    pid: u32,
    addr: String,
    stderr_path: PathBuf,
}

impl ServeGuard {
    /// Spawn `serve` with its stderr captured to a file. Does **not** wait for readiness.
    ///
    /// A file rather than `Stdio::piped()` on purpose: an unread pipe fills its kernel buffer and
    /// then **blocks the child**, which would turn the capture into a new way of never binding —
    /// a readiness harness that causes the failure it reports. A file has no such limit and can be
    /// read at any moment, including while the child is still alive.
    pub fn spawn(bin: &str, conn: &str, listen: &str, key: &str) -> Self {
        let seq = CAPTURE_SEQ.fetch_add(1, Ordering::Relaxed);
        let stderr_path = std::env::temp_dir().join(format!(
            "cairn-serve-{}-{}-{seq}.log",
            listen.replace([':', '.'], "-"),
            std::process::id()
        ));
        // `File::create` truncates, so a capture can never inherit a previous run's text.
        let capture = std::fs::File::create(&stderr_path)
            .unwrap_or_else(|e| panic!("harness: cannot create {}: {e}", stderr_path.display()));

        let child = Command::new(bin)
            .args(["serve", "--conn", conn, "--listen", listen, "--key", key])
            .stderr(Stdio::from(capture))
            .spawn()
            .unwrap_or_else(|e| panic!("harness: cannot spawn {bin} serve on {listen}: {e}"));

        Self {
            pid: child.id(),
            child,
            addr: listen.to_string(),
            stderr_path,
        }
    }

    /// Where this child's stderr is being captured (used by the harness's own tests).
    pub fn stderr_path(&self) -> &Path {
        &self.stderr_path
    }

    /// Everything the child has written to stderr so far.
    ///
    /// Read on demand rather than held, so a message built at any point in the wait shows what the
    /// child had said *by then*. An unreadable capture degrades to a stated fact, never to
    /// silence — silence here would be indistinguishable from a child that printed nothing.
    fn captured_stderr(&self) -> String {
        match std::fs::read_to_string(&self.stderr_path) {
            Ok(text) => text,
            Err(e) => format!(
                "(harness could not read the capture file {}: {e})",
                self.stderr_path.display()
            ),
        }
    }

    /// Poll until the child serves, dies, or the ceiling is spent.
    ///
    /// Returns `Err(message)` rather than panicking so this module's own tests can assert on the
    /// wording — the wording *is* the fix — without catching an unwind.
    pub fn wait_ready(&mut self, ceiling: Duration) -> Result<(), String> {
        let started = Instant::now();
        // At least one poll even for a zero ceiling: a caller asking "is it up?" must get a real
        // answer, not an immediate timeout that never looked.
        let mut polls_remaining =
            (ceiling.as_millis() / POLL_INTERVAL.as_millis()).clamp(1, u32::MAX as u128) as u32;

        loop {
            polls_remaining = polls_remaining.saturating_sub(1);

            // Ask the CHILD before the port — see `classify_readiness` for why the order matters.
            let exited = self.child.try_wait().map_err(|e| {
                format!(
                    "harness: cannot poll the serve child (pid {}) on {}: {e}",
                    self.pid, self.addr
                )
            })?;
            let status = exited.map(|s| s.to_string());
            let accepts = std::net::TcpStream::connect(&self.addr).is_ok();

            match classify_readiness(status.as_deref(), accepts, polls_remaining) {
                Readiness::Listening => return Ok(()),
                Readiness::Starting => std::thread::sleep(POLL_INTERVAL),
                failure => {
                    return Err(readiness_failure(
                        &self.addr,
                        self.pid,
                        &failure,
                        started.elapsed(),
                        &self.captured_stderr(),
                    ));
                }
            }
        }
    }
}

impl Drop for ServeGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        // Best-effort: a capture file that outlives its guard would be read as a later child's
        // evidence. Failing to remove it is not worth failing a test over, but it must be tried.
        let _ = std::fs::remove_file(&self.stderr_path);
    }
}

/// Spawn `serve` and block until it is genuinely serving.
///
/// This is what the twelve `clinical_pull.rs` tests call. It replaces a fourteen-line
/// `Command::new(...).spawn()` block **plus** a separate `wait_listening(PORT)` call at each
/// site — a shape in which a thirteenth test could add the spawn and forget the wait, or wait on
/// a different port than it served. Here the two cannot come apart: the address is named once.
///
/// Panics with the child's own evidence if it never serves.
pub fn serve(bin: &str, conn: &str, listen: &str, key: &str) -> ServeGuard {
    let mut guard = ServeGuard::spawn(bin, conn, listen, key);
    if let Err(message) = guard.wait_ready(READY_CEILING) {
        panic!("{message}");
    }
    guard
}
