//! #1130 + #1137: leaving a chart (`leave_chart.js`). A chart leaves the screen only once every
//! act in flight on it has settled, so an act's answer is said on the chart it acted on, never on
//! the next patient's; and an open never overtakes a close still in progress.
//!
//! The window's scripts have no test rig (no npm or bundler: main.js's header, issue #332), so,
//! like `whos_here_webview_tests.rs`, each guard reads a script's source (`crate::source_guard`)
//! and pins the ORDER or the CHOICE a property is. The behaviour was walked headlessly (a stub
//! `invoke` in a browser) when it was written — see the PR that added this file.
use crate::source_guard::{js_fn, position};

const LEAVE_JS: &str = include_str!("../src-ui/leave_chart.js");
const NOTES_JS: &str = include_str!("../src-ui/notes.js");
const MAIN_JS: &str = include_str!("../src-ui/main.js");
const HOLDS_JS: &str = include_str!("../src-ui/holds.js");
const ALLERGIES_JS: &str = include_str!("../src-ui/allergies.js");
const MENU_JS: &str = include_str!("../src-ui/person_menu.js");
const FUNNEL_JS: &str = include_str!("../src-ui/funnel.js");
const OVERLAYS_JS: &str = include_str!("../src-ui/allergy_overlays.js");
const INDEX_HTML: &str = include_str!("../src-ui/index.html");

/// Every `src-ui/*.js` file, as (name, source): a guard over them all covers a new script too.
pub(crate) fn scripts() -> Vec<(String, String)> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src-ui");
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("src-ui is readable") {
        let path = entry.expect("a directory entry").path();
        if path.extension().is_some_and(|e| e == "js") {
            let name = path
                .file_name()
                .expect("a name")
                .to_string_lossy()
                .into_owned();
            out.push((
                name,
                std::fs::read_to_string(&path).expect("a readable script"),
            ));
        }
    }
    out
}

/// #1130: the close waits for every act in flight BEFORE it takes anything down, so a Withhold
/// pressed on A is said under A's header and its re-read is of A — never "Withheld" under B's,
/// and never A's "not known" feeding B's stale-list state. The wait is the close's FIRST step:
/// `displayedPatient = null` before it would make the act's answer think its chart was left.
#[test]
fn a_close_waits_for_every_act_in_flight_before_it_takes_the_chart_down() {
    let leave = js_fn(LEAVE_JS, "leaveChart");
    let waited = position(leave, "await signsSettled();");
    for step in [
        "displayedPatient = null;",
        "clearChart();",
        "invoke(\"close_chart\")",
    ] {
        assert!(waited < position(leave, step), "{step}: {leave}");
    }
}

/// #1137: a second close joins the one in progress (two would interleave their saves, and the
/// second's `close_chart` could close the NEXT chart on the backend), and is forgotten once done.
#[test]
fn a_close_in_progress_is_joined_never_run_twice() {
    let close = js_fn(LEAVE_JS, "closeChart");
    assert!(
        close.contains("closing ??= leaveChart(menuReadFollows).finally(() => {"),
        "{close}"
    );
    assert!(close.contains("closing = null;"), "{close}");
    assert!(close.contains("return closing;"), "{close}");
    assert!(LEAVE_JS.contains("let closing = null;"), "{LEAVE_JS}");
}

/// #1137: an open waits for a close still in progress. The who's-here bar is never inert, so the
/// person menu can be clicked while `closeChart` waits on A's saves; opening B then would refuse
/// A's remaining saves as "not the chart that is open" (text lost without a word), and A's resumed
/// `close_chart` would close B on the backend.
#[test]
fn an_open_waits_for_a_close_in_progress() {
    let open = js_fn(MENU_JS, "openMenuRow");
    assert!(
        position(open, "await closing;")
            < position(
                open,
                "if (displayedPatient) await closeChart({ menuReadFollows: true });"
            ),
        "{open}"
    );
    let open_chart = js_fn(FUNNEL_JS, "openChart");
    assert!(
        position(open_chart, "await closing;") < position(open_chart, "invoke(\"open_chart\""),
        "{open_chart}"
    );
}

/// #1130: the acts that set no flag of their own join one window-wide set: sign-off and Stop
/// (main.js), Withhold and Release (holds.js), every allergy press (allergies.js's `pressOnce`).
/// Each says its outcome in a known place, which the act's words are read from once it settles —
/// `#outcome` for the first four; an allergy press hands back its own (the next guard), its status
/// line read only when it hands back none.
#[test]
fn every_chart_press_runs_as_an_act_in_flight() {
    assert!(
        MAIN_JS.contains("el(\"sign-off\").addEventListener(\"click\", () => void chartAct(signOff, \"outcome\"));"),
        "sign-off"
    );
    assert!(
        MAIN_JS.contains("void chartAct(() => cease(row.group_id, reason.value), \"outcome\")"),
        "Stop"
    );
    assert!(
        HOLDS_JS.contains("chartAct(() => withhold(row.group_id, whyHold.value, kind, untilAt.value), \"outcome\")"),
        "Withhold"
    );
    assert!(
        HOLDS_JS.contains("chartAct(\n        () => release("),
        "Release"
    );
    let press = js_fn(ALLERGIES_JS, "pressOnce");
    assert!(
        press.contains("await chartAct(act, \"allergy-status\");"),
        "{press}"
    );
    // And nothing else calls them bare: every press is the one wired above.
    for (src, call) in [
        (MAIN_JS, "signOff"),
        (MAIN_JS, "cease("),
        (HOLDS_JS, "withhold("),
        (HOLDS_JS, "release("),
    ] {
        let bare = src.matches(call).count() - src.matches(&format!("function {call}")).count();
        assert_eq!(bare, 1, "{call} has one caller, inside chartAct");
    }
}

/// One strip credential serves one act at a time (#1002): a chart act pressed while another act —
/// any of them, a note's included — is in flight does nothing, and answers `false` (Withhold's and
/// Release's "not spent", so their button comes back). The check is the act's first step.
#[test]
fn a_chart_act_never_starts_while_another_is_in_flight() {
    let act = js_fn(LEAVE_JS, "chartAct");
    assert!(
        position(act, "if (signInFlight()) return Promise.resolve(false);")
            < position(act, "run()"),
        "{act}"
    );
    let in_flight = js_fn(NOTES_JS, "signInFlight");
    assert!(
        in_flight.contains("noteSignInFlight || medListSignInFlight || actsInFlight.size > 0"),
        "{in_flight}"
    );
}

/// An allergy press says its outcome in one of several places — the status line, the strip (a sign
/// whose list did not come back), `#outcome` (a refusal behind the record, an outcome NOT known,
/// #814), under a field, in an overlay's row — so it hands back the words it said, and `chartAct`
/// takes them over its one element. Read from `#allergy-status` alone, a refused or unknown write
/// settled as "said nothing", and "Find another patient" left the chart that said it (PR #1142
/// review).
#[test]
fn an_allergy_press_hands_back_the_words_it_said() {
    let act = js_fn(LEAVE_JS, "chartAct");
    assert!(
        act.contains("typeof value === \"string\" ? value : el(saidIn).textContent"),
        "{act}"
    );
    let signed = js_fn(ALLERGIES_JS, "afterSigned");
    assert_eq!(
        signed.matches("return answer.outcome;").count(),
        2,
        "{signed}"
    );
    let refused = js_fn(ALLERGIES_JS, "afterRefused");
    // The behind-the-record re-read, the row's own line, and the field or status line.
    assert_eq!(refused.matches("return text;").count(), 3, "{refused}");
    for (src, name) in [
        (ALLERGIES_JS, "recordAllergy"),
        (ALLERGIES_JS, "recordNoneKnown"),
        (OVERLAYS_JS, "signOverlay"),
    ] {
        let body = js_fn(src, name);
        assert!(body.contains("return afterRefused("), "{name}: {body}");
        assert!(body.contains("return afterSigned("), "{name}: {body}");
    }
}

/// A chart act settles as the words it said (`{ words }`, which whos_here.js's picker repeats once
/// the switch has taken the chart away), or null when it said nothing — never a rejection, so a
/// switch's wait always ends. `signsSettled` answers a chart act's words while one is in flight.
#[test]
fn a_chart_act_settles_as_the_words_it_said() {
    let act = js_fn(LEAVE_JS, "chartAct");
    assert!(
        act.contains("const said = typeof value === \"string\" ? value : el(saidIn).textContent;"),
        "{act}"
    );
    assert!(
        act.contains("return said ? { words: said } : null;"),
        "{act}"
    );
    assert!(act.contains("return { words: CHART_ACT_FAULT };"), "{act}");
    assert!(act.contains("actsInFlight.add(settled);"), "{act}");
    assert!(act.contains("actsInFlight.delete(settled)"), "{act}");
    let settled = js_fn(NOTES_JS, "signsSettled");
    assert!(
        position(settled, "if (actsInFlight.size > 0)") < position(settled, "return noteSigning;"),
        "{settled}"
    );
    assert!(settled.contains("Promise.all(actsInFlight)"), "{settled}");
}

/// #1130's "dropped" half: "Find another patient" waits for an act in flight and leaves only when
/// it settled well (`SETTLED_WELL`: a note act that went through). A chart act's words — success
/// included, as the person menu does — keep the chart on screen: leaving would clear the only place
/// that says them. And it is the button's ONE listener: a second, bare `closeChart` would leave
/// regardless.
#[test]
fn find_another_patient_stays_unless_the_act_settled_well() {
    assert!(
        LEAVE_JS.contains(
            "el(\"close-chart\").addEventListener(\"click\", () => void findAnotherPatient());"
        ),
        "{LEAVE_JS}"
    );
    let find = js_fn(LEAVE_JS, "findAnotherPatient");
    let settle = position(
        find,
        "const outcome = signInFlight() ? await signsSettled() : null;",
    );
    let stay = position(find, "if (outcome && !SETTLED_WELL.has(outcome)) return;");
    let close = position(find, "await closeChart();");
    assert!(settle < stay && stay < close, "{find}");
    let listeners: usize = scripts()
        .iter()
        .map(|(_, js)| js.matches("el(\"close-chart\").addEventListener").count())
        .sum();
    assert_eq!(listeners, 1, "one listener on #close-chart");
}

/// The single choke point that makes per-answer "is this still my chart?" checks unneeded: only
/// `leaveChart` takes a chart off the screen or closes it on the backend, and `enterChart` is only
/// reached with no chart displayed (the front door, the boot, the person menu after its close).
#[test]
fn only_leave_chart_takes_a_chart_off_screen() {
    let mut closes = Vec::new();
    let mut clears = Vec::new();
    for (name, js) in scripts() {
        for _ in 0..js.matches("invoke(\"close_chart\")").count() {
            closes.push(name.clone());
        }
        let declared = js.matches("let displayedPatient = null").count(); // main.js's declaration
        for _ in declared..js.matches("displayedPatient = null").count() {
            clears.push(name.clone());
        }
    }
    assert_eq!(closes, ["leave_chart.js"]);
    assert_eq!(clears, ["leave_chart.js"]);
    // Loaded after funnel.js (whose `resetFrontDoor` it calls) and the scripts whose state it reads.
    let tag = position(INDEX_HTML, "<script src=\"leave_chart.js\"></script>");
    assert!(position(INDEX_HTML, "<script src=\"funnel.js\"></script>") < tag);
    assert!(
        !FUNNEL_JS.contains("function closeChart("),
        "moved to leave_chart.js"
    );
}

/// The other way a chart leaves the screen is another one replacing it: `enterChart` sets
/// `displayedPatient` and clears the chart without waiting for anything. So every caller is placed:
/// funnel.js's are the front door's (a row, a registration, the boot) and `openChart` itself; the
/// tray (worklist.js) and who's-here's return button sit inside `#front-door`; the person menu
/// opens after `await closeChart()`. A new caller fails here until it is placed — one reachable
/// from the chart view must close first.
#[test]
fn every_chart_opener_is_placed() {
    let mut found: Vec<(String, usize)> = scripts()
        .into_iter()
        .map(|(name, js)| {
            let calls = js.matches("enterChart(").count() + js.matches("openChart(").count()
                - js.matches("function enterChart(").count()
                - js.matches("function openChart(").count();
            (name, calls)
        })
        .filter(|(_, calls)| *calls > 0)
        .collect();
    found.sort();
    let expected: Vec<(String, usize)> = [
        ("funnel.js", 4),
        ("person_menu.js", 1),
        ("whos_here.js", 1),
        ("worklist.js", 1),
    ]
    .iter()
    .map(|(n, c)| (n.to_string(), *c))
    .collect();
    assert_eq!(found, expected);
    let menu = js_fn(MENU_JS, "openMenuRow");
    assert!(
        position(
            menu,
            "if (displayedPatient) await closeChart({ menuReadFollows: true });"
        ) < position(menu, "enterChart(header);"),
        "{menu}"
    );
}

/// Every press that reads the strip's credential, by script. A new one fails here until it is
/// placed: a chart act joins the in-flight set (`chartAct`, or the note's and the medication
/// list's own flags), or it is named as not a chart act. Counts, so a second press in a file
/// already listed is caught too.
#[test]
fn every_credential_reader_is_placed() {
    // Joins the window-wide in-flight set: `chartAct`, or notes.js's / med_list_sign.js's flags.
    let in_flight = [
        ("allergies.js", 2),
        ("allergy_overlays.js", 1),
        ("holds.js", 2),
        ("main.js", 2),
        ("med_list_sign.js", 1),
        ("med_stat.js", 2),
        ("med_versions_settle.js", 1),
        ("notes.js", 1),
        ("notes_compound.js", 2),
        ("notes_discard.js", 1),
    ];
    // Not yet: link and unlink say a late answer through link.js's own `sayAnywhere` (#709, and
    // #1002's credential side for them); the rest are not chart acts (enrolment, re-key, the
    // superuser and supervision panels, run from the front door's bar).
    let outside = [
        ("enrol.js", 1),
        ("link.js", 1),
        ("rekey.js", 1),
        ("superuser.js", 1),
        ("supervision.js", 1),
        ("unlink.js", 2),
    ];
    let mut expected: Vec<(String, usize)> = in_flight
        .iter()
        .chain(outside.iter())
        .map(|(n, c)| (n.to_string(), *c))
        .collect();
    expected.sort();
    let mut found: Vec<(String, usize)> = scripts()
        .into_iter()
        .filter(|(name, _)| name != "signer.js") // where it is defined
        .map(|(name, js)| {
            let count = js.matches("signerCredential()").count();
            (name, count)
        })
        .filter(|(_, count)| *count > 0)
        .collect();
    found.sort();
    assert_eq!(found, expected);
}
