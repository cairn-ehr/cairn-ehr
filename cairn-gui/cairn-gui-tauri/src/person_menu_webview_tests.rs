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
const SUPERVISION_JS: &str = include_str!("../src-ui/supervision.js");
const INDEX_HTML: &str = include_str!("../src-ui/index.html");

/// Review Focus 5: from an open chart the queued saves land first (a sign in flight settles, then
/// `closeChart` awaits the saves), and only then does the node open the row's chart.
#[test]
fn open_menu_row_settles_signs_and_closes_the_chart_before_opening() {
    let open = js_fn(MENU_JS, "openMenuRow");
    let settle = position(
        open,
        "const outcome = signInFlight() ? await signsSettled() : null;",
    );
    let close = position(open, "if (displayedPatient) await closeChart();");
    let ask = position(open, "invoke(\"person_menu_open\"");
    let enter = position(open, "enterChart(");
    assert!(settle < close && close < ask && ask < enter, "{open}");
    // The focus target is set before the chart is entered: the note pane may draw at once.
    assert!(
        position(open, "pendingMenuFocus = row.focus") < enter,
        "{open}"
    );
    // A refusal is said in the menu's own line AFTER the re-read lands (a drawn menu clears the
    // status), then focus moves to it: the clicked button is gone.
    let reread = position(open, "await refreshMenuCount()");
    let said = position(open, "String(refusal)");
    assert!(reread < said, "{open}");
    assert!(
        said < position(open, "el(\"person-menu-status\").focus()"),
        "{open}"
    );
    // Mid-registration the menu opens nothing (funnel.js's openChart has the same first line).
    assert!(
        position(open, "if (registering) return;")
            < position(open, "const outcome = signInFlight()"),
        "{open}"
    );
}

/// The button's visible words and its accessible name are both Rust's, and are two fields.
#[test]
fn the_open_button_shows_short_words_and_is_named_in_full() {
    let row = js_fn(MENU_JS, "menuRow");
    assert!(
        row.contains("row.open_words") && row.contains("\"aria-label\": row.open_label"),
        "{row}"
    );
}

/// A failed read must be heard while the menu is closed or never drawn: the status line sits
/// beside the `<details>`, in the bar, not inside it.
#[test]
fn the_menu_status_is_outside_the_details() {
    let status = position(INDEX_HTML, "id=\"person-menu-status\"");
    assert!(status < position(INDEX_HTML, "<details id=\"person-menu\""));
    assert!(position(INDEX_HTML, "id=\"whos-here-bar\"") < status);
    assert!(js_fn(MENU_JS, "drawMenu").contains("setMessage(el(\"person-menu-status\"), \"\")"));
}

/// A failed staff list says so, and the counts' failure line never overwrites it.
#[test]
fn counts_are_not_read_after_a_failed_staff_list() {
    let open = js_fn(WHOS_HERE_JS, "openPicker");
    assert!(
        position(open, "listed = false;") < position(open, "if (listed) void loadPickerCounts();"),
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
    // Nothing anywhere in the script opens anything; only `.open = false` is ever written.
    assert!(!MENU_JS.contains(".open = true"));
}

#[test]
fn the_menu_is_hidden_for_nobody() {
    let draw = js_fn(MENU_JS, "drawMenu");
    assert!(
        position(draw, "view.kind === \"Nobody\"") < position(draw, "view.count_words"),
        "{draw}"
    );
    assert!(draw.contains("hideMenu()"), "{draw}"); // hideMenu sets `hidden = true`
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

/// F1 (final review): a different person named means the bar's menu is no longer theirs, so it
/// leaves the screen BEFORE the new person's read is sent; a failed read leaves it hidden too.
/// Either way nothing of the last person's list is ever drawn under the next person's name.
#[test]
fn the_last_persons_menu_leaves_before_the_next_read() {
    let hide = js_fn(MENU_JS, "hideMenu");
    for step in [
        "hidden = true",
        "open = false",
        "el(\"person-menu-body\").replaceChildren()",
    ] {
        assert!(hide.contains(step), "{step}: {hide}");
    }
    let show = js_fn(WHOS_HERE_JS, "showClaim");
    assert!(
        position(show, "hideMenu()") < position(show, "refreshMenuCount()"),
        "{show}"
    );
    // The catch path hides the menu; the failure sentence lives in the bar, outside the <details>.
    let read = js_fn(MENU_JS, "refreshMenuCount");
    let catch = &read[position(read, "catch (failure)")..];
    assert!(
        position(catch, "hideMenu()") < position(catch, "return false;\n  }"),
        "{read}"
    );
    assert!(js_fn(MENU_JS, "drawMenu").contains("hideMenu()"));
}

/// F3 (final review): a second press while the first is still closing one chart and opening
/// another returns at once, and the flag is cleared in a `finally`, so a refusal or a throw can
/// never leave the buttons dead.
#[test]
fn open_menu_row_refuses_a_second_press_while_one_runs() {
    let open = js_fn(MENU_JS, "openMenuRow");
    let registering = position(open, "if (registering) return;");
    let guard = position(open, "if (menuOpening) return;");
    assert!(registering < guard, "{open}");
    // Nothing runs between the two guards: the flag check is the next statement.
    let between = &open[registering..guard];
    assert_eq!(between.matches(';').count(), 1, "{open}");
    assert!(
        guard < position(open, "menuOpening = true;")
            && position(open, "menuOpening = true;")
                < position(open, "const outcome = signInFlight()"),
        "{open}"
    );
    let finally = &open[position(open, "} finally {")..];
    assert!(finally.contains("menuOpening = false;"), "{open}");
    assert!(MENU_JS.contains("let menuOpening = false;"));
}

/// F4 (final review): a supervision act changes who co-signs what, so it re-reads the bar's count
/// once it went through (ADR-0095: every place that can change what waits re-reads it).
#[test]
fn a_supervision_act_re_reads_the_bar() {
    let act = js_fn(SUPERVISION_JS, "supervisionAct");
    let sent = position(act, "await invoke(\"supervision_act\"");
    let reread = position(act, "void refreshMenuCount();");
    assert!(
        sent < reread && reread < position(act, "catch (failure)"),
        "{act}"
    );
}

/// Finalcheck (silent-failure review): a Sign, Discard, sign-off or review still in flight that
/// did NOT go through (or is not known to have) keeps its chart open, so the note pane that says
/// how it went is not cleared by `closeChart`, and the bar's line says it in whos_here.js's words.
/// Only an act that went through lets the row's chart open.
#[test]
fn open_menu_row_stays_on_a_chart_whose_act_did_not_go_through() {
    let open = js_fn(MENU_JS, "openMenuRow");
    let settle = position(
        open,
        "const outcome = signInFlight() ? await signsSettled() : null;",
    );
    let stay = position(open, "if (outcome && !SETTLED_WELL.has(outcome)) {");
    let said = position(open, "signOutcomeLine(outcome)");
    let close = position(open, "if (displayedPatient) await closeChart();");
    assert!(settle < stay && stay < said && said < close, "{open}");
    assert!(open[stay..close].contains("return;"), "{open}");
    assert!(
        MENU_JS.contains(
            r#"const SETTLED_WELL = new Set(["signed", "discarded", "signed-off", "reviewed"]);"#
        ),
        "{MENU_JS}"
    );
}

/// Finalcheck (silent-failure review): when the re-read after a refusal itself fails, its line
/// (the list could not be read) stays; the refusal, which says the list WAS read again, does not
/// overwrite it.
#[test]
fn a_refusal_never_overwrites_a_failed_re_read() {
    let refresh = js_fn(MENU_JS, "refreshMenuCount");
    let caught = &refresh[position(refresh, "} catch (failure) {")..];
    assert!(caught.contains("return false;"), "{refresh}");
    let open = js_fn(MENU_JS, "openMenuRow");
    assert!(
        open.contains(
            "if (await refreshMenuCount()) setMessage(el(\"person-menu-status\"), String(refusal));"
        ),
        "{open}"
    );
}
