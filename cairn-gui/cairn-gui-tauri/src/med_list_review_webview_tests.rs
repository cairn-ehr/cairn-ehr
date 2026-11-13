//! Batch charting slice 1, Task 6's review fix round: the list pane's source guards (the rig of
//! `med_list_webview_tests.rs`, whose helpers these share), each also walked headlessly with a
//! stub `invoke`, asserting computed visibility (see the task report).
use crate::med_chart_webview_tests::{not_hidden_by_css, section};
use crate::med_list_webview_tests::{in_the_pane, LIST_JS};
use crate::source_guard::{js_fn, position};

const NOTES_JS: &str = include_str!("../src-ui/notes.js");
const WHOS_HERE_JS: &str = include_str!("../src-ui/whos_here.js");

/// C1: unticking a line is the clinician's veto, STORED (`held`): a closed chart, a re-lock, a
/// who's-here switch or a restart puts the list away and reads it again, and the line must come
/// back as the clinician left it. So the untick is saved, read back, and drawn from Rust's
/// `ticked` — never from a set this window forgets. Changed words clear it (the door does too).
#[test]
fn an_untick_is_stored_and_survives_putting_the_list_away() {
    let tick = js_fn(LIST_JS, "tickMedLine");
    assert!(tick.contains("line.held = !checked;"), "{tick}");
    assert!(
        !tick.contains("medUnticked.add("),
        "an untick is never session-only: {tick}"
    );
    assert_eq!(
        LIST_JS.matches("medUnticked.add(").count(),
        1,
        "only a Sign's Changed lines"
    );
    assert!(js_fn(LIST_JS, "queueSave").contains("held: line.held,"));
    assert!(js_fn(LIST_JS, "sendLine").contains("held: sent.held,"));
    assert!(js_fn(LIST_JS, "lineFrom").contains("held: v.held,"));
    assert!(js_fn(LIST_JS, "mergeLine").contains("line.held = v.held;"));
    assert!(js_fn(LIST_JS, "lineTyped").contains("line.held = false;"));
    // Drawn from what the node read (Rust's `ticked` honours `held`), once the save landed.
    let drawn = js_fn(LIST_JS, "drawnTicked");
    assert!(drawn.contains("line.view.ticked"), "{drawn}");
}

/// I1: a list Sign in flight is waited for wherever a note's is — the who's-here switch, the
/// re-lock and the person menu ask `signsSettled` / `signInFlight` — and its outcome is said in
/// the picker in Rust's words. A Sign that answers after another person's list is on screen draws
/// nothing on it.
#[test]
fn a_list_sign_in_flight_is_waited_for_and_said_in_the_picker() {
    let settled = js_fn(NOTES_JS, "signsSettled");
    assert!(
        settled.contains("if (!medListSignInFlight) return noteSigning;"),
        "{settled}"
    );
    assert!(settled.contains("medListSigning"), "{settled}");
    assert!(js_fn(NOTES_JS, "signInFlight").contains("noteSignInFlight || medListSignInFlight"));
    let line = js_fn(WHOS_HERE_JS, "signOutcomeLine");
    assert!(
        line.contains("if (outcome && outcome.words) return outcome.words;"),
        "{line}"
    );
    assert!(js_fn(WHOS_HERE_JS, "switchWhosHere").contains("await signsSettled();"));
    let press = &LIST_JS[position(LIST_JS, "el(\"med-list-sign\").addEventListener(\"click\"")..];
    let flag = position(press, "medListSignInFlight = true;");
    let run = position(press, "medListSigning = signMedList()");
    let down = position(press, "medListSignInFlight = false;");
    assert!(flag < run && run < down, "{press}");
    let sign = js_fn(LIST_JS, "signMedList");
    assert!(
        sign.contains(
            "return { words: answer.kind === \"changed\" ? answer.message : answer.outcome };"
        ),
        "{sign}"
    );
    assert!(
        sign.contains("return { words: failureText(failure) };"),
        "{sign}"
    );
    // Whose list: captured at the press, checked before anything is drawn on the list.
    assert!(sign.contains("const writer = medList.writer;"), "{sign}");
    let after = js_fn(LIST_JS, "afterMedListSigned");
    assert!(
        position(after, "if (!ownsTheList(writer)) return;")
            < position(after, "medLineNotes.clear();"),
        "{after}"
    );
    assert!(js_fn(LIST_JS, "afterMedListRefused").contains("if (!ownsTheList(writer)) return;"));
    assert!(js_fn(LIST_JS, "ownsTheList").contains("medList !== null && medList.writer === writer"));
}

/// I2: a duplicate line says which line it duplicates, in Rust's words, as visible text on the
/// line's state; the window never words it.
#[test]
fn a_duplicate_says_which_line_it_duplicates() {
    let draw = js_fn(LIST_JS, "drawLine");
    assert!(
        draw.contains("if (v && v.duplicate_of) words.push(v.duplicate_of);"),
        "{draw}"
    );
    assert!(
        position(draw, "words.push(v.duplicate_of)")
            < position(draw, "setMessage(r.state, words.join(\" · \"));")
    );
    assert!(!LIST_JS.contains("already on the list"), "Rust's words");
    not_hidden_by_css(".med-list-state");
}

/// M2: whose list it is, said above the lines, visibly.
#[test]
fn the_writer_line_is_visible() {
    in_the_pane("med-list-writer");
    not_hidden_by_css("#med-list-writer");
    assert!(js_fn(LIST_JS, "drawMedList").contains("setMessage(el(\"med-list-writer\"), whose);"));
}

/// M4: no tick before the node has read the line's words: the tick vouches for the warnings DRAWN
/// on it, and until that read none are.
#[test]
fn no_tick_before_the_line_is_read() {
    let draw = js_fn(LIST_JS, "drawLine");
    assert!(
        draw.contains("r.tick.disabled = blocked || !v ||"),
        "{draw}"
    );
}

/// M5: a Sign stopped by a line that could not be saved clears the passphrase, as every other
/// way out does.
#[test]
fn a_failed_save_clears_the_passphrase() {
    let sign = js_fn(LIST_JS, "signMedList");
    let stopped = position(sign, "if (saved.includes(false)) {");
    let cleared = stopped + position(&sign[stopped..], "clearSigner();");
    let out = stopped + position(&sign[stopped..], "return { words:");
    assert!(cleared < out, "{sign}");
}

/// M6: the outcome is announced once — by the chart's own outcome line — and only shown under the
/// list (no second live region).
#[test]
fn the_outcome_is_announced_once() {
    assert!(section().contains("<p id=\"med-list-outcome\" hidden></p>"));
    assert_eq!(
        js_fn(LIST_JS, "drawSignedTable")
            .matches("say(answer.outcome);")
            .count(),
        1
    );
}

/// M7: Enter on a line pressed and left empty drops it and keeps the focus on the press, never
/// lost to the page.
#[test]
fn enter_on_an_empty_line_keeps_the_focus_on_the_press() {
    let finished = js_fn(LIST_JS, "lineFinished");
    let dropped = position(finished, "dropLine(line);");
    let focus = dropped + position(&finished[dropped..], "if (byKey) focusPress(line);");
    let gone = dropped + position(&finished[dropped..], "return;");
    assert!(focus < gone, "{finished}");
    // focusPress moved to med_chart.js, beside the presses it focuses (slice 2a, file size).
    assert!(js_fn(include_str!("../src-ui/med_chart.js"), "focusPress").contains(".focus();"));
}
