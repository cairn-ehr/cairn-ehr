//! Sign-in slice 3b-2b, Task 6: the webview's Discard draft button and the discarded-draft
//! markers (`notes_discard.js`).
//!
//! The window's scripts have no test rig (no npm or bundler: main.js's header, issue #332), so,
//! like `notes_webview_tests.rs`, each guard reads the script's source (`crate::source_guard`) and
//! pins the ORDER or the CHOICE a property is. The behaviour was walked headlessly (a stub
//! `invoke` in a browser) when it was written — see the slice's PR.
use crate::source_guard::{js_const, js_fn, position};

const DISCARD_JS: &str = include_str!("../src-ui/notes_discard.js");
/// The note pane's other two scripts as one text: notes.js, and its save queue notes_save.js.
const NOTES_JS: &str = concat!(
    include_str!("../src-ui/notes.js"),
    include_str!("../src-ui/notes_save.js")
);
const INDEX_HTML: &str = include_str!("../src-ui/index.html");
const WHOS_HERE_JS: &str = include_str!("../src-ui/whos_here.js");
const IDLE_JS: &str = include_str!("../src-ui/idle.js");

/// A pending autosave that fired after the press would only say what the press already carries,
/// so it is cancelled BEFORE the discard is sent; a partly typed time is refused before that, so
/// the pending save still goes (the same order as Sign).
#[test]
fn discard_cancels_the_pending_save_before_it_discards() {
    let discard = js_fn(DISCARD_JS, "discardNote");
    let sent = position(discard, "invoke(\"note_discard\"");
    assert!(
        position(discard, "clearTimeout(noteSaveTimer);") < sent,
        "{discard}"
    );
    assert!(
        position(discard, "if (when.validity.badInput) {")
            < position(discard, "clearTimeout(noteSaveTimer);"),
        "{discard}"
    );
}

/// Review Focus 4: the audit copy holds the text ON SCREEN, not the last autosave, so the press
/// sends the field's value, the stated time, the captured writer and the strip's credential.
#[test]
fn discard_sends_the_credential_and_the_text_on_screen() {
    let discard = js_fn(DISCARD_JS, "discardNote");
    for line in [
        "narrative: el(\"note-text\").value,",
        "statedTime: when.value,",
        "writer: held.writer,",
        "credential: signerCredential(),",
    ] {
        assert!(discard.contains(line), "{line} in {discard}");
    }
}

/// Principle 3: a confirmation dialog is not a safety mechanism. The discard is one press; the
/// signed record of what it held is the safety net.
#[test]
fn the_discard_button_never_asks_to_confirm() {
    for dialog in ["confirm(", "alert(", "prompt(", "<dialog"] {
        assert!(
            !DISCARD_JS.contains(dialog),
            "notes_discard.js uses {dialog}"
        );
    }
    assert!(INDEX_HTML.contains(
        "<button id=\"note-sign\" type=\"button\">Sign note</button>\n            <button id=\"note-discard\" type=\"button\">Discard draft</button>"
    ));
}

/// Ruling R13 for a discard: while it is in flight the editor is read-only and both buttons are
/// down (a key pressed then would be in neither the audit copy nor any draft), and it shares
/// Sign's in-flight flags, so idle.js's re-lock and whos_here.js's switch wait for it.
#[test]
fn the_editor_is_read_only_while_a_discard_is_in_flight() {
    assert!(js_fn(NOTES_JS, "setSigning").contains("el(\"note-discard\").disabled = on;"));
    let discard = js_fn(DISCARD_JS, "discardNote");
    assert!(
        position(discard, "setSigning(true);") < position(discard, "invoke(\"note_discard\""),
        "{discard}"
    );
    let caught = &discard[position(discard, "catch (failure) {")..];
    assert!(caught.contains("setSigning(false);"), "{discard}");
    assert!(NOTES_JS.contains(
        "el(\"note-discard\").addEventListener(\"click\", () => {\n  noteSignInFlight = true;\n  noteSigning = discardNote()\n    .then("
    ));
}

/// Controller ruling 1: the press carries the text, so a late save answering `discarded` after
/// THIS window's own discard never forks. The mark is set before the send, and given back only
/// on a refusal known to have recorded nothing.
#[test]
fn a_discard_marks_the_press_as_carrying_the_text() {
    let discard = js_fn(DISCARD_JS, "discardNote");
    let marked = position(discard, "held.mayBeSigned = true;");
    assert!(
        position(discard, "const wasMaybeSigned = Boolean(held.mayBeSigned);") < marked,
        "{discard}"
    );
    assert!(
        marked < position(discard, "invoke(\"note_discard\""),
        "{discard}"
    );
    let caught = &discard[position(discard, "catch (failure) {")..];
    assert!(
        caught.contains(
            "if (behindTheRecord(failure) !== \"unknown\") held.mayBeSigned = wasMaybeSigned;"
        ),
        "{discard}"
    );
}

/// A save answering `discarded` is handled exactly as `spent`: it forks (the DISCARDED_ELSEWHERE
/// words) only when this window did not press; after its own press the text went with it.
#[test]
fn a_discarded_save_is_handled_as_spent() {
    let send = js_fn(NOTES_JS, "sendDraft");
    assert!(
        send.contains(
            "if (view.moved || view.committed || ((view.spent || view.discarded) && !sent.held.mayBeSigned)) return keepUnsavedText(sent, view.fork);"
        ),
        "{send}"
    );
    assert!(
        send.contains("const kept = view.saved || view.spent || view.discarded;"),
        "{send}"
    );
}

/// Controller ruling 2: a refused discard rolled back its own save (the node saves the on-screen
/// text and discards it in one transaction), so the refusal saves the text on screen again, and
/// the editor stays open with it — every refusal says "your text is still here".
#[test]
fn a_refused_discard_saves_the_text_on_screen_again() {
    let discard = js_fn(DISCARD_JS, "discardNote");
    let caught = &discard[position(discard, "catch (failure) {")..];
    let caught = &caught[..position(caught, "\n  }")];
    assert!(caught.contains("void saveNoteNow();"), "{discard}");
    // Rust's refusal already says "Not discarded: …"; the window adds no second lead.
    assert!(
        caught.contains("noteStatus(failureText(failure));"),
        "{discard}"
    );
    assert!(!caught.contains("closeEditor()"), "{discard}");
    assert!(
        caught.contains(
            "return behindTheRecord(failure) === \"unknown\" ? \"unknown\" : \"not-discarded\";"
        ),
        "{discard}"
    );
    // Recorded: the editor closes and says so. Not known: the text is saved first, as Sign does.
    let recorded = &discard[position(discard, "\n  }")..];
    assert!(
        position(recorded, "closeEditor();") < position(recorded, "noteStatus(report.text);"),
        "{discard}"
    );
    assert!(
        recorded.contains("if (report.behind_the_record && !(await saveNoteNow())) {"),
        "{discard}"
    );
}

/// The text a clinician typed is shown as text, never parsed as markup (ADR-0041).
#[test]
fn markers_are_text_never_html() {
    for markup in [
        "innerHTML",
        "outerHTML",
        "insertAdjacentHTML",
        "document.write",
    ] {
        assert!(
            !DISCARD_JS.contains(markup),
            "notes_discard.js uses {markup}"
        );
    }
    let item = js_fn(DISCARD_JS, "discardItem");
    assert!(
        item.contains("cell("),
        "a marker draws through `cell`: {item}"
    );
}

/// ADR-0087 R8: a discarded draft is never a note. Its marker is a `<details>` in its own list,
/// `#note-discards`, never `#note-list`; it is drawn with every pane and taken down with the chart.
#[test]
fn a_marker_is_a_details_not_a_note() {
    let render = js_fn(DISCARD_JS, "renderDiscards");
    assert!(
        render
            .contains("el(\"note-discards\").replaceChildren(...pane.discards.map(discardItem));"),
        "{render}"
    );
    assert!(!DISCARD_JS.contains("note-list"), "{DISCARD_JS}");
    let item = js_fn(DISCARD_JS, "discardItem");
    assert!(item.contains("\"details\""), "{item}");
    assert!(
        item.contains("cell(\"summary\", discard.summary)"),
        "{item}"
    );
    assert!(js_fn(NOTES_JS, "renderNotes").contains("renderDiscards(pane);"));
    assert!(js_fn(NOTES_JS, "clearNotes").contains("el(\"note-discards\").replaceChildren();"));
    assert!(
        position(INDEX_HTML, "<ol id=\"note-list\"></ol>")
            < position(
                INDEX_HTML,
                "<ul id=\"note-discards\" aria-label=\"Discarded drafts\"></ul>"
            )
    );
    // Loaded after notes.js, whose `setSigning`, `closeEditor` and `noteStatus` it calls.
    assert!(
        position(INDEX_HTML, "<script src=\"notes.js\"></script>")
            < position(INDEX_HTML, "<script src=\"notes_discard.js\"></script>")
    );
}

/// The drift guard every webview file carries (`commands::tests::fields_read_in`): every field
/// notes_discard.js reads off a payload is one the Rust type actually sends. A renamed field
/// would otherwise read `undefined` — for `discard.summary`, a marker with no words.
#[test]
fn notes_discard_js_reads_no_field_the_backend_does_not_send() {
    use crate::commands::tests::fields_read_in;
    use crate::notes::parts::{DiscardPartView, DiscardView, NotePane};
    use crate::notes::NoteSignReport;
    use crate::whos_here::{claim_view, Claim};
    let keys = |v: serde_json::Value| -> std::collections::BTreeSet<String> {
        v.as_object().expect("an object").keys().cloned().collect()
    };
    let pane = NotePane {
        notes: vec![],
        withheld_sentence: None,
        drafts: vec![],
        discards: vec![],
        claim: claim_view(&Claim::Unknown),
    };
    let discard = DiscardView {
        event_id: "e".into(),
        summary: "s".into(),
        parts: vec![],
    };
    let discard_part = DiscardPartView {
        heading: "h".into(),
        text: "t".into(),
    };
    let report = NoteSignReport {
        text: "t".into(),
        behind_the_record: None,
    };
    for (binding, available) in [
        ("pane", keys(serde_json::to_value(&pane).unwrap())),
        ("discard", keys(serde_json::to_value(&discard).unwrap())),
        (
            "discardPart",
            keys(serde_json::to_value(&discard_part).unwrap()),
        ),
        ("report", keys(serde_json::to_value(&report).unwrap())),
    ] {
        let read = fields_read_in(DISCARD_JS, binding);
        assert!(
            !read.is_empty(),
            "notes_discard.js no longer reads `{binding}`"
        );
        for field in read {
            assert!(
                available.contains(&field),
                "notes_discard.js reads `{binding}.{field}`, not sent"
            );
        }
    }
}

/// The discard writes as the writer captured when the editor opened, never the claim held now
/// (whos_here_webview_tests.rs pins the same for every save and the Sign): a discard landing
/// after a switch still names the person who typed it.
#[test]
fn a_discard_never_re_reads_the_claim() {
    let discard = js_fn(DISCARD_JS, "discardNote");
    assert!(discard.contains("writer: held.writer,"), "{discard}");
    assert!(!discard.contains("whosHere("), "{discard}");
}

/// Task 6 review, item 1. A who's-here switch waits for a Discard as for a Sign (they share
/// `noteSigning`), then says its outcome in the picker — in a discard's OWN words, never a Sign's.
/// The listener renames a discard's "unknown" (and its own fault) to "discard-unknown", so
/// `signOutcomeLine` can never word an unknown discard as an unknown signature.
#[test]
fn a_switch_during_a_discard_says_what_the_discard_did() {
    assert!(NOTES_JS.contains(
        "noteSigning = discardNote()\n    .then((outcome) => (outcome === \"unknown\" ? \"discard-unknown\" : outcome))\n    .catch((failure) => {\n      console.error(failure);\n      return \"discard-unknown\";"
    ));
    assert_eq!(
        js_const(WHOS_HERE_JS, "DISCARDED_LINE"),
        "Your draft was discarded."
    );
    assert_eq!(
        js_const(WHOS_HERE_JS, "NOT_DISCARDED_LINE"),
        "Your draft was NOT discarded \u{2014} open the chart to see where it stands."
    );
    assert_eq!(
        js_const(WHOS_HERE_JS, "DISCARD_UNKNOWN_LINE"),
        "Whether your draft was discarded is not known \u{2014} check the chart."
    );
    let line = js_fn(WHOS_HERE_JS, "signOutcomeLine");
    for arm in [
        "if (outcome === \"discarded\") return DISCARDED_LINE;",
        "if (outcome === \"not-discarded\") return NOT_DISCARDED_LINE;",
        "if (outcome === \"discard-unknown\") return DISCARD_UNKNOWN_LINE;",
    ] {
        assert!(line.contains(arm), "{line}");
    }
    // discardNote's own answers are the ones the lines name (plus "unknown", renamed above).
    let discard = js_fn(DISCARD_JS, "discardNote");
    for answer in ["\"discarded\"", "\"not-discarded\"", "\"unknown\""] {
        assert!(discard.contains(answer), "{answer} in {discard}");
    }
    // The idle re-lock only WAITS for the press; it words no outcome, so it has none to get wrong.
    let relock = js_fn(IDLE_JS, "idleRelock");
    assert!(relock.contains("await signsSettled();"), "{relock}");
    assert!(!IDLE_JS.contains("signOutcomeLine"), "{IDLE_JS}");
}
