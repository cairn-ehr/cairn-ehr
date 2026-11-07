//! Sign-in slice 3c-3c, Task 6: the webview's "Waiting for you" menu (`person_menu.js`) and the
//! few lines in other scripts that call it. The window's scripts have no test rig (no npm), so each
//! guard reads the source (`crate::source_guard`) and pins the ORDER or the CHOICE a property is;
//! the behaviour itself was walked headlessly with a stub `invoke` (see the slice's PR).
use crate::source_guard::{js_fn, position};

const MENU_JS: &str = include_str!("../src-ui/person_menu.js");
const WHOS_HERE_JS: &str = include_str!("../src-ui/whos_here.js");
const NOTES_JS: &str = include_str!("../src-ui/notes.js");
const COMPOUND_JS: &str = include_str!("../src-ui/notes_compound.js");
const FUNNEL_JS: &str = include_str!("../src-ui/funnel.js");
const INDEX_HTML: &str = include_str!("../src-ui/index.html");

/// Review Focus 5: from an open chart the queued saves land first (a sign in flight settles, then
/// `closeChart` awaits the saves), and only then does the node open the row's chart.
#[test]
fn open_menu_row_settles_signs_and_closes_the_chart_before_opening() {
    let open = js_fn(MENU_JS, "openMenuRow");
    let settle = position(open, "if (signInFlight()) await signsSettled();");
    let close = position(open, "if (displayedPatient) await closeChart();");
    let ask = position(open, "invoke(\"person_menu_open\"");
    let enter = position(open, "enterChart(");
    assert!(settle < close && close < ask && ask < enter, "{open}");
    // The focus target is set before the chart is entered: the note pane may draw at once.
    assert!(
        position(open, "pendingMenuFocus = row.focus") < enter,
        "{open}"
    );
    // A refusal is said in the menu's own line, then the list is read again.
    assert!(
        open.contains("person-menu-status") && open.contains("refreshMenuCount()"),
        "{open}"
    );
}

#[test]
fn the_review_group_is_a_closed_details() {
    let group = js_fn(MENU_JS, "reviewGroup");
    assert!(
        group.contains("document.createElement(\"details\")"),
        "{group}"
    );
    assert!(
        !group.contains(".open = true") && !group.contains("setAttribute(\"open\""),
        "{group}"
    );
}

#[test]
fn the_menu_is_hidden_for_nobody() {
    let draw = js_fn(MENU_JS, "drawMenu");
    assert!(
        position(draw, "view.kind === \"Nobody\"") < position(draw, "view.count_words"),
        "{draw}"
    );
    assert!(draw.contains("menu.hidden = true"), "{draw}");
    assert!(INDEX_HTML.contains("<details id=\"person-menu\" hidden>"));
}

#[test]
fn stale_menu_answers_are_dropped() {
    let read = js_fn(MENU_JS, "refreshMenuCount");
    assert!(
        position(read, "++menuRevision") < position(read, "await invoke(\"person_menu\")"),
        "{read}"
    );
    let sent = position(read, "await invoke(\"person_menu\")");
    // Checked after the await and before anything is drawn, on the answer AND on the failure.
    assert_eq!(
        read.matches("revision !== menuRevision").count(),
        2,
        "{read}"
    );
    assert!(
        position(&read[sent..], "revision !== menuRevision") < position(&read[sent..], "drawMenu("),
        "{read}"
    );
    let picker = js_fn(MENU_JS, "loadPickerCounts");
    assert!(
        position(picker, "++pickerRevision") < position(picker, "await invoke("),
        "{picker}"
    );
    assert!(picker.contains("revision !== pickerRevision"), "{picker}");
}

#[test]
fn the_picker_draws_before_counts_land() {
    let open = js_fn(WHOS_HERE_JS, "openPicker");
    let draw = position(open, "drawPicker();");
    let counts = position(open, "loadPickerCounts()");
    assert!(draw < counts, "{open}");
    // The counts are remembered, so a later search redraw keeps the suffixes.
    assert!(js_fn(WHOS_HERE_JS, "drawPicker").contains("pickerSuffix"));
    // A failed read is worded by Rust: the script writes no sentence of its own.
    assert!(js_fn(MENU_JS, "loadPickerCounts").contains("failureText(failure)"));
}

#[test]
fn the_script_writes_no_sentence_the_clinician_reads() {
    for sentence in [
        "Waiting for you",
        "Nothing here",
        "Open note",
        "could not be read",
    ] {
        assert!(!MENU_JS.contains(sentence), "{sentence}");
    }
}

#[test]
fn the_other_scripts_call_the_menu_where_it_changes() {
    // A new claim, a drawn note pane, a closed chart: each can change the count.
    assert!(js_fn(WHOS_HERE_JS, "showClaim").contains("refreshMenuCount()"));
    assert!(js_fn(NOTES_JS, "readNotes").contains("refreshMenuCount()"));
    assert!(js_fn(NOTES_JS, "renderNotes").contains("takeMenuFocus()"));
    assert!(js_fn(FUNNEL_JS, "closeChart").contains("refreshMenuCount()"));
    assert!(NOTES_JS.contains("li.dataset.draft = draft.draft_id;"));
    assert!(COMPOUND_JS.contains("li.dataset.parts = compound.shown_parts.join(\" \");"));
}

/// Load ORDER, never a `typeof` guard (R5a): the menu is declared before the script that calls it
/// at load time (`showClaim` runs from whos_here.js's seed).
#[test]
fn the_script_loads_before_whos_here() {
    assert!(
        position(INDEX_HTML, "<script src=\"person_menu.js\">")
            < position(INDEX_HTML, "<script src=\"whos_here.js\">")
    );
    assert!(!MENU_JS.contains("typeof "));
}
