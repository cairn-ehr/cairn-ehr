//! Guards on the webview's half of the chart re-read: the generation token (#800), the typed
//! stop reasons a re-read keeps (#799), and a medication list that fails to DRAW (#801).
//!
//! Every fix here lives in the webview (`main.js`, `stop_reasons.js`, `duplicates.js`,
//! `index.html`), which has no test rig of its own (no npm or bundler: main.js's header, issue
//! #332). So, like
//! `duplicates/webview_tests.rs`, each guard reads the script's source (`crate::source_guard`)
//! and pins the ORDER or the CHOICE the fix is. Reverting any of these fixes leaves every
//! behavioural test green; these turn red. The behaviour itself was walked headlessly (a stub
//! `invoke` in a browser) when the fixes were written — see the PR that added this file.
use crate::source_guard::{js_const, js_fn, position};

const MAIN_JS: &str = include_str!("../src-ui/main.js");
const STOP_REASONS_JS: &str = include_str!("../src-ui/stop_reasons.js");
const DUPLICATES_JS: &str = include_str!("../src-ui/duplicates.js");
const INDEX_HTML: &str = include_str!("../src-ui/index.html");

/// The two questions every chart read asks (#800), each pinned as its EXACT expression — a test
/// that only looked for both comparisons would pass `&&` mutated to `||`, which brings #800 back.
///
/// - `mayDraw`: an answer is drawn while its chart is on screen AND nothing newer has been DRAWN
///   (`drawnRevision`). Not "nothing newer STARTED": then the older of two overlapping re-reads,
///   landing first as it usually does, was dropped, and if the newer then failed the screen kept a
///   list older than both (PR #802 review).
/// - `isLatestRead`: a FAILURE is reported only while no newer read has STARTED — that read
///   speaks for the chart now, and must not be pre-empted by an older read's "Could not read".
#[test]
fn a_read_draws_unless_something_newer_is_drawn_and_fails_aloud_only_if_newest() {
    let may_draw = js_fn(MAIN_JS, "mayDraw");
    assert!(
        may_draw.contains("return patient === displayedPatient && revision >= drawnRevision;"),
        "{may_draw}"
    );
    let latest = js_fn(MAIN_JS, "isLatestRead");
    assert!(
        latest.contains("return patient === displayedPatient && revision === chartReadRevision;"),
        "{latest}"
    );
}

/// #800. `refresh` takes its revision BEFORE it sends the read; an answer passes `mayDraw` and
/// records itself as the newest drawn before it renders; a failure passes `isLatestRead` before
/// it says anything.
#[test]
fn a_chart_read_is_drawn_only_while_nothing_newer_is_on_screen() {
    let refresh = js_fn(MAIN_JS, "refresh");
    let sent = position(refresh, "await invoke(\"med_list\"");
    assert!(
        position(refresh, "const revision = ++chartReadRevision;") < sent,
        "{refresh}"
    );
    let caught_at = position(refresh, "catch (e)");
    let answered = &refresh[sent..caught_at];
    let checked = position(answered, "if (!mayDraw(patient, revision)) return;");
    let recorded = position(answered, "drawnRevision = revision;");
    assert!(
        checked < recorded && recorded < position(answered, "render(answer, patient);"),
        "{refresh}"
    );
    let caught = &refresh[caught_at..];
    assert!(
        position(caught, "if (!isLatestRead(patient, revision)) return;")
            < position(caught, "say("),
        "{refresh}"
    );
    // The patient-only test is gone, not kept beside the new ones as a second spelling.
    assert!(
        !refresh.contains("patient !== displayedPatient"),
        "{refresh}"
    );
}

/// #800's reopen case. A chart change starts a new floor: `clearChart` bumps the revision and
/// makes it the newest "drawn", so no answer from before — even for the same chart, A → front
/// door → A — can pass `mayDraw` or `isLatestRead` again. This floor is load-bearing, not belt
/// and braces: `mayDraw` asks only "is anything newer DRAWN?", so without it a first-visit read
/// newer than the first visit's last drawn list would be drawn on the second visit while the
/// second visit's own read is still in flight.
#[test]
fn a_chart_change_outdates_every_read_in_flight() {
    let clear = js_fn(MAIN_JS, "clearChart");
    assert!(
        clear.contains("drawnRevision = ++chartReadRevision;"),
        "{clear}"
    );
}

/// #800, the banner's half. A banner read alone (after a failed list read, or a decided refusal)
/// notes the revision before it sends and is drawn only if no NEWER chart read has been drawn
/// since (`mayDraw`). It must NOT bump the revision itself — that would make a whole-chart read
/// already in flight fail `isLatestRead`, so a failure of that read would go unsaid.
#[test]
fn a_banner_read_alone_is_dropped_once_a_newer_chart_read_is_drawn() {
    let alone = js_fn(DUPLICATES_JS, "redrawBannerAlone");
    let sent = position(alone, "await invoke(\"duplicate_banner\"");
    assert!(
        position(alone, "const revision = chartReadRevision;") < sent,
        "{alone}"
    );
    let after = &alone[sent..];
    assert!(
        position(after, "if (!mayDraw(patient, revision)) return;")
            < position(after, "drawDuplicates(section);"),
        "{alone}"
    );
    assert!(!alone.contains("++chartReadRevision"), "{alone}");
    assert!(!alone.contains("chartReadRevision +="), "{alone}");
    assert!(!alone.contains("drawnRevision ="), "{alone}");
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

    let typed = js_fn(STOP_REASONS_JS, "typedReasons");
    assert!(typed.contains("[id^=\"reason-\"]"), "{typed}");
    assert!(typed.contains("document.activeElement"), "{typed}");
    let restore = js_fn(STOP_REASONS_JS, "restoreReasons");
    for part in [".value =", ".focus()", "setSelectionRange"] {
        assert!(restore.contains(part), "{part} in {restore}");
    }
}

/// #801. Everything `render` draws but the banner — the drawn chart and set, the linked charts,
/// the links, the warnings, the rows and sign-off — is drawn inside ONE guard, so a fault part-way
/// never leaves a partial list under a live "Sign off N" and never escapes into `refresh`'s "Could
/// not read the chart" (the chart WAS read). The banner is drawn AFTER it, under its own guard
/// (`drawDuplicates`), so it sees whether a list is on screen: its Review is offered only then.
#[test]
fn every_part_of_the_list_is_drawn_inside_one_guard() {
    let render = js_fn(MAIN_JS, "render");
    let tried_at = position(render, "try {");
    let caught_at = position(render, "catch (failure)");
    assert!(
        render[tried_at..caught_at].contains("drawList(pane, patient);"),
        "{render}"
    );
    // The catch block alone: from `catch` to the banner's line after it.
    let caught = &render[caught_at..position(render, "drawDuplicates(")];
    assert!(caught.contains("showUnshownList(failure);"), "{render}");
    assert!(!caught.contains("throw"), "{render}");
    assert!(
        position(render, "showUnshownList(failure);")
            < position(render, "drawDuplicates(pane.duplicates);"),
        "the banner is drawn after the list: {render}"
    );

    let draw = js_fn(MAIN_JS, "drawList");
    for drawer in [
        "renderedPatient = patient;",
        "renderedCharts = view.charts;",
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
/// in the chart's warnings — ABOVE the table, so it is read before the (now empty) list — never
/// on the outcome line, where it would overwrite a signed act's outcome.
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
    assert!(
        shown.contains(
            "button.textContent = \"Nothing to sign off — the list could not be shown\";"
        ),
        "{shown}"
    );
    assert!(!shown.contains("outcome"), "{shown}");
    // No list on screen, so nothing may compare or sign over one: the banner drawn next sees
    // `renderedPatient` null and offers no Review (`REVIEW_WAITS_FOR_LIST`), and `compare` refuses
    // (`NO_LIST_TO_COMPARE`) — the rule a failed or in-flight read already keeps (PR #802 review).
    for part in ["renderedPatient = null;", "renderedCharts = null;"] {
        assert!(shown.contains(part), "{part} in {shown}");
    }
    assert!(!shown.contains("say("), "{shown}");

    let cleared = js_fn(MAIN_JS, "clearDrawnList");
    for part in [
        "renderMembers([])",
        "renderLinks([])",
        "el(\"med-rows\").replaceChildren()",
        "el(\"chart-warnings\").hidden = true",
        "setMessage(el(\"chart-unshown\"), \"\")",
        // #797: a list taken down takes its out-of-date warning with it.
        "setMessage(el(\"chart-stale\"), \"\")",
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

/// `sign_off` and `cease` fail with an ErrorView object (`{text, retry}`), not a string.
/// Concatenating that object into a sentence would show the clinician "[object Object]" in the
/// one place that says why their act did not happen, so both catches must word it through
/// `failureText` (`funnel.js`), keeping their own prefixes.
#[test]
fn a_failed_sign_off_or_cease_is_worded_from_its_error_view() {
    let sign_off = js_fn(MAIN_JS, "signOff");
    assert!(sign_off.contains("\"Sign-off failed: \" + failureText(failure)"));
    let cease = js_fn(MAIN_JS, "cease");
    assert!(cease.contains("\"Could not stop this drug: \" + failureText(failure)"));
    assert!(!sign_off.contains("failed: \" + e)"));
    assert!(!cease.contains("this drug: \" + e)"));
}
