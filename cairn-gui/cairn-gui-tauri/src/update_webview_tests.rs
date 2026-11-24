//! #909 slice C1: the window's face of a waiting update (`update.js`). The calm line, Update now,
//! the below-floor alert, and the restart at the idle re-lock or a chart's close.
//!
//! The window's scripts have no test rig (no npm or bundler: main.js's header, issue #332), so,
//! like `leave_chart_webview_tests.rs`, each guard reads a script's source (`crate::source_guard`)
//! and pins the ORDER or the CHOICE a property is. The behaviour was walked headlessly (a stub
//! `invoke` in a browser) when it was written — see the PR that added this file.
use crate::source_guard::{js_fn, position};

const UPDATE_JS: &str = include_str!("../src-ui/update.js");
const IDLE_JS: &str = include_str!("../src-ui/idle.js");
const LEAVE_JS: &str = include_str!("../src-ui/leave_chart.js");
const INDEX_HTML: &str = include_str!("../src-ui/index.html");

/// The plan's copy, verbatim (Global Constraints). The below-floor sentence carries the stamp's
/// reason, so its two fixed halves are pinned.
#[test]
fn the_copy_is_the_designs() {
    let face = format!("{UPDATE_JS}{INDEX_HTML}");
    for copy in [
        "An update is waiting; it installs when this terminal is next idle.",
        ">Update now</button>",
        "No newer version is installed on this terminal yet — ask whoever installs Cairn here.",
        "The version installed here does not fit this node yet.",
        "\"This terminal needs its update: it does not show \" + reason + \". Finish what you are doing.\"",
    ] {
        assert!(face.contains(copy), "{copy}");
    }
}

/// Design §4.4: a restart waits for every act in flight, then closes the chart (its queued saves
/// awaited) — the idle re-lock's own sequence — and only then restarts the program.
#[test]
fn a_restart_waits_for_acts_and_the_close() {
    let restart = js_fn(UPDATE_JS, "restartIfFits");
    let settled = position(restart, "await signsSettled();");
    let closed = position(restart, "await closeChart();");
    let restarted = position(restart, "invoke(\"restart_window\")");
    assert!(settled < closed && closed < restarted, "{restart}");
}

/// Design §4.4: the installed program is asked FIRST. Unchanged or not fitting, the window says so
/// and closes nothing: restarting into the same program would bring the same line back.
#[test]
fn a_restart_asks_first() {
    let restart = js_fn(UPDATE_JS, "restartIfFits");
    let asked = position(restart, "invoke(\"installed_program\")");
    let fits = position(restart, "if (installed.kind !== \"fits\") {");
    assert!(asked < fits, "{restart}");
    assert!(
        fits < position(restart, "await signsSettled();"),
        "{restart}"
    );
    assert!(fits < position(restart, "await closeChart();"), "{restart}");
}

/// Review Focus 3: at the lock screen the poll restarts only after a whole idle period without a
/// touch, so someone typing the terminal passphrase is never restarted under; and never before
/// the lock poll has said how long that period is.
#[test]
fn the_lock_screen_restart_needs_a_full_idle_period() {
    let poll = js_fn(UPDATE_JS, "pollUpdate");
    for needed in [
        "!el(\"terminal-lock\").hidden",
        "idleLockMs > 0",
        "Date.now() - lastActivityAt >= idleLockMs",
        "updateView.kind !== \"current\"",
    ] {
        assert!(
            position(poll, needed) < position(poll, "restartIfFits()"),
            "{needed}"
        );
    }
    // Only a person resets it (idle.js): set where the idle clock is pushed back, and at load.
    assert!(IDLE_JS.contains("let lastActivityAt = Date.now();"));
    assert!(js_fn(IDLE_JS, "noteActivity").contains("lastActivityAt = Date.now();"));
    assert_eq!(
        IDLE_JS.matches("lastActivityAt =").count(),
        2,
        "the declaration and noteActivity, nowhere else"
    );
    assert!(
        !UPDATE_JS.contains("lastActivityAt ="),
        "a poll is not a person"
    );
}

/// Design §4.5: the alert is persistent but never takes the chart away; the clinician finishes it.
#[test]
fn the_alert_never_closes_the_open_chart() {
    let render = js_fn(UPDATE_JS, "renderUpdate");
    assert!(!render.contains("closeChart"), "{render}");
    assert!(!render.contains("restartIfFits"), "{render}");
}

/// The re-lock and a chart's close are where the update installs (design §4.4–§4.5); both hand
/// over only after they finished. After a close the window restarts below the floor, or with an
/// update waiting when the lock is showing — never under someone still at the front door.
#[test]
fn the_relock_and_the_close_hand_over_once_done() {
    let relock = js_fn(IDLE_JS, "idleRelock");
    assert!(position(relock, "relockTerminal(lock);") < position(relock, "updateAfterRelock();"));
    let leave = js_fn(LEAVE_JS, "leaveChart");
    assert!(position(leave, "invoke(\"close_chart\")") < position(leave, "updateAfterClose();"));
    let after_close = js_fn(UPDATE_JS, "updateAfterClose");
    assert!(after_close.contains("updateView.kind === \"below_floor\""));
    assert!(
        after_close.contains("(updateView.kind === \"waiting\" && !el(\"terminal-lock\").hidden)")
    );
    assert!(js_fn(UPDATE_JS, "updateAfterRelock").contains("updateView.kind !== \"current\""));
    // A second attempt while one is running does nothing: two would close and restart twice.
    assert!(js_fn(UPDATE_JS, "restartIfFits").contains("if (restarting) return;"));
}

/// Its own cadence, not the 10 s lock poll's (design §4.4): a minute bounds how long a running
/// window takes to learn of an upgrade.
#[test]
fn the_update_poll_is_a_minute() {
    assert!(UPDATE_JS.contains("const UPDATE_POLL_MS = 60000;"));
    assert!(UPDATE_JS.contains("setInterval(pollUpdate, UPDATE_POLL_MS);"));
}

/// Loaded after leave_chart.js (whose `closeChart` it calls) and idle.js (whose clock it reads).
#[test]
fn update_js_loads_after_leave_chart() {
    let tag = position(INDEX_HTML, "<script src=\"update.js\"></script>");
    assert!(position(INDEX_HTML, "<script src=\"leave_chart.js\"></script>") < tag);
    assert!(position(INDEX_HTML, "<script src=\"idle.js\"></script>") < tag);
}
