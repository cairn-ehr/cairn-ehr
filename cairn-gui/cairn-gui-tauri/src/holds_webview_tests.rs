//! Guards on the webview's half of the medication withhold (#832, ADR-0081): the "W" drawn on a
//! line, the reason it reveals, and the Withhold and Release gestures.
//!
//! The webview (`holds.js`, wired in by `main.js`, `stop_reasons.js` and `index.html`) has no test
//! rig of its own (no npm or bundler: main.js's header, issue #332). So, like
//! `chart_view_webview_tests.rs`, each guard reads the script's source (`crate::source_guard`) and
//! pins the exact expression or the ORDER a rule depends on, so the plausible mutation turns it
//! red. What each gesture SAYS is worded in Rust (`hold::outcome`) and tested there; the
//! behaviour itself was walked headlessly (a stub `invoke` in a browser) when it was written.
use crate::source_guard::{js_fn, position};

const HOLDS_JS: &str = include_str!("../src-ui/holds.js");
const MAIN_JS: &str = include_str!("../src-ui/main.js");
const STOP_REASONS_JS: &str = include_str!("../src-ui/stop_reasons.js");
const INDEX_HTML: &str = include_str!("../src-ui/index.html");

/// The W is a real button whose accessible name is its reason (Rust's `hold.detail`, which is
/// also what it reveals), so a screen reader hears WHY on reaching it. The reveal works on focus
/// (keyboard) AND on click alone: WebKit, Tauri's webview on macOS, does not focus a clicked
/// button, so a focus-only reveal would never open for a mouse user there. A ceased line's
/// reason rides the same button, on its status word.
#[test]
fn the_w_is_a_button_whose_label_is_its_reason() {
    let status = js_fn(HOLDS_JS, "statusCell");
    for part in [
        "document.createElement(\"button\")",
        "button.setAttribute(\"aria-label\", label);",
        "button.setAttribute(\"aria-controls\", id);",
        "button.setAttribute(\"aria-expanded\", String(open));",
        // The detail starts hidden; it is the button that shows it.
        "detail.hidden = true;",
        "detail.hidden = !open;",
        "button.addEventListener(\"focus\", () => show(true));",
        "button.addEventListener(\"mouseenter\", () => show(true));",
        // Leaving keeps a PINNED detail open: a mouse user who clicked to read it can move away.
        "button.addEventListener(\"blur\", () => show(pinned));",
        "button.addEventListener(\"mouseleave\", () => show(pinned));",
        "pinned = !pinned;",
        // The hold's label and revealed text are both its detail, never its symbol.
        "reveal(hold.symbol, hold.detail, hold.detail, \"hold-detail-\" + hold.withhold_event_id)",
        "\"ceased — \" + row.cessation_detail",
    ] {
        assert!(status.contains(part), "{part} in {status}");
    }
    let clicked = position(status, "button.addEventListener(\"click\"");
    assert!(
        status[clicked..].contains("show(pinned)"),
        "a click shows what it pinned: {status}"
    );
}

/// main.js draws the status cell through `statusCell`, marks a withheld line in the DOM (not by
/// colour alone) and appends the hold controls beside Stop; index.html loads holds.js before the
/// first chart can be drawn (funnel.js's boot) and names the column for both acts.
#[test]
fn the_row_draws_the_w_and_its_controls() {
    let row = js_fn(MAIN_JS, "renderRow");
    for part in [
        "statusCell(row),",
        "if (row.status_label === \"withheld\") tr.setAttribute(\"data-withheld\", \"true\");",
        "action.append(...holdControls(row));",
    ] {
        assert!(row.contains(part), "{part} in {row}");
    }
    assert!(
        !row.contains("cell(\"td\", row.status_label)"),
        "the bare status cell would hide every W: {row}"
    );
    let stops = position(INDEX_HTML, "<script src=\"stop_reasons.js\"></script>");
    let holds = position(INDEX_HTML, "<script src=\"holds.js\"></script>");
    let boot = position(INDEX_HTML, "<script src=\"funnel.js\"></script>");
    assert!(stops < holds && holds < boot, "{INDEX_HTML}");
    assert!(INDEX_HTML.contains("<th scope=\"col\">Stop or withhold</th>"));
}

/// A release ends exactly the holds this line DREW — the ids the clinician was shown (ADR-0081
/// decision 4; Rust's `plan::release_plan` refuses anything else). So the ids are taken from the
/// row the button was built for, and the only `invoke("release"` sends the ids it was handed.
#[test]
fn a_release_sends_only_the_holds_it_drew() {
    let controls = js_fn(HOLDS_JS, "holdControls");
    assert!(
        controls.contains(
            "release(row.group_id, row.holds.map((hold) => hold.withhold_event_id), whyRelease.value)"
        ),
        "{controls}"
    );
    let release = js_fn(HOLDS_JS, "release");
    assert!(release.contains("withholds: withholds,"), "{release}");
    for js in [HOLDS_JS, MAIN_JS] {
        let sends = js.matches("invoke(\"release\"").count();
        assert_eq!(sends, usize::from(js == HOLDS_JS), "one release call");
    }
    assert!(release.contains("invoke(\"release\""), "{release}");
}

/// "Until reviewed" is the default end (the open-ended paper W), and it is sent only when it was
/// the choice: "until a time" with no time, or a time the browser cannot read, is sent as `""`,
/// so Rust words the refusal ("choose when the hold ends") — never quietly an open-ended hold.
#[test]
fn until_reviewed_is_the_default_and_never_an_empty_time() {
    let controls = js_fn(HOLDS_JS, "holdControls");
    assert!(
        position(controls, "[\"reviewed\", \"until reviewed\"]")
            < position(controls, "[\"time\", \"until a time\"]"),
        "{controls}"
    );
    assert!(
        !controls.contains("selected"),
        "the first option is the default: {controls}"
    );
    let withhold = js_fn(HOLDS_JS, "withhold");
    assert!(
        withhold.contains("const at = new Date(local);"),
        "{withhold}"
    );
    assert!(
        withhold.contains(
            "const until = kind === \"reviewed\" ? \"reviewed\" : isNaN(at) ? \"\" : at.toISOString();"
        ),
        "{withhold}"
    );
}

/// Both gestures end exactly as `cease` does: a failure is SAID before the re-read and re-reads
/// flagged by `behindTheRecord(failure)` (so a failed re-read after an interrupted commit marks
/// the list out of date, #811); a success says Rust's line and re-reads with the report's mark.
#[test]
fn both_gestures_re_read_like_cease() {
    for (name, failed) in [
        (
            "withhold",
            "say(\"Could not withhold this drug: \" + failureText(failure));",
        ),
        (
            "release",
            "say(\"Could not release this hold: \" + failureText(failure));",
        ),
    ] {
        let body = js_fn(HOLDS_JS, name);
        let caught_at = position(body, "catch (failure)");
        let caught = &body[caught_at..caught_at + body[caught_at..].find("\n  }").unwrap()];
        assert!(
            position(caught, failed)
                < position(
                    caught,
                    "await refresh(el(\"outcome\").textContent, behindTheRecord(failure));"
                ),
            "{name}: {body}"
        );
        assert!(caught.contains("return;"), "{name}: {body}");
        let after = &body[caught_at + caught.len()..];
        assert!(
            position(after, "say(report.text);")
                < position(
                    after,
                    "await refresh(el(\"outcome\").textContent, report.behind_the_record);"
                ),
            "{name}: {body}"
        );
        for sent in ["patientId: renderedPatient,", "charts: renderedCharts,"] {
            assert!(body.contains(sent), "{name}: {sent}");
        }
    }
}

/// A re-read (#799) keeps the W reason, the until choice and the time a clinician has set but
/// not sent. A `<select>` and a `datetime-local` field have no caret — reading or setting one
/// throws or returns null — so the caret is recorded only for a text field and put back only when
/// one was recorded.
#[test]
fn a_re_read_keeps_the_until_choice_and_time() {
    let typed = js_fn(STOP_REASONS_JS, "typedReasons");
    assert!(typed.contains("[id^=\"until-\"]"), "{typed}");
    for part in [
        "field.type === \"text\" ? { start: field.selectionStart",
        "{ id: active.id, ...caret(active) }",
    ] {
        assert!(typed.contains(part), "{part} in {typed}");
    }
    let restore = js_fn(STOP_REASONS_JS, "restoreReasons");
    assert!(
        position(restore, "if (typeof typed.focused.start === \"number\")")
            < position(restore, "focused.setSelectionRange("),
        "{restore}"
    );
}

/// Typing a time IS choosing "until a time" (controller ruling on #832 task 5): a time typed while
/// the select still shows its default "until reviewed" would otherwise be dropped without a word
/// and the hold written open-ended — the clinician's stated end silently discarded (principle 4).
#[test]
fn typing_a_time_chooses_until_a_time() {
    let controls = js_fn(HOLDS_JS, "holdControls");
    let listens = position(controls, "untilAt.addEventListener(\"input\", () => {");
    assert!(
        controls[listens..].contains("untilKind.value = \"time\";"),
        "{controls}"
    );
}

/// Every control names its drug: six rows of an identical "Withhold until (local time)" are
/// ambiguous to a screen reader that reads the field out of table context.
#[test]
fn the_time_field_names_its_drug() {
    let controls = js_fn(HOLDS_JS, "holdControls");
    assert!(
        controls.contains(
            "untilAt.setAttribute(\"aria-label\", \"Withhold \" + row.primary + \" until (local time)\");"
        ),
        "{controls}"
    );
}

/// A gesture that RETURNED a report (recorded, or not known) clears what was typed for it BEFORE
/// the re-read: #799's `restoreReasons` would otherwise put the W reason back on a line that is
/// still current, and a second press would write a second W. A THROWN failure keeps the fields,
/// so the clinician can correct and retry (a refused empty time, say).
#[test]
fn a_returned_report_clears_the_line_before_the_re_read() {
    for (name, clears) in [
        (
            "withhold",
            &[
                "setField(\"reason-w-\" + groupId, \"\");",
                "setField(\"until-kind-\" + groupId, \"reviewed\");",
                "setField(\"until-at-\" + groupId, \"\");",
            ][..],
        ),
        ("release", &["setField(\"reason-r-\" + groupId, \"\");"][..]),
    ] {
        let body = js_fn(HOLDS_JS, name);
        let caught_at = position(body, "catch (failure)");
        let caught = &body[caught_at..caught_at + body[caught_at..].find("\n  }").unwrap()];
        assert!(
            !caught.contains("setField("),
            "{name} keeps a failure's fields: {body}"
        );
        let after = &body[caught_at + caught.len()..];
        let reread = position(after, "await refresh(");
        for clear in clears {
            assert!(position(after, clear) < reread, "{name}: {clear} in {body}");
        }
    }
    // A line a concurrent re-read redrew away has no field: clearing must not throw after a
    // write that landed, or the re-read below it never runs.
    let set = js_fn(HOLDS_JS, "setField");
    assert!(set.contains("if (field) field.value = value;"), "{set}");
}
