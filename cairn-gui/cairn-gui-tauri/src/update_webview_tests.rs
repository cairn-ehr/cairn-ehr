//! #909 slice C1: the window's face of a waiting update (`update.js`). The calm line, Update now,
//! the below-floor alert, and the restart at the idle re-lock or a chart's close.
//!
//! The window's scripts have no test rig (no npm or bundler: main.js's header, issue #332), so,
//! like `leave_chart_webview_tests.rs`, each guard reads a script's source (`crate::source_guard`)
//! and pins the ORDER or the CHOICE a property is. The behaviour was walked headlessly (a stub
//! `invoke` in a browser) when it was written — see the PR that added this file.
use crate::source_guard::{js_const, js_fn, position};

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
        "This terminal cannot reach the node right now; the update installs once it can.",
        "This terminal cannot check which version is installed here; restart it once the update is installed.",
        "\"This terminal needs its update: it does not show \" + reason + \". Finish what you are doing.\"",
    ] {
        assert!(face.contains(copy), "{copy}");
    }
}

/// Design §4.4: a restart waits for every act in flight, then closes the chart (its queued saves
/// awaited) — the idle re-lock's own sequence — and only then restarts the program. An act that
/// did not settle well keeps the chart on screen and restarts nothing, as "Find another patient"
/// and the person menu do (#1130): closing it would erase the only place its outcome is said.
///
/// The act's outcome is CAPTURED before the install check (re-review of I1): a note's Sign holds
/// the window's one connection, so `installed_program` waits behind it and answers only once the
/// Sign has settled and `signInFlight()` is false again. Sampled after the answer, a Sign cut off
/// at 15 s would read as nothing in flight, and the chart would close over its outcome.
#[test]
fn a_restart_waits_for_acts_and_the_close() {
    let restart = js_fn(UPDATE_JS, "restartIfFits");
    let captured = position(
        restart,
        "const settling = signInFlight() ? signsSettled() : null;",
    );
    let asked = position(restart, "invoke(\"installed_program\")");
    // Both checks (second re-review): the act captured before the check, THEN any act begun while
    // the program answered (up to 5 s without the connection) — a one-click Stop, say. Either one
    // that did not settle well keeps the chart.
    let first = position(restart, "const first = await settling;");
    let first_kept = position(restart, "if (first && !SETTLED_WELL.has(first)) return;");
    let later = position(
        restart,
        "const later = signInFlight() ? await signsSettled() : null;",
    );
    let later_kept = position(restart, "if (later && !SETTLED_WELL.has(later)) return;");
    let closed = position(restart, "await closeChart();");
    let restarted = position(restart, "invoke(\"restart_window\")");
    assert!(
        captured < asked
            && asked < first
            && first < first_kept
            && first_kept < later
            && later < later_kept
            && later_kept < closed
            && closed < restarted,
        "{restart}"
    );
    // The same test as leave_chart.js's own (#1130).
    let find = js_fn(LEAVE_JS, "findAnotherPatient");
    assert!(find.contains("if (outcome && !SETTLED_WELL.has(outcome)) return;"));
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
        fits < position(restart, "const first = await settling;"),
        "{restart}"
    );
    assert!(fits < position(restart, "await closeChart();"), "{restart}");
}

/// Review Focus 3: at the lock screen the poll restarts only after a whole idle period without a
/// touch, so someone typing the terminal passphrase is never restarted under; and never before
/// the lock poll has said how long that period is. The installed program can take 5 s to answer,
/// so the poll's restart asks again after the answer (fix round 1).
#[test]
fn the_lock_screen_restart_needs_a_full_idle_period() {
    let idle = js_fn(UPDATE_JS, "idleForAWholePeriod");
    assert!(
        idle.contains("return idleLockMs > 0 && Date.now() - lastActivityAt >= idleLockMs;"),
        "{idle}"
    );
    let poll = js_fn(UPDATE_JS, "pollUpdate");
    for needed in [
        "!el(\"terminal-lock\").hidden",
        "idleForAWholePeriod()",
        "updateView.kind !== \"current\"",
    ] {
        assert!(
            position(poll, needed) < position(poll, "restartIfFits(idleForAWholePeriod);"),
            "{needed}"
        );
    }
    let restart = js_fn(UPDATE_JS, "restartIfFits");
    assert!(restart.contains("function restartIfFits(stillWanted = () => true) {"));
    let asked = position(restart, "invoke(\"installed_program\")");
    let again = position(restart, "if (!updatePressed && !stillWanted()) return;");
    assert!(asked < again, "{restart}");
    assert!(
        again < position(restart, "if (installed.kind !== \"fits\") {"),
        "{restart}"
    );
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

/// Fix round 2: `unreachable` (the node did not answer just now) is said and returns before
/// anything closes: restarting into an outage relaunches a window that exits at its first connect.
#[test]
fn unreachable_closes_nothing() {
    let said = js_const(UPDATE_JS, "UPDATE_NOT_NOW");
    assert!(
        UPDATE_JS.contains("  unreachable: UPDATE_UNREACHABLE,\n"),
        "{said}"
    );
    let restart = js_fn(UPDATE_JS, "restartIfFits");
    let branch = position(restart, "if (installed.kind !== \"fits\") {");
    let say = position(
        restart,
        "sayUpdate(UPDATE_NOT_NOW[installed.kind] ?? UPDATE_UNREACHABLE);",
    );
    let leave = branch + position(&restart[branch..], "return;");
    assert!(branch < say && say < leave, "{restart}");
    assert!(
        leave < position(restart, "await closeChart();"),
        "{restart}"
    );
}

/// Fix round 2: an Update now press made while the idle poll's attempt waits on the installed
/// program joins that attempt instead of being dropped: the press is also a touch, which would
/// otherwise make the poll's own idleness re-check give up.
#[test]
fn a_press_during_an_idle_attempt_counts() {
    let now = js_fn(UPDATE_JS, "updateNow");
    assert!(
        position(now, "updatePressed = true;") < position(now, "await restartIfFits();"),
        "{now}"
    );
    let restart = js_fn(UPDATE_JS, "restartIfFits");
    assert!(
        position(restart, "invoke(\"installed_program\")")
            < position(restart, "if (!updatePressed && !stillWanted()) return;"),
        "{restart}"
    );
    assert!(
        position(restart, "} finally {") < position(restart, "updatePressed = false;"),
        "{restart}"
    );
    assert!(UPDATE_JS.contains("let updatePressed = false;"));
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
    let relocked = js_fn(UPDATE_JS, "updateAfterRelock");
    assert!(relocked.contains("updateView.kind !== \"current\""));
    // Final review M1: a touch during the installed program's answer (up to 5 s) after the
    // re-lock means someone is back at the lock, so the restart gives up as the poll's does.
    assert!(
        relocked.contains("restartIfFits(idleForAWholePeriod)"),
        "{relocked}"
    );
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

/// Final review M2: a refusal learned mid-session (the database restored to an older
/// generation, say) shows the refusal's own sentence, never the below-floor copy, which would
/// claim an update is what this terminal needs.
#[test]
fn a_refusal_shows_its_own_sentence() {
    let render = js_fn(UPDATE_JS, "renderUpdate");
    assert!(
        render
            .contains("const alert = below ? view.refusal ?? belowFloorText(view.reason) : \"\";"),
        "{render}"
    );
}

/// Final review M3: a window that cannot name its own program's path (macOS, a path through a
/// symlink) says it cannot check, and restarts nothing.
#[test]
fn an_unknown_install_says_so() {
    let said = js_const(UPDATE_JS, "UPDATE_NOT_NOW");
    assert!(
        UPDATE_JS.contains("  unknown_install: UPDATE_UNKNOWN_INSTALL,\n"),
        "{said}"
    );
}

/// #909 slice C2: the courier's note (fetching, or the installer's refusal) sits INSIDE the update
/// line, which is shown whenever there is a note, even with no update waiting; then only the note
/// shows (no calm line, no Update now: nothing is installed to restart into). Visible is pinned,
/// not just its text: the note's ancestors are the line alone, the line is un-hidden by the note,
/// no style rule hides the note, and the sentence is set as text, never HTML.
#[test]
fn a_refused_install_shows_its_sentence() {
    let line = &INDEX_HTML[position(INDEX_HTML, "<div id=\"update-line\" hidden>")..];
    let line = &line[..position(line, "</div>")];
    assert!(
        line.contains("<p id=\"update-note\" role=\"status\" hidden></p>"),
        "{line}"
    );
    let render = js_fn(UPDATE_JS, "renderUpdate");
    for shown in [
        "el(\"update-line\").hidden = view.kind === \"current\" && !view.note;",
        "el(\"update-note\").hidden = !view.note;",
        "el(\"update-calm\").hidden = below || view.kind === \"current\";",
        "el(\"update-now\").hidden = view.kind === \"current\";",
        "if (el(\"update-note\").textContent !== note) el(\"update-note\").textContent = note;",
    ] {
        assert!(render.contains(shown), "{shown}\n{render}");
    }
    assert!(!UPDATE_JS.contains("innerHTML"), "text, never HTML");
    let css = include_str!("../src-ui/style.css");
    assert!(
        !css.contains("#update-note"),
        "no rule may hide or restyle the note"
    );
    // The copy itself is Rust's (`crate::release_fetch`), and `installer_view` puts it in `note`.
    assert_eq!(
        crate::release_fetch::refused_note("{sentence}"),
        "This terminal did not install the update: {sentence}"
    );
}
