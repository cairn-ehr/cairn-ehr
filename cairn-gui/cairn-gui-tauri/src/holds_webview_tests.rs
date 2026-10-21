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
const STYLE_CSS: &str = include_str!("../src-ui/style.css");

/// The W is a real button whose accessible name is its visible text THEN its reason (WCAG 2.5.3,
/// label in name: a voice user says what they see), so a screen reader hears WHY on reaching it. The reveal works on focus
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
        "button.addEventListener(\"blur\", () => show(pinned()));",
        "button.addEventListener(\"mouseleave\", () => show(pinned()));",
        "button.dataset.pinned = String(!pinned());",
        // The label is the visible text first, then the reason; the reveal is the reason alone.
        "reveal(hold.symbol, hold.symbol + \" — \" + hold.detail, hold.detail, \"hold-detail-\" + hold.withhold_event_id)",
        "row.status_label + \" — \" + row.cessation_detail",
    ] {
        assert!(status.contains(part), "{part} in {status}");
    }
    let clicked = position(status, "button.addEventListener(\"click\"");
    assert!(
        status[clicked..].contains("show(pinned())"),
        "a click shows what it pinned: {status}"
    );
}

/// A pinned detail sits over the cells beside it (out of flow — see the CSS guard below), so it
/// can cover the same line's Stop/Withhold/Release and the line under it. Escape, or a click
/// anywhere outside it, puts it away. ONE document-level pair of listeners, registered at the top
/// level of holds.js — never inside `statusCell`, which runs on every redraw and would stack them.
#[test]
fn escape_or_a_click_elsewhere_unpins_a_detail() {
    let unpin = js_fn(HOLDS_JS, "unpinDetails");
    for part in [
        "if (target && (button.contains(target) || detail.contains(target))) continue;",
        "button.dataset.pinned = \"false\";",
        "detail.hidden = true;",
        "button.setAttribute(\"aria-expanded\", \"false\");",
    ] {
        assert!(unpin.contains(part), "{part} in {unpin}");
    }
    for listener in [
        "\ndocument.addEventListener(\"keydown\", (event) => {\n  if (event.key === \"Escape\") unpinDetails(null);",
        "\ndocument.addEventListener(\"click\", (event) => unpinDetails(event.target));",
    ] {
        assert_eq!(HOLDS_JS.matches(listener).count(), 1, "{listener}");
    }
    assert!(!js_fn(HOLDS_JS, "statusCell").contains("document.addEventListener"));
}

/// Revealed IN flow, the detail re-laid the table out, its button jumped from under the pointer,
/// `mouseleave` hid it, the button jumped back — a flicker loop no mouse could click through
/// (found walking #832). `position: absolute` is what keeps the reveal from moving anything.
#[test]
fn a_revealed_detail_moves_nothing() {
    let at = position(STYLE_CSS, "\n.hold-detail {");
    let rule = &STYLE_CSS[at..at + STYLE_CSS[at..].find("\n}").unwrap()];
    assert!(rule.contains("\n  position: absolute;"), "{rule}");
}

/// A withheld line that will be signed keeps BOTH left bars: the two single-attribute rules set
/// `box-shadow` on the same cell, so whichever comes later would erase the other (final review,
/// #832). A rule for a row carrying both draws them side by side.
#[test]
fn a_withheld_line_that_will_be_signed_keeps_both_bars() {
    let at = position(
        STYLE_CSS,
        "\ntr[data-will-sign=\"true\"][data-withheld=\"true\"] th[scope=\"row\"] {",
    );
    let rule = &STYLE_CSS[at..at + STYLE_CSS[at..].find("\n}").unwrap()];
    assert!(
        rule.contains("var(--accent)") && rule.contains("var(--warn-line)"),
        "{rule}"
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
    // The two words the styling compares are the words Rust sends: a reworded label must turn
    // this red, not silently drop the withheld (or ceased) bar.
    let view =
        cairn_gui_tab_medications::build_view(&cairn_medication_view::fixtures::sample_chart());
    for word in ["withheld", "ceased"] {
        assert!(
            view.rows.iter().any(|r| r.status_label == word),
            "the fixture chart has no {word} line: {view:?}"
        );
        assert!(
            row.contains(&format!("row.status_label === \"{word}\"")),
            "{word} in {row}"
        );
    }
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
    assert!(release.contains("invoke(\"release\""), "{release}");
    // ...and it is the only one in every script the webview has, so a new script is covered.
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src-ui");
    let mut sends = 0;
    for entry in std::fs::read_dir(&dir).expect("src-ui is readable") {
        let path = entry.expect("a directory entry").path();
        if path.extension().is_some_and(|e| e == "js") {
            let js = std::fs::read_to_string(&path).expect("a readable script");
            sends += js.matches("invoke(\"release\"").count();
        }
    }
    assert_eq!(sends, 1, "one release call in src-ui");
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
    // Only the field's own shape is read: a text-field fallback's "02/03/2026 08:00" would be
    // read month-first by `new Date`, a wrong end with no word said.
    assert!(
        withhold.contains("const at = LOCAL_TIME.test(local) ? new Date(local) : NaN;"),
        "{withhold}"
    );
    assert!(HOLDS_JS.contains(
        "const LOCAL_TIME = /^\\d{4}-\\d{2}-\\d{2}T\\d{2}:\\d{2}(:\\d{2}(\\.\\d+)?)?$/;"
    ));
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
        // Resolves false on a refusal (nothing written: the button comes back), true on a report.
        assert!(caught.contains("return false;"), "{name}: {body}");
        let after = &body[caught_at + caught.len()..];
        let reread = position(
            after,
            "await refresh(el(\"outcome\").textContent, report.behind_the_record);",
        );
        assert!(
            position(after, "say(report.text);") < reread
                && reread < position(after, "return true;"),
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
///
/// A PARTLY typed time (a date, no hour) is not caught by that listener: the field's value stays
/// `""` and fires no `input`. So the press itself reads the field: anything in it — a value, or
/// input the browser cannot read (`validity.badInput`) — makes the kind "time", and a partial
/// time then goes as `""`, which Rust refuses ("choose when the hold ends").
#[test]
fn typing_a_time_chooses_until_a_time() {
    let controls = js_fn(HOLDS_JS, "holdControls");
    let listens = position(controls, "untilAt.addEventListener(\"input\", () => {");
    assert!(
        controls[listens..].contains("untilKind.value = \"time\";"),
        "{controls}"
    );
    let pressed = position(controls, "holdButton.addEventListener(\"click\", () => {");
    let kind = position(
        controls,
        "const kind = (untilAt.value || untilAt.validity.badInput) ? \"time\" : untilKind.value;",
    );
    let shown = position(controls, "untilKind.value = kind;");
    let sent = position(
        controls,
        "void withhold(row.group_id, whyHold.value, kind, untilAt.value).then(",
    );
    assert!(pressed < kind && kind < shown && shown < sent, "{controls}");
}

/// The other direction (final review, #832): once a time is typed, setting the select BACK to
/// "until reviewed" must mean it. Without this, the press still saw the typed time and sent kind
/// "time" — a doctor's "hold until I've reviewed the BP" became a W that lapsed silently at the
/// typed hour, and the antihypertensive read as givable. So choosing "until reviewed" empties the
/// time field. `value = ""` alone does not discard a PARTLY typed entry in WebKit (Tauri's macOS
/// webview; walked 2026-10-21: `badInput` stayed true, so the press sent `""` and was refused),
/// so the field's type is also reset, which does.
#[test]
fn choosing_until_reviewed_clears_the_time() {
    let controls = js_fn(HOLDS_JS, "holdControls");
    let listens = position(controls, "untilKind.addEventListener(\"change\", () => {");
    let clears = position(controls, "if (untilKind.value === \"reviewed\") {");
    assert!(listens < clears, "{controls}");
    let body = &controls[clears..clears + controls[clears..].find("\n      }").unwrap()];
    for part in [
        "untilAt.value = \"\";",
        "untilAt.type = \"text\";\n        untilAt.type = \"datetime-local\";",
    ] {
        assert!(body.contains(part), "{part} in {body}");
    }
    assert!(
        clears < position(controls, "holdButton.addEventListener(\"click\""),
        "{controls}"
    );
}

/// One press, one W. A second press while the first is in flight would read the same still-filled
/// fields and write a second W (nothing downstream refuses an overlapping hold), so each button
/// disables itself before it sends; the re-read after the gesture rebuilds the line with a fresh,
/// enabled button. That re-read can fail and keep the old line, so a REFUSED press (nothing was
/// written) re-enables its own button — else Withhold sat dead with no word why (#838 review). A
/// press that returned a report keeps it disabled: a recorded W must not be pressable again.
#[test]
fn a_gesture_button_disables_itself_before_it_sends() {
    let controls = js_fn(HOLDS_JS, "holdControls");
    for (button, send) in [
        ("holdButton", "withhold(row.group_id,"),
        ("releaseButton", "release(row.group_id,"),
    ] {
        let pressed = position(
            controls,
            &format!("{button}.addEventListener(\"click\", () => {{"),
        );
        let disabled =
            pressed + position(&controls[pressed..], &format!("{button}.disabled = true;"));
        assert!(
            disabled < pressed + position(&controls[pressed..], send),
            "{button}: {controls}"
        );
        let resolved = pressed + position(&controls[pressed..], ".then((reported) => {");
        assert!(
            controls[resolved..].starts_with(&format!(
                ".then((reported) => {{\n        if (!reported) {button}.disabled = false;"
            )),
            "{button}: {controls}"
        );
    }
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

/// A W starts and ends with the clock, so nothing but a timer re-reads the chart then (#838
/// review: an unattended ward screen kept "withheld · W until 20:00" into the next morning, which
/// then read as today's 20:00, and kept "current" past a pending W's start). Every list drawn arms
/// the re-read at Rust's `next_hold_change`, replacing the last; the wait is capped at a day
/// (setTimeout fires AT ONCE past ~24.8 days, which would loop); the re-read runs only while that
/// patient's list is the one drawn; and a failed re-read tries again rather than leaving the
/// boundary passed in silence.
#[test]
fn the_list_is_read_again_when_a_hold_starts_or_ends() {
    let draw = js_fn(MAIN_JS, "drawList");
    assert!(
        position(draw, "restoreReasons(typed);")
            < position(
                draw,
                "rereadAtNextHoldChange(view.next_hold_change, patient);"
            ),
        "{draw}"
    );
    let arm = js_fn(HOLDS_JS, "rereadAtNextHoldChange");
    for part in [
        "clearTimeout(holdChangeTimer);\n  holdChangeTimer = null;\n  if (!next) return;",
        "Math.min(Math.max(0, Date.parse(next) - Date.now()) + 1000, DAY_MS)",
        "if (renderedPatient !== patient) return;\n    await refresh(rereadLead());",
        "if (holdChangeTimer === armed && renderedPatient === patient) {",
        "holdChangeTimer = armed;",
    ] {
        assert!(arm.contains(part), "{part} in {arm}");
    }
    assert!(HOLDS_JS.contains("const DAY_MS = 24 * 60 * 60 * 1000;"));
}

/// The detail's reveal is the `hidden` attribute, and it hides only because the global
/// `[hidden]` rule wins over any `display` an element's own rule sets: lose `!important`, or give
/// `.hold-detail` a `display`, and every reason sits open over Stop/Withhold/Release.
#[test]
fn a_hidden_detail_is_hidden() {
    let at = position(STYLE_CSS, "\n[hidden] {");
    let rule = &STYLE_CSS[at..at + STYLE_CSS[at..].find("\n}").unwrap()];
    assert!(rule.contains("display: none !important;"), "{rule}");
    let at = position(STYLE_CSS, "\n.hold-detail {");
    let rule = &STYLE_CSS[at..at + STYLE_CSS[at..].find("\n}").unwrap()];
    assert!(
        !rule.contains("display:") && !rule.contains("visibility:"),
        "{rule}"
    );
}
