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
const FUNNEL_JS: &str = include_str!("../src-ui/funnel.js");

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

/// Ruling R14 (final review M1): at most one save a second WHILE typing, never only after a
/// pause. The first keystroke of a burst saves at once (so `recording_started` is when writing
/// began); later ones wait for the second since the last save to pass, and the timer they set is
/// never reset by the next keystroke, so continuous typing still saves every second (design
/// §3.2: a power cut loses at most that interval). The timer, read when it fires, is the trailing
/// save after the last keystroke. A new draft's id is minted in the window.
#[test]
fn saves_go_out_once_a_second_while_typing_and_draft_ids_are_minted_here() {
    assert!(NOTES_JS.contains("const NOTE_SAVE_DEBOUNCE_MS = 1000;"));
    let input = js_fn(NOTES_JS, "onNoteInput");
    for line in [
        "if (noteSaveTimer !== null) return;",
        "const wait = noteLastSave + NOTE_SAVE_DEBOUNCE_MS - performance.now();",
        "if (wait <= 0) void saveNoteNow();",
        "else noteSaveTimer = setTimeout(saveNoteNow, wait);",
    ] {
        assert!(input.contains(line), "{input}");
    }
    assert!(
        !input.contains("clearTimeout"),
        "a keystroke never pushes the pending save back: {input}"
    );
    assert!(js_fn(NOTES_JS, "saveNoteNow").contains("noteLastSave = performance.now();"));
    // A fresh draft's first keystroke saves at once, whenever the last draft's save went.
    assert!(js_fn(NOTES_JS, "closeEditor").contains("noteLastSave = -Infinity;"));
    assert!(js_fn(NOTES_JS, "newNote").contains("crypto.randomUUID()"));
}

/// Final review M7: after a Sign that worked, the re-read may reopen another of this terminal's
/// drafts on the chart. Its words are ADDED to the status line, never put in place of "Signed —
/// the note is on the chart.", so the clinician still sees that the sign landed.
#[test]
fn a_reopened_draft_never_hides_the_signed_confirmation() {
    let open = js_fn(NOTES_JS, "openDraft");
    assert!(
        open.contains("noteStatus(said ? said + \" · \" + draft.words : draft.words);"),
        "{open}"
    );
    assert!(!open.contains("noteStatus(draft.words)"), "{open}");
    let sign = js_fn(NOTES_JS, "signNote");
    assert!(
        position(sign, "noteStatus(report.text); // worded in Rust")
            < sign
                .rfind("await refresh(report.text, report.behind_the_record);")
                .unwrap(),
        "the confirmation is said before the re-read that may reopen a draft: {sign}"
    );
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

/// Fix round 1, I1. Closing a chart waits for every queued save BEFORE it sends `close_chart`.
/// Being SENT first is not enough: Tauri runs async commands as separate tasks, so a save sent
/// before `close_chart` may still take the chart lock after it, and be refused. The queue never
/// rejects (`sendDraft` catches everything), so the close always goes on.
#[test]
fn closing_a_chart_waits_for_every_queued_save() {
    let close = js_fn(FUNNEL_JS, "closeChart");
    let waited = position(
        close,
        "if (typeof notesSaved === \"function\") await notesSaved();",
    );
    assert!(position(close, "clearChart();") < waited, "{close}");
    assert!(
        waited < position(close, "invoke(\"close_chart\")"),
        "{close}"
    );
    assert!(js_fn(NOTES_JS, "notesSaved").contains("return noteSaveQueue;"));
    let save = js_fn(NOTES_JS, "saveNoteNow");
    assert!(
        save.contains("noteSaveQueue = noteSaveQueue.then(() => sendDraft(sent));"),
        "{save}"
    );
    assert!(
        !NOTES_JS.contains("savesInFlight"),
        "the send-at-once special case is gone"
    );
}

/// Fix round 1, I2. A partly typed time is never saved as "not stated": the save sends the last
/// COMPLETE time the draft held — the one it reopened with, or the last complete one typed.
#[test]
fn a_partly_typed_time_saves_the_last_complete_one() {
    let save = js_fn(NOTES_JS, "saveNoteNow");
    assert!(
        save.contains(
            "statedTime: when.validity.badInput ? held.statedTime : (held.statedTime = when.value),"
        ),
        "{save}"
    );
    assert!(
        js_fn(NOTES_JS, "openDraft").contains("statedTime: draft.stated_time || \"\""),
        "a reopened draft holds the time it was saved with"
    );
    assert!(js_fn(NOTES_JS, "newNote").contains("statedTime: \"\""));
}

/// Fix round 1, I3 (ruling R13). While a Sign is in flight the editor is read-only: a keystroke
/// then would be in neither the signed note nor any draft. It comes back on a refusal and with
/// every editor shown; closing the editor drops any pending save timer.
#[test]
fn the_editor_is_read_only_while_a_sign_is_in_flight() {
    let signing = js_fn(NOTES_JS, "setSigning");
    for line in [
        "el(\"note-sign\").disabled = on;",
        "el(\"note-text\").readOnly = on;",
        "el(\"note-when\").readOnly = on;",
    ] {
        assert!(signing.contains(line), "{signing}");
    }
    let sign = js_fn(NOTES_JS, "signNote");
    assert!(
        position(sign, "setSigning(true);") < position(sign, "invoke(\"note_sign\""),
        "{sign}"
    );
    let caught = &sign[position(sign, "catch (failure) {")..];
    assert!(caught.contains("setSigning(false);"), "{sign}");
    assert!(js_fn(NOTES_JS, "showEditor").contains("setSigning(false);"));
    assert!(js_fn(NOTES_JS, "closeEditor").contains("clearTimeout(noteSaveTimer);"));
}

/// Fix round 1, I4. Text not yet in the node survives a chart change: a keystroke or a failed save
/// marks the editor unsaved, an answered save clears it, and the close flushes on either. Sign's
/// partly-typed-time refusal comes BEFORE it cancels the pending save, so the save still goes. An
/// unknown-outcome Sign keeps the editor unless the save before closing it was answered.
#[test]
fn unsaved_text_survives_a_chart_change() {
    assert!(js_fn(NOTES_JS, "onNoteInput").contains("noteUnsaved = true;"));
    let send = js_fn(NOTES_JS, "sendDraft");
    let caught = &send[position(send, "catch (failure) {")..];
    assert!(caught.contains("noteUnsaved = true;"), "{send}");
    assert!(
        send[..position(send, "catch (failure) {")].contains("noteUnsaved = false;"),
        "{send}"
    );
    assert!(js_fn(NOTES_JS, "clearNotes")
        .contains("if (noteUnsaved || noteSaveTimer !== null) void saveNoteNow();"));
    let sign = js_fn(NOTES_JS, "signNote");
    assert!(
        position(sign, "if (when.validity.badInput) {")
            < position(sign, "clearTimeout(noteSaveTimer);"),
        "{sign}"
    );
    let unknown = position(
        sign,
        "if (report.behind_the_record && !(await saveNoteNow())) {",
    );
    assert!(unknown < position(sign, "closeEditor();"), "{sign}");
}
