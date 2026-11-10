//! Charting slice 1a, Task 6 fix round 1 and the final fix wave: closing an opened line without signing (ruling R18), and
//! a changed drug part reading "not checked" until its own check answers (Review Focus 2). Source
//! guards like `med_chart_webview_tests.rs` (whose visibility helpers they share); both behaviours
//! were also walked headlessly with a stub `invoke` (see the task report).
use crate::med_chart_webview_tests::{in_the_line, not_hidden_by_css, section};
use crate::source_guard::{js_fn, position};

const MED_JS: &str = include_str!("../src-ui/med_chart.js");
const MAIN_JS: &str = include_str!("../src-ui/main.js");

/// R18: an opened line closes without signing — Cancel on the line, or Escape when no pick list is
/// open (with one open, Escape closes only the list). Closing discards the unsigned line: the row
/// is HIDDEN again, its words, codings and press forgotten. Neither acts while a Sign is in flight.
#[test]
fn an_opened_line_closes_without_signing() {
    let s = section();
    assert!(
        s.contains("<button id=\"med-line-cancel\" type=\"button\">Cancel</button>"),
        "{s}"
    );
    in_the_line("med-line-cancel");
    not_hidden_by_css("#med-line-cancel");
    assert!(position(s, "id=\"med-sign\"") < position(s, "id=\"med-line-cancel\""));
    assert!(MED_JS.contains("el(\"med-line-cancel\").addEventListener(\"click\", cancelMedLine);"));
    let cancel = js_fn(MED_JS, "cancelMedLine");
    assert!(
        position(cancel, "if (medBusy) return;") < position(cancel, "closeMedLine();"),
        "{cancel}"
    );
    // What closing does: the row hidden, the line emptied, nothing held, nothing pressed.
    let close = js_fn(MED_JS, "closeMedLine");
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
        keys.contains("if (event.key === \"Escape\") {\n    if (n > 0) closeMedPick();\n    else cancelMedLine();"),
        "{keys}"
    );
    assert!(js_fn(MED_JS, "syncMedSign").contains("el(\"med-line-cancel\").disabled = medBusy;"));
}

/// Review Focus 2: when the drug words change (a picked "amoxicillin" retyped "amoxapine"), the
/// line reads "not checked" AT ONCE — never the old, possibly clean verdict for the delay plus the
/// round trip, or for ever if the check hangs — and every check already in flight is outdated, so
/// an answer about the old words can never draw over it. A new pick does the same.
#[test]
fn a_changed_drug_part_reads_not_checked_until_its_check_answers() {
    assert!(MED_JS.contains("const LINE_NOT_CHECKED = { marks: [], notes: [], unverified: true };"));
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
    // And "not checked" is a visible tag (drawCheck's unverified branch un-hides it).
    let drawn = js_fn(MED_JS, "drawCheck");
    assert!(
        position(drawn, "} else if (check.unverified) {") < position(drawn, "tag.hidden = !shown;"),
        "{drawn}"
    );
}

/// Ruling R21: the list's dose cell shows the signed directions beside the dose ("1 g — 1g IV
/// stat"), as `link/view.rs` words a line, so a stat, tds or route reads as it was signed; with no
/// directions, the dose alone (an empty part is dropped, never drawn as a dangling dash).
#[test]
fn the_list_shows_the_signed_directions_beside_the_dose() {
    let render = js_fn(MAIN_JS, "renderRow");
    assert!(
        render.contains("cell(\"td\", [row.dose, row.sig].filter(Boolean).join(\" — \")),"),
        "{render}"
    );
    assert!(!render.contains("cell(\"td\", row.dose),"), "{render}");
}

/// Final review I2: an open line's verdict is about the list it was checked against. Every
/// re-draw (a refused Sign's re-read after the chart set changed, a newly signed allergy) checks
/// an open line again, and a plain refusal does too — each through `recheck`, so the old,
/// possibly clean verdict is taken down AT ONCE rather than left drawn until the answer.
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
    let refused = js_fn(MED_JS, "afterChartRefused");
    let said = position(refused, "setMessage(el(\"med-line-status\"), text);");
    assert!(said < position(refused, "recheck();"), "{refused}");
    assert!(!refused.contains("scheduleCheck();"), "{refused}");
    // main.js's `render` (every drawn answer, the refused Sign's re-read included) ends in it.
    assert!(js_fn(MAIN_JS, "render").contains("syncMedChart();"));
}
