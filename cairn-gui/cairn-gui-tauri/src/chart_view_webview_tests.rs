//! Guards on the webview's half of the chart re-read: the generation token (#800), the typed
//! stop reasons a re-read keeps (#799), and a medication list that fails to DRAW (#801).
//!
//! Every fix here lives in JavaScript (`main.js`, `duplicates.js`), and the webview has no test
//! rig of its own (no npm, by design: `docs/developers/durable-rules.md`). So, like
//! `duplicates/webview_tests.rs`, each guard reads the script's source (`crate::source_guard`)
//! and pins the ORDER or the CHOICE the fix is. Reverting any of these fixes leaves every
//! behavioural test green; these turn red. The behaviour itself was walked headlessly (a stub
//! `invoke` in a browser) when the fixes were written — see the PR that added this file.
use crate::source_guard::{js_const, js_fn, position};

const MAIN_JS: &str = include_str!("../src-ui/main.js");
const DUPLICATES_JS: &str = include_str!("../src-ui/duplicates.js");
const INDEX_HTML: &str = include_str!("../src-ui/index.html");

/// The one test every chart read's answer passes before it is drawn: it is for the chart on
/// screen, AND no newer read (or chart change) has started since. Checking the patient alone let
/// an older answer for the SAME chart land over a fresher one — two overlapping re-reads, or the
/// clinician leaving chart A and opening it again (#800).
#[test]
fn a_read_is_current_only_for_the_chart_on_screen_and_the_newest_read() {
    let current = js_fn(MAIN_JS, "isCurrentRead");
    assert!(
        current.contains("patient === displayedPatient"),
        "{current}"
    );
    assert!(
        current.contains("revision === chartReadRevision"),
        "{current}"
    );
}

/// #800. `refresh` takes its revision BEFORE it sends the read, and both the answer and the
/// failure are dropped once a newer read has started — a superseded failure must not say
/// "Could not read the chart" over a list a newer read has drawn.
#[test]
fn a_chart_read_is_drawn_only_while_it_is_the_newest() {
    let refresh = js_fn(MAIN_JS, "refresh");
    let sent = position(refresh, "await invoke(\"med_list\"");
    assert!(
        position(refresh, "const revision = ++chartReadRevision;") < sent,
        "{refresh}"
    );
    let caught_at = position(refresh, "catch (e)");
    let answered = &refresh[sent..caught_at];
    assert!(
        position(answered, "if (!isCurrentRead(patient, revision)) return;")
            < position(answered, "render(answer, patient);"),
        "{refresh}"
    );
    let caught = &refresh[caught_at..];
    assert!(
        position(caught, "if (!isCurrentRead(patient, revision)) return;")
            < position(caught, "say("),
        "{refresh}"
    );
    // The patient-only test is gone, not kept beside the new one as a second spelling.
    assert!(
        !refresh.contains("patient !== displayedPatient"),
        "{refresh}"
    );
}

/// #800's reopen case. Leaving a chart (or opening one) outdates every read still in flight, so an
/// answer from the previous visit — even for the same chart, A → front door → A — is never drawn.
#[test]
fn a_chart_change_outdates_every_read_in_flight() {
    let clear = js_fn(MAIN_JS, "clearChart");
    assert!(clear.contains("chartReadRevision += 1;"), "{clear}");
}

/// #800, the banner's half. A banner read alone (after a failed list read, or a decided refusal)
/// notes the revision before it sends, and is dropped once a newer chart read has started: that
/// read draws a fresher banner. It must NOT bump the revision itself — that would drop a whole
/// chart read already in flight, leaving its list undrawn.
#[test]
fn a_banner_read_alone_is_dropped_once_a_newer_chart_read_started() {
    let alone = js_fn(DUPLICATES_JS, "redrawBannerAlone");
    let sent = position(alone, "await invoke(\"duplicate_banner\"");
    assert!(
        position(alone, "const revision = chartReadRevision;") < sent,
        "{alone}"
    );
    let after = &alone[sent..];
    assert!(
        position(after, "if (!isCurrentRead(patient, revision)) return;")
            < position(after, "drawDuplicates(section);"),
        "{alone}"
    );
    assert!(!alone.contains("++chartReadRevision"), "{alone}");
    assert!(!alone.contains("chartReadRevision +="), "{alone}");
}

/// #799. A re-read rebuilds every row, so the stop reasons typed but not yet sent are read out
/// BEFORE the rows are emptied and written back AFTER they are rebuilt — with focus and caret, so
/// a clinician mid-word keeps typing. Re-typing a reason is work paper never loses (§1.2).
#[test]
fn a_re_read_keeps_every_typed_stop_reason_whose_line_survives() {
    let draw = js_fn(MAIN_JS, "drawList");
    let kept = position(draw, "const typed = typedReasons();");
    let emptied = position(draw, "body.replaceChildren();");
    let rebuilt = position(draw, "renderRow(row)");
    let restored = position(draw, "restoreReasons(typed);");
    assert!(
        kept < emptied && emptied < rebuilt && rebuilt < restored,
        "{draw}"
    );

    let typed = js_fn(MAIN_JS, "typedReasons");
    assert!(typed.contains("[id^=\"reason-\"]"), "{typed}");
    assert!(typed.contains("document.activeElement"), "{typed}");
    let restore = js_fn(MAIN_JS, "restoreReasons");
    for part in [".value =", ".focus()", "setSelectionRange"] {
        assert!(restore.contains(part), "{part} in {restore}");
    }
}

/// #801. The banner is drawn first, under its own guard (`drawDuplicates`); everything else
/// `render` draws — the linked charts, the links, the warnings, the rows and sign-off — is drawn
/// inside ONE guard, so a fault part-way never leaves a partial list under a live "Sign off N",
/// and never escapes into `refresh`'s "Could not read the chart" (the chart WAS read).
#[test]
fn every_part_of_the_list_is_drawn_inside_one_guard() {
    let render = js_fn(MAIN_JS, "render");
    let tried_at = position(render, "try {");
    assert!(
        position(render, "drawDuplicates(pane.duplicates)") < tried_at,
        "{render}"
    );
    let caught_at = position(render, "catch (failure)");
    assert!(
        render[tried_at..caught_at].contains("drawList(pane);"),
        "{render}"
    );
    let caught = &render[caught_at..];
    assert!(caught.contains("showUnshownList(failure);"), "{render}");
    assert!(!caught.contains("throw"), "{render}");

    let draw = js_fn(MAIN_JS, "drawList");
    for drawer in [
        "renderMembers(",
        "renderLinks(",
        "renderWarnings(",
        "renderRow(",
        "el(\"sign-off\")",
    ] {
        assert!(draw.contains(drawer), "{drawer} in {draw}");
        assert!(
            !render.contains(drawer),
            "{drawer} outside the guard: {render}"
        );
    }
    // A list that DOES draw takes down a previous fault's warning.
    assert!(
        draw.contains("setMessage(el(\"chart-unshown\"), \"\");"),
        "{draw}"
    );
}

/// #801. On a drawing fault nothing of the list stays up (it may be the partial one), sign-off is
/// disabled and says why, the fault goes to the console with its stack, and the clinician is told
/// in the chart's warnings — ABOVE the table, so no list can look complete before its warning —
/// never on the outcome line, where it would overwrite a signed act's outcome.
#[test]
fn a_list_that_fails_to_draw_leaves_nothing_partial_and_no_live_sign_off() {
    let shown = js_fn(MAIN_JS, "showUnshownList");
    assert!(shown.contains("console.error(failure);"), "{shown}");
    assert!(
        position(shown, "clearDrawnList();")
            < position(shown, "setMessage(el(\"chart-unshown\"), LIST_NOT_SHOWN"),
        "{shown}"
    );
    assert!(
        shown.contains("el(\"chart-warnings\").hidden = false;"),
        "{shown}"
    );
    assert!(shown.contains("button.disabled = true;"), "{shown}");
    assert!(!shown.contains("say("), "{shown}");

    let cleared = js_fn(MAIN_JS, "clearDrawnList");
    for part in [
        "renderMembers([])",
        "renderLinks([])",
        "el(\"med-rows\").replaceChildren()",
        "el(\"chart-warnings\").hidden = true",
        "setMessage(el(\"chart-unshown\"), \"\")",
    ] {
        assert!(cleared.contains(part), "{part} in {cleared}");
    }
    assert!(
        js_fn(MAIN_JS, "clearChart").contains("clearDrawnList();"),
        "one list of what a chart draws, not two"
    );

    let sentence = js_const(MAIN_JS, "LIST_NOT_SHOWN");
    assert!(sentence.contains("could not be shown"), "{sentence}");
    assert!(
        sentence.contains("nothing on this list can be signed off or stopped"),
        "{sentence}"
    );

    let warnings = position(INDEX_HTML, "<section id=\"chart-warnings\"");
    let unshown = position(INDEX_HTML, "<p id=\"chart-unshown\"");
    assert!(warnings < unshown, "inside the warnings section");
    assert!(
        unshown < position(INDEX_HTML, "<table id=\"med-table\">"),
        "above the list"
    );
}
