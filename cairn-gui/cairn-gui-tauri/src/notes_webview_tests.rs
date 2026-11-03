//! Sign-in slice 3a, Task 6: the webview's note editor (`notes.js`).
//!
//! The window's scripts have no test rig (no npm or bundler: main.js's header, issue #332), so,
//! like `chart_view_webview_tests.rs`, each guard reads the script's source
//! (`crate::source_guard`) and pins the ORDER or the CHOICE a property is. The behaviour was
//! walked headlessly (a stub `invoke` in a browser) when it was written — see the PR that added
//! this file.
use crate::source_guard::{js_const, js_fn, position};

const NOTES_JS: &str = include_str!("../src-ui/notes.js");
const MAIN_JS: &str = include_str!("../src-ui/main.js");
const INDEX_HTML: &str = include_str!("../src-ui/index.html");

/// Review Focus 3: a debounced save that fires after Sign must never be sent. `signNote` cancels
/// the pending timer BEFORE it sends the sign (the node answers `spent` to a late save anyway;
/// this is the window's half).
#[test]
fn sign_cancels_the_pending_save_before_it_signs() {
    let sign = js_fn(NOTES_JS, "signNote");
    assert!(
        position(sign, "clearTimeout(noteSaveTimer);") < position(sign, "invoke(\"note_sign\""),
        "{sign}"
    );
    // The sign sends the text ON SCREEN with it (Review Focus 4), never an older autosave.
    assert!(
        sign.contains("narrative: el(\"note-text\").value,"),
        "{sign}"
    );
    // A partly typed time reads as "" — sent, it would sign as "not stated". Refused first.
    assert!(
        position(sign, "if (when.validity.badInput) {") < position(sign, "invoke(\"note_sign\""),
        "{sign}"
    );
}

/// Review Focus 2: the connection drops while typing. The text stays in the editor (nothing in
/// the failure path touches the field) and the status line says the exact sentence.
#[test]
fn a_failed_save_keeps_the_text_and_says_so() {
    assert_eq!(
        js_const(NOTES_JS, "NOT_SAVED"),
        "not saved yet \u{2014} your text is still here"
    );
    let send = js_fn(NOTES_JS, "sendDraft");
    let caught = &send[position(send, "catch (failure) {")..];
    assert!(caught.contains("saySaved(NOT_SAVED);"), "{send}");
    assert!(
        !send.contains(".value ="),
        "a save never writes the field: {send}"
    );
    // A save that wrote nothing (spent, elsewhere, no chart) says the node's own sentence.
    assert!(send.contains("view.saved ? \"\" : view.sentence"), "{send}");
    // A save that worked takes down only what a save said: a Sign's refusal, said just before
    // the save its catch sends, stays up (found in the headless walk).
    let said = js_fn(NOTES_JS, "saySaved");
    assert!(
        said.contains(
            "if (text || el(\"note-status\").textContent === saveSaid) noteStatus(text);"
        ),
        "{said}"
    );
}

/// The text a clinician typed is shown as text, never parsed as markup (ADR-0041: the window
/// escapes only at signing, and shows back exactly what was typed).
#[test]
fn notes_and_drafts_are_drawn_as_text() {
    for markup in [
        "innerHTML",
        "outerHTML",
        "insertAdjacentHTML",
        "document.write",
    ] {
        assert!(!NOTES_JS.contains(markup), "notes.js uses {markup}");
    }
    for name in ["noteItem", "draftItem"] {
        let body = js_fn(NOTES_JS, name);
        assert!(
            body.contains("cell("),
            "{name} draws through `cell` (textContent): {body}"
        );
    }
}

/// Draft prose lives in the node, behind definer doors. A browser-storage copy would put it on
/// this machine's disk in the clear.
#[test]
fn notes_js_never_touches_browser_storage() {
    for store in ["localStorage", "sessionStorage", "indexedDB"] {
        assert!(!NOTES_JS.contains(store), "notes.js uses {store}");
    }
}

/// A draft started at THIS terminal reopens into the editor when the pane is drawn (if the
/// editor is free); one started elsewhere is listed as its words and text only — no editor, no
/// Sign.
#[test]
fn a_here_draft_reopens_and_another_terminals_draft_is_read_only() {
    let render = js_fn(NOTES_JS, "renderNotes");
    assert!(
        render.contains("if (draft.here && noteDraft === null) openDraft(draft);"),
        "{render}"
    );
    let item = js_fn(NOTES_JS, "draftItem");
    for control in ["\"button\"", "\"textarea\"", "\"input\"", "note-sign"] {
        assert!(
            !item.contains(control),
            "a listed draft has no {control}: {item}"
        );
    }
    let open = js_fn(NOTES_JS, "openDraft");
    assert!(
        open.contains("el(\"note-text\").value = draft.narrative;"),
        "{open}"
    );
}

/// One save a second while typing; a new draft's id is minted in the window.
#[test]
fn the_debounce_is_one_second_and_draft_ids_are_minted_here() {
    assert!(NOTES_JS.contains("const NOTE_SAVE_DEBOUNCE_MS = 1000;"));
    let input = js_fn(NOTES_JS, "onNoteInput");
    assert!(
        input.contains("setTimeout(saveNoteNow, NOTE_SAVE_DEBOUNCE_MS)"),
        "{input}"
    );
    assert!(js_fn(NOTES_JS, "newNote").contains("crypto.randomUUID()"));
}

/// Every chart read reads the notes too, and a chart change takes the pane (and the editor's
/// draft) down, so one patient's notes are never under another's header.
#[test]
fn the_chart_read_reads_the_notes_and_a_chart_change_clears_them() {
    assert!(
        js_fn(MAIN_JS, "refresh").contains("void readNotes(patient);"),
        "refresh reads the note pane"
    );
    assert!(js_fn(MAIN_JS, "clearChart").contains("clearNotes();"));
    // The chart change flushes the last second of typing to its own draft's chart, and that save
    // is SENT before `closeChart` sends `close_chart` (an idle save goes out at once, not on a
    // later tick — the walk caught it arriving after the chart had closed, so refused).
    assert!(
        js_fn(NOTES_JS, "clearNotes").contains("if (noteSaveTimer !== null) void saveNoteNow();")
    );
    let save = js_fn(NOTES_JS, "saveNoteNow");
    assert!(
        save.contains("noteSaveQueue = savesInFlight === 1 ? send() : noteSaveQueue.then(send);"),
        "{save}"
    );
    let read = js_fn(NOTES_JS, "readNotes");
    assert!(
        read.contains("invoke(\"note_list\", { patientId: patient })"),
        "{read}"
    );
    // An answer for a chart since left, or overtaken by a newer read, is dropped.
    assert!(
        read.contains("patient === displayedPatient && revision === notesRevision"),
        "{read}"
    );
}

/// The editor is labelled for VoiceOver, the status line is polite-live, notes.js loads after
/// signer.js (it calls `signerCredential`/`clearSigner` at a press), and nothing asks to confirm.
#[test]
fn the_editor_is_labelled_and_never_asks_to_confirm() {
    for needle in [
        "<section id=\"notes\"",
        "<label for=\"note-text\">Progress note</label>",
        "<label for=\"note-when\">When it happened \u{2014} leave empty if it is now</label>",
        "<input id=\"note-when\" type=\"datetime-local\"",
        "<p id=\"note-status\" role=\"status\" aria-live=\"polite\"></p>",
        "<button id=\"new-note\" type=\"button\">New note</button>",
        "<button id=\"note-sign\" type=\"button\">Sign note</button>",
    ] {
        assert!(INDEX_HTML.contains(needle), "index.html lacks {needle}");
    }
    assert!(
        position(INDEX_HTML, "<script src=\"signer.js\"></script>")
            < position(INDEX_HTML, "<script src=\"notes.js\"></script>")
    );
    assert!(
        position(INDEX_HTML, "<table id=\"med-table\">")
            < position(INDEX_HTML, "<section id=\"notes\""),
        "the notes sit under the medication list"
    );
    for dialog in ["confirm(", "alert(", "prompt(", "<dialog"] {
        assert!(!NOTES_JS.contains(dialog), "notes.js uses {dialog}");
    }
}

/// The drift guard every webview file carries (`commands::tests::fields_read_in`): every field
/// notes.js reads off a payload is one the Rust type actually sends. A renamed field would
/// otherwise read `undefined` — for `draft.here`, every draft of this terminal shown read-only;
/// for `withheld_sentence`, held-back notes withheld in silence.
#[test]
fn notes_js_reads_no_field_the_backend_does_not_send() {
    use crate::commands::tests::fields_read_in;
    use crate::notes::{DraftView, NotePane, NoteRowView, NoteSignReport, SaveView};
    let keys = |v: serde_json::Value| -> std::collections::BTreeSet<String> {
        v.as_object().expect("an object").keys().cloned().collect()
    };
    let note = NoteRowView {
        event_id: "e".into(),
        text: "t".into(),
        byline: "b".into(),
        when: "w".into(),
    };
    let draft = DraftView {
        draft_id: "d".into(),
        narrative: "n".into(),
        stated_time: None,
        here: true,
        words: "w".into(),
    };
    let pane = NotePane {
        notes: vec![],
        withheld_sentence: None,
        drafts: vec![],
    };
    let view = SaveView {
        saved: true,
        sentence: None,
    };
    let report = NoteSignReport {
        text: "t".into(),
        behind_the_record: None,
    };
    for (binding, available) in [
        ("pane", keys(serde_json::to_value(&pane).unwrap())),
        ("note", keys(serde_json::to_value(&note).unwrap())),
        ("draft", keys(serde_json::to_value(&draft).unwrap())),
        ("view", keys(serde_json::to_value(&view).unwrap())),
        ("report", keys(serde_json::to_value(&report).unwrap())),
    ] {
        let read = fields_read_in(NOTES_JS, binding);
        assert!(!read.is_empty(), "notes.js no longer reads `{binding}`");
        for field in read {
            assert!(
                available.contains(&field),
                "notes.js reads `{binding}.{field}`, not sent"
            );
        }
    }
}
