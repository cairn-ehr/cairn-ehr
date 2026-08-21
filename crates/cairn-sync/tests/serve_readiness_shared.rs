//! #457 — the serve-readiness harness's own tests, in ONE home.
//!
//! `tests/common/serve.rs` is pulled into the clinical-pull suite with `#[path]`. Its self-tests
//! deliberately do NOT live inside it: Cargo compiles `tests/*.rs` with `--test`, which sets
//! `cfg(test)`, so a `#[cfg(test)] mod tests` inside the shared module would be compiled and RUN
//! once per including binary — duplicate assertions, duplicate wall clock, and two places a
//! failure has to be read. (Same reasoning, and the same shape, as
//! `crates/cairn-node/tests/source_walk_shared.rs` for the #452 source walk.)
//!
//! # What these pin, and why it is exactly this
//!
//! The defect in #457 is not that `serve` sometimes fails to start. It is that the harness could
//! not tell a **slow** child from a **dead** one, so every cause produced one identical message
//! blaming startup latency — and two rounds of fixes (#238's ceiling, #263's port floor) were
//! aimed at that wrong cause because nobody had ever seen the real one. So the properties worth
//! pinning are about *what the harness can distinguish and what it says*, not about timing:
//!
//! * a dead child is reported as dead, immediately, with its own exit status and stderr;
//! * a child that is alive but never binds is reported as *that*, with its pid, because that is
//!   a different problem with a different diagnosis;
//! * silence from the child is **stated**, never merely absent from the message.
//!
//! These run without the database gate — they are pure functions plus one child process that
//! needs no Postgres — so they hold in every environment, including a DB-free run.

#[path = "common/serve.rs"]
mod serve;

use serve::{classify_readiness, readiness_failure, Readiness, ServeGuard};
use std::time::Duration;

/// A port nothing in this repository ever binds, kept below the #263 ephemeral floor for the
/// same reason every real listen port is: above 32768 the kernel could hand it to an unrelated
/// outbound connection, and a stranger answering on it would make this test's premise ("nothing
/// is listening here") quietly false.
const NEVER_BOUND: &str = "127.0.0.1:25729";

// ---------------------------------------------------------------------------
// classify_readiness — the pure decision behind one poll
// ---------------------------------------------------------------------------

/// **The load-bearing ordering.** A child that has exited can never bind, so "dead" must beat
/// "the port answered" — otherwise a *stranger* holding the port (issue #263's failure mode, and
/// the reason the ports moved below the ephemeral floor) reads as a successful start, and the
/// test proceeds to talk to something that is not our serve loop.
///
/// This is the assertion that fails if someone reorders the match arms for readability.
#[test]
fn a_dead_child_beats_an_accepting_port() {
    assert_eq!(
        classify_readiness(Some("exit status: 1"), true, 100),
        Readiness::ChildExited("exit status: 1".to_string()),
        "an exited child must be reported as exited even when something answers on the port"
    );
}

/// The happy path: alive, and the socket accepts.
#[test]
fn a_live_child_on_an_accepting_port_is_listening() {
    assert_eq!(classify_readiness(None, true, 100), Readiness::Listening);
}

/// Alive, silent, budget left — the only case where polling again is the right answer.
#[test]
fn a_live_child_on_a_silent_port_keeps_waiting() {
    assert_eq!(classify_readiness(None, false, 1), Readiness::Starting);
}

/// Alive, silent, budget spent. Distinct from `ChildExited` because the diagnosis is different:
/// a process that is running and has not bound is stalled, not crashed, and the message says so.
#[test]
fn a_live_child_that_never_binds_times_out() {
    assert_eq!(classify_readiness(None, false, 0), Readiness::TimedOut);
}

/// A dead child at the very last poll is still reported as dead, not as a timeout.
///
/// Pinned separately because the obvious `if polls_remaining == 0 { TimedOut }` early return —
/// a natural way to write this loop — puts the exhausted-budget test *before* the liveness test
/// and silently reclassifies the one case that carries a real exit status.
#[test]
fn a_dead_child_on_the_last_poll_is_still_reported_dead() {
    assert_eq!(
        classify_readiness(Some("signal: 9 (SIGKILL)"), false, 0),
        Readiness::ChildExited("signal: 9 (SIGKILL)".to_string())
    );
}

// ---------------------------------------------------------------------------
// readiness_failure — what the operator actually reads
// ---------------------------------------------------------------------------

/// The #457 message defect, pinned directly: a dead child must not be described in the words of
/// a slow one. The old harness said *"serve did not start listening on ADDR within 60s"* for
/// every cause, which sent two rounds of fixes after startup latency.
#[test]
fn a_dead_child_message_names_the_status_and_does_not_blame_latency() {
    let msg = readiness_failure(
        NEVER_BOUND,
        4242,
        &Readiness::ChildExited("exit status: 1".to_string()),
        Duration::from_millis(40),
        "Error: Os { code: 2, kind: NotFound }\n",
    );

    assert!(
        msg.contains("exit status: 1"),
        "the child's exit status is the whole point of the message: {msg}"
    );
    assert!(
        msg.contains("NotFound"),
        "the child's own stderr must be quoted, not discarded: {msg}"
    );
    assert!(
        !msg.contains("did not start listening"),
        "a child that EXITED must not be described as one that was merely slow: {msg}"
    );
}

/// A timeout names the pid, because the actionable next step for a live-but-stalled child is to
/// look at it (`sample <pid>` on macOS, `gdb -p` / `/proc/<pid>/stack` on Linux). Without the
/// pid the message is a dead end — which is what it was before #457.
#[test]
fn a_timeout_message_names_the_pid_so_the_stall_can_be_inspected() {
    let msg = readiness_failure(
        NEVER_BOUND,
        4242,
        &Readiness::TimedOut,
        Duration::from_secs(60),
        "",
    );

    assert!(
        msg.contains("4242"),
        "the pid must be in the message: {msg}"
    );
    assert!(
        msg.contains("still running") || msg.contains("still alive"),
        "a timeout must say the child is ALIVE — that is what distinguishes it from a crash: {msg}"
    );
}

/// Silence is **stated**, never merely absent.
///
/// An empty stderr section and a message that simply omits stderr look identical to a reader, and
/// they mean opposite things: "the child said nothing" versus "this harness did not capture what
/// the child said". The first is evidence; the second is the #457 defect wearing a fix's clothes.
#[test]
fn an_empty_stderr_is_declared_rather_than_omitted() {
    let msg = readiness_failure(
        NEVER_BOUND,
        4242,
        &Readiness::ChildExited("exit status: 101".to_string()),
        Duration::from_millis(10),
        "",
    );

    assert!(
        msg.contains("nothing to stderr"),
        "an empty stderr must be reported as empty, so it cannot be read as uncaptured: {msg}"
    );
}

// ---------------------------------------------------------------------------
// The end-to-end property, against a real child that needs no database
// ---------------------------------------------------------------------------

/// **The #457 regression test.** A serve child that dies before binding is reported with its own
/// exit status and its own stderr — not as a startup-latency timeout.
///
/// The child is the real `cairn-sync serve` binary given a `--key` path that does not exist. That
/// is a genuine cause of a dead serve child (the key is loaded *before* `TcpListener::bind`, so a
/// bad key means the port is never bound at all), it needs no Postgres — `--conn` is not touched
/// until the first inbound connection — and it fails in milliseconds.
///
/// **Why the assertion is on the message and not on elapsed time.** A timing bound would be the
/// obvious way to say "fails fast", and it would be the wrong one here: this test runs inside the
/// same loaded `cargo test --workspace` sweep whose scheduling noise is the subject of the issue,
/// so a `< 2s` assertion would import exactly the flakiness being fixed. Content separates the
/// two outcomes just as sharply and cannot flake: with the liveness check removed, the harness
/// polls the full ceiling and returns the `TimedOut` text, which contains neither the exit status
/// nor the child's stderr — so every assertion below fails. The ceiling is set generously for the
/// same reason: a slow *exec* must still end in a correct verdict, only a later one.
#[test]
fn a_dead_serve_child_is_reported_with_its_own_evidence() {
    let mut child = ServeGuard::spawn(
        env!("CARGO_BIN_EXE_cairn-sync"),
        "host=127.0.0.1 dbname=this-test-never-connects",
        NEVER_BOUND,
        "/nonexistent/cairn-457/key.json",
    );

    let err = child
        .wait_ready(Duration::from_secs(30))
        .expect_err("a serve child with a nonexistent --key cannot possibly bind");

    assert!(
        err.contains("exit status"),
        "the harness must report the child's exit status: {err}"
    );
    assert!(
        err.contains("No such file or directory") || err.contains("NotFound"),
        "the harness must quote the child's own stderr, which names the missing key file: {err}"
    );
    assert!(
        !err.contains("still running"),
        "a child that exited must not be reported as a live stall: {err}"
    );
}

/// The stderr capture file is cleaned up when the guard drops, on the failure path too.
///
/// Without this the harness would litter one file per failed start into the temp directory, and
/// a *later* run reading a stale file would attribute one test's failure to another's evidence —
/// a fresh way to be told the wrong cause, which is the defect this module exists to remove.
#[test]
fn the_stderr_capture_file_does_not_outlive_the_guard() {
    let path = {
        let mut child = ServeGuard::spawn(
            env!("CARGO_BIN_EXE_cairn-sync"),
            "host=127.0.0.1 dbname=this-test-never-connects",
            NEVER_BOUND,
            "/nonexistent/cairn-457/key.json",
        );
        let path = child.stderr_path().to_path_buf();
        assert!(
            path.exists(),
            "the capture file must exist while the child does: {}",
            path.display()
        );
        let _ = child.wait_ready(Duration::from_secs(30));
        path
    };

    assert!(
        !path.exists(),
        "the guard must remove its capture file on drop: {}",
        path.display()
    );
}
