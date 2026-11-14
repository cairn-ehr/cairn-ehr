//! Charting slice 1a, Task 6 fix round 1 and the final fix wave: closing an opened line without signing (ruling R18), and
//! a changed drug part reading "not checked" until its own check answers (Review Focus 2). Source
//! guards like `med_chart_webview_tests.rs` (whose visibility helpers they share); both behaviours
//! were also walked headlessly with a stub `invoke` (see the task report).
use crate::med_chart_webview_tests::{not_hidden_by_css, section};
use crate::source_guard::{js_fn, position};

const MED_JS: &str = include_str!("../src-ui/med_chart.js");
const MAIN_JS: &str = include_str!("../src-ui/main.js");
const LIST_JS: &str = include_str!("../src-ui/med_list_sign.js");

/// R18, as batch charting slice 1 changed it: the lone line's Cancel discarded it, but a list
/// line is durable, so leaving the editor never deletes — Escape (with no pick list open; with
/// one, Escape closes only the list) or Enter FINISHES the line and closes the editor, and the
/// list keeps it. Discarding is the line's own Remove (med_list_webview_tests.rs). Closing
/// forgets the editor's words, codings and press, and is told to nobody: the list is detached
/// before the row moves home (moving it blurs the field).
#[test]
fn an_opened_line_closes_and_the_list_keeps_it() {
    let s = section();
    assert!(!s.contains("id=\"med-line-cancel\""), "{s}");
    assert!(!s.contains("id=\"med-sign\""), "{s}");
    let close = js_fn(MED_JS, "closeMedLine");
    assert!(
        position(close, "medEditor = null;")
            < position(close, "el(\"med-chart\").append(MED_LINE_ROW);"),
        "{close}"
    );
    for step in [
        "el(\"med-line-row\").hidden = true;",
        "el(\"med-line\").value = \"\";",
        "medHeld = [];",
        "medPress = null;",
        "button.setAttribute(\"aria-pressed\", \"false\")",
    ] {
        assert!(close.contains(step), "{step}: {close}");
    }
    // Escape: the list first, the line only when no list is open.
    let keys = &MED_JS[position(MED_JS, "el(\"med-line\").addEventListener(\"keydown\"")..];
    let keys = &keys[..position(keys, "});")];
    assert!(
        keys.contains("if (event.key === \"Escape\") {\n    if (n > 0) closeMedPick();\n    else finishMedLine(true);"),
        "{keys}"
    );
    assert!(js_fn(MED_JS, "finishMedLine").contains("if (medEditor) medEditor.finished(byKey);"));
    // A click on a drug keeps the focus in the field, so it never finishes the line.
    assert!(MED_JS.contains(
        "el(\"med-pick\").addEventListener(\"mousedown\", (event) => event.preventDefault());"
    ));
    not_hidden_by_css("#med-line");
}

/// Review Focus 2: when the drug words change (a picked "amoxicillin" retyped "amoxapine"), the
/// line reads "not checked" AT ONCE — never the old, possibly clean verdict for the delay plus the
/// round trip, or for ever if the check hangs — and every check already in flight is outdated, so
/// an answer about the old words can never draw over it. A new pick does the same.
#[test]
fn a_changed_drug_part_reads_not_checked_until_its_check_answers() {
    assert!(MED_JS.contains(
        "const LINE_NOT_CHECKED = { marks: [], warning_notes: [], notes: [], unverified: true };"
    ));
    let input = js_fn(MED_JS, "onMedLineInput");
    assert!(input.contains("if (drugChanged) recheck();"), "{input}");
    let recheck = js_fn(MED_JS, "recheck");
    let outdate = position(recheck, "medCheckRevision += 1;");
    let draw = position(recheck, "drawCheck(LINE_NOT_CHECKED);");
    let ask = position(recheck, "scheduleCheck();");
    assert!(outdate < draw && draw < ask, "{recheck}");
    assert!(js_fn(MED_JS, "chooseMedPick").contains("recheck();"));
    // An answer is drawn only for the newest check asked.
    let check = js_fn(MED_JS, "checkLine");
    assert!(
        check.contains("if (revision === medCheckRevision) drawCheck(check);"),
        "{check}"
    );
    // And "not checked" is a visible tag (drawMarks's unverified branch un-hides it).
    let drawn = js_fn(MED_JS, "drawMarks");
    assert!(
        position(drawn, "} else if (check.unverified) {") < position(drawn, "tag.hidden = !shown;"),
        "{drawn}"
    );
}

/// Rulings R21/R25: the list's dose cell shows the signed directions beside the dose ("1 g — 1g
/// IV stat"), so a stat, tds or route reads as it was signed. The wording is decided in Rust
/// (`MedListRowView::dose_cell`, pinned by row_view_tests.rs); the window only draws it.
#[test]
fn the_list_shows_the_signed_directions_beside_the_dose() {
    let render = js_fn(MAIN_JS, "renderRow");
    assert!(render.contains("cell(\"td\", row.dose_cell),"), "{render}");
    assert!(!render.contains("cell(\"td\", row.dose),"), "{render}");
}

/// Final review I2: an open line's verdict is about the list it was checked against. Every
/// re-draw (a refused Sign's re-read after the chart set changed, a newly signed allergy) checks
/// an open line again through `recheck`, so the old, possibly clean verdict is taken down AT
/// ONCE rather than left drawn until the answer; a refused list Sign reads the list again.
#[test]
fn a_redrawn_list_or_a_refusal_checks_an_open_line_again() {
    let sync = js_fn(MED_JS, "syncMedChart");
    assert!(
        position(sync, "el(\"med-chart\").hidden =")
            < position(
                sync,
                "if (medPress !== null && !el(\"med-chart\").hidden) recheck();"
            ),
        "{sync}"
    );
    // A refused list Sign reads the list again, so each line's verdict is Rust's fresh one.
    let refused = js_fn(LIST_JS, "afterMedListRefused");
    let said = position(refused, "setMessage(el(\"med-list-status\"), text);");
    assert!(
        said < position(refused, "void readMedDraft();"),
        "{refused}"
    );
    // main.js's `render` (every drawn answer, the refused Sign's re-read included) ends in it.
    assert!(js_fn(MAIN_JS, "render").contains("syncMedChart();"));
}

/// PR #1066 review finding: a read-back that fails must never show a blank parse with no
/// "not read" part. The typed directions show whole, as not read, so nothing is dropped.
#[test]
fn a_failed_read_back_shows_the_directions_as_not_read() {
    let read_back = js_fn(MED_JS, "readBack");
    assert!(
        position(read_back, "catch (failure)")
            < position(read_back, "reading = { unread: [line.directions] };"),
        "{read_back}"
    );
}
