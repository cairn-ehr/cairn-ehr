//! Batch charting slice 1, Task 6 (split from `med_list_webview_tests.rs`, same rig): the
//! medication list and who's here — a switch never lets one person type into another's list —
//! and the person menu's "meds:" row landing on the visible list; and a list held at another
//! terminal (moved here from `med_list_webview_tests.rs` for its size cap).
use crate::med_chart_webview_tests::section;
use crate::med_list_webview_tests::{in_the_pane, LIST_JS};
use crate::source_guard::{js_fn, position};

const MED_JS: &str = include_str!("../src-ui/med_chart.js");
const WHOS_HERE_JS: &str = include_str!("../src-ui/whos_here.js");
const MENU_JS: &str = include_str!("../src-ui/person_menu.js");

/// Review requirement: when who's here names someone else, the last person's list leaves the
/// screen at once — editor closed, its pending saves queued under ITS writer, every read in flight
/// outdated — and nothing can be typed until the new person's list is read. Otherwise B's
/// keystrokes would save into A's list under A's name, a complete clean line would be
/// default-ticked, and A would sign a line they never typed.
#[test]
fn a_whos_here_switch_never_types_into_the_previous_list() {
    let claim = js_fn(WHOS_HERE_JS, "showClaim");
    assert!(
        position(claim, "if (changed) {") < position(claim, "medListClaimChanged();"),
        "{claim}"
    );
    let switched = js_fn(LIST_JS, "medListClaimChanged");
    assert!(
        position(switched, "const flushed = putAwayMedList();")
            < position(switched, "void flushed.then(() => readMedDraft());"),
        "{switched}"
    );
    let away = js_fn(LIST_JS, "putAwayMedList");
    let steps = [
        "medEditingLine = null;",
        "closeMedLine();",
        "const flushed = flushMedLines();",
        "medList = null;",
        "medLines = [];",
        "medListRevision += 1;",
        "el(\"med-list\").replaceChildren();",
        "drawMedList();",
    ];
    let at: Vec<usize> = steps.iter().map(|s| position(away, s)).collect();
    assert!(at.windows(2).all(|w| w[0] < w[1]), "{away}");
    // With no list read, nothing can be added, and no read for the old claim is drawn.
    assert!(js_fn(LIST_JS, "appendMedLine")
        .contains("if (medList === null || medSigning || medList.elsewhere !== null) return;"));
    assert!(js_fn(LIST_JS, "drawMedList").contains(
        "button.disabled = medSigning || medList === null || medList.elsewhere !== null;"
    ));
    let read = js_fn(LIST_JS, "readMedDraft");
    assert!(
        read.contains("if (revision !== medListRevision || patient !== renderedPatient) return;"),
        "{read}"
    );
    // A new line takes the writer of the list it was added to.
    assert!(js_fn(LIST_JS, "newLine").contains("lineFrom(blank, medList, medList.patient)"));
}

/// Task 5 review: a menu row's "meds:" focus lands on the medication pane once it is VISIBLE.
/// The note pane may draw first (the pane is still hidden then): the focus waits, and the list
/// takes it when it draws.
#[test]
fn a_meds_menu_row_lands_on_the_visible_list() {
    let focus = js_fn(MENU_JS, "focusMenuTarget");
    let meds = position(focus, "if (kind === \"meds\") {");
    let wait = position(focus, "if (pane.hidden) {");
    let kept = position(focus, "pendingMenuFocus = focus;");
    assert!(meds < wait && wait < kept, "{focus}");
    assert!(focus.contains("pane.focus();"), "{focus}");
    let drawn = js_fn(LIST_JS, "drawMedRead");
    assert!(
        drawn.contains("if (pendingMenuFocus === \"meds:\") focusMenuTarget(takeMenuFocus());"),
        "{drawn}"
    );
    // The list is read only over a drawn, writable chart: the pane is then shown (syncMedChart).
    assert!(js_fn(LIST_JS, "syncMedList")
        .contains("if (renderedPatient !== null && !fixtureMode) void readMedDraft();"));
    assert!(js_fn(MED_JS, "syncMedChart")
        .contains("el(\"med-chart\").hidden = renderedPatient === null || fixtureMode;"));
}

/// A list held at another terminal is drawn read-only with "Continue here" (`med_list_take`),
/// which moves it here and draws what it answers.
#[test]
fn another_terminal_offers_continue_here() {
    let s = section();
    assert!(
        s.contains("<button id=\"med-list-take\" type=\"button\">Continue here</button>"),
        "{s}"
    );
    in_the_pane("med-list-elsewhere");
    assert!(position(s, "id=\"med-list-elsewhere\"") < position(s, "id=\"med-list-take\""));
    let pane = js_fn(LIST_JS, "drawMedList");
    assert!(
        pane.contains("el(\"med-list-elsewhere\").hidden = !(medList && medList.elsewhere);"),
        "{pane}"
    );
    let take = js_fn(LIST_JS, "continueMedListHere");
    assert!(
        take.contains("invoke(\"med_list_take\", { patientId: patient })"),
        "{take}"
    );
    assert!(take.contains("drawMedRead(fresh, patient);"), "{take}");
    // Held elsewhere, nothing on the line acts.
    let draw = js_fn(LIST_JS, "drawLine");
    assert!(
        draw.contains(
            "const blocked = medSigning || medList === null || medList.elsewhere !== null;"
        ),
        "{draw}"
    );
    for control in ["r.text.disabled = blocked;", "r.remove.disabled = blocked;"] {
        assert!(draw.contains(control), "{control}: {draw}");
    }
}
