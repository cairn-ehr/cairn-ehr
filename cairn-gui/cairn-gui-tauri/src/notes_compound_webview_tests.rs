//! Sign-in slice 3c-1, Task 6: the webview's compound note and "Add to this note"
//! (`notes_compound.js`, and the target `notes.js`, `notes_save.js` and `notes_discard.js` carry).
//!
//! The window's scripts have no test rig (no npm or bundler: main.js's header, issue #332), so,
//! like `notes_webview_tests.rs`, each guard reads the script's source (`crate::source_guard`) and
//! pins the CHOICE a property is. The behaviour was walked headlessly (a stub `invoke` in Chromium
//! and WebKit) when it was written — see the slice's PR.
use crate::source_guard::{js_fn, position};

const COMPOUND_JS: &str = include_str!("../src-ui/notes_compound.js");
const NOTES_JS: &str = include_str!("../src-ui/notes.js");
const SAVE_JS: &str = include_str!("../src-ui/notes_save.js");
const DISCARD_JS: &str = include_str!("../src-ui/notes_discard.js");
const INDEX_HTML: &str = include_str!("../src-ui/index.html");

/// "Add to this note" is offered only on a note of the chart on screen (Rust's `can_add`) and only
/// while the editor is free — one draft is written at a time, as with New note. Its accessible
/// name says WHICH note, since a list of notes would otherwise read as a list of identical buttons.
#[test]
fn add_is_offered_only_on_this_charts_note_while_the_editor_is_free() {
    let item = js_fn(COMPOUND_JS, "compoundItem");
    let offered = position(item, "if (compound.can_add && noteDraft === null) {");
    assert!(
        offered < position(item, "cell(\"button\", \"Add to this note\""),
        "{item}"
    );
    assert!(
        item.contains("\"aria-label\": \"Add to this note: \" + compound.when"),
        "{item}"
    );
}

/// Adding to a note needs somebody to have said who is writing, exactly as New note does: the
/// picker asks first and the editor opens once it is answered. The draft names the ROOT it adds
/// to, and the editor says Rust's own line for it.
#[test]
fn add_asks_whos_here_first_and_names_the_root() {
    let add = js_fn(COMPOUND_JS, "addToNote");
    let free = position(
        add,
        "if (displayedPatient === null || noteDraft !== null) return;",
    );
    let asked = position(add, "if (!claimAsked()) {");
    let ask = position(add, "askWhosHere(() => addToNote(compound));");
    let start = position(
        add,
        "startNote(compound.root_event_id, compound.adds_line);",
    );
    assert!(free < asked && asked < ask && ask < start, "{add}");
    assert!(
        add[ask..start].contains("return;"),
        "nothing opens before the picker is answered: {add}"
    );
    // New note opens the same editor, as a new note.
    assert!(js_fn(NOTES_JS, "newNote").contains("startNote(null, null);"));
    let start = js_fn(NOTES_JS, "startNote");
    assert!(start.contains("addendumTo: addendumTo,"), "{start}");
    assert!(start.contains("crypto.randomUUID()"), "{start}");
}

/// Review Focus 2: every save, Sign and Discard carries the draft's target, so one pressed inside
/// the debounce (no header yet) still makes an addendum, never a new note. Every invoke of the
/// three commands in every note script, so a new call site cannot go out without it.
#[test]
fn every_save_sign_and_discard_carries_the_target() {
    for command in ["note_draft_save", "note_sign", "note_discard"] {
        let mut calls = 0;
        for js in [NOTES_JS, SAVE_JS, DISCARD_JS, COMPOUND_JS] {
            for (at, _) in js.match_indices(&format!("invoke(\"{command}\"")) {
                let args = &js[at..at + position(&js[at..], "});")];
                assert!(args.contains("addendumTo: "), "{args}");
                calls += 1;
            }
        }
        assert!(calls > 0, "{command} is invoked");
    }
    assert!(js_fn(SAVE_JS, "sendDraft").contains("addendumTo: sent.held.addendumTo,"));
    assert!(js_fn(NOTES_JS, "signNote").contains("addendumTo: held.addendumTo,"));
    assert!(js_fn(DISCARD_JS, "discardNote").contains("addendumTo: held.addendumTo,"));
}

/// Review Focus 5: a draft reopened after a restart is still an addendum — its target is the
/// node's, copied verbatim — and its editor says what it adds to (Rust's line, never built here).
#[test]
fn a_reopened_draft_keeps_its_target_and_its_line() {
    let open = js_fn(NOTES_JS, "openDraft");
    assert!(open.contains("addendumTo: draft.addendum_to,"), "{open}");
    assert!(
        position(open, "addendumTo: draft.addendum_to,")
            < position(open, "showAddsTo(draft.adding_to);"),
        "{open}"
    );
    // In the drafts list, the line is its own paragraph, above the draft's words.
    let item = js_fn(NOTES_JS, "draftItem");
    assert!(
        position(item, "cell(\"p\", draft.adding_to)") < position(item, "cell(\"p\", draft.words)"),
        "{item}"
    );
}

/// The "Adding to…" line sits above the text box and is shown only for an addendum draft —
/// toggled by `hidden`, never `style.display` (the page's reset hides `[hidden]`); closing the
/// editor takes it down, so the next New note never says it adds to anything.
#[test]
fn the_adding_to_line_shows_only_for_an_addendum() {
    let line = "<p id=\"note-adds-to\" hidden></p>";
    assert!(
        position(INDEX_HTML, "<div id=\"note-editor\" hidden>") < position(INDEX_HTML, line)
            && position(INDEX_HTML, line) < position(INDEX_HTML, "<textarea id=\"note-text\""),
        "the line is inside the editor, above the text box"
    );
    let show = js_fn(NOTES_JS, "showAddsTo");
    assert!(
        show.contains("addsTo.textContent = line || \"\";"),
        "{show}"
    );
    assert!(
        show.contains("addsTo.hidden = noteDraft === null || noteDraft.addendumTo === null;"),
        "{show}"
    );
    let close = js_fn(NOTES_JS, "closeEditor");
    assert!(
        position(close, "noteDraft = null;") < position(close, "showAddsTo(null);"),
        "{close}"
    );
    assert!(js_fn(NOTES_JS, "startNote").contains("showAddsTo(addsTo);"));
    for js in [NOTES_JS, COMPOUND_JS] {
        assert!(!js.contains("style.display"), "visibility is `hidden`");
    }
}

/// A compound reads as one text, with its parts behind a `<details>` disclosure (keyboard-operable
/// by default) each drawn as the note it is (`noteItem`, whose byline is heard in its spoken form,
/// #973). Every word is Rust's, set as text — the typed text is never parsed as markup (ADR-0041).
#[test]
fn a_compound_is_one_text_with_its_parts_behind_a_disclosure() {
    assert!(js_fn(NOTES_JS, "renderNotes").contains("pane.notes.map(compoundItem)"));
    let item = js_fn(COMPOUND_JS, "compoundItem");
    for line in [
        "const li = noteItem(compound);",
        "if (compound.state) li.append(cell(\"p\", compound.state));",
        "const details = document.createElement(\"details\");",
        "parts.append(...compound.parts.map(noteItem));",
        "details.append(cell(\"summary\", compound.parts_label), parts);",
    ] {
        assert!(item.contains(line), "{line} in {item}");
    }
    for markup in [
        "innerHTML",
        "outerHTML",
        "insertAdjacentHTML",
        "document.write",
    ] {
        assert!(
            !COMPOUND_JS.contains(markup),
            "notes_compound.js uses {markup}"
        );
    }
    for store in ["localStorage", "sessionStorage", "indexedDB"] {
        assert!(
            !COMPOUND_JS.contains(store),
            "notes_compound.js uses {store}"
        );
    }
    // Loaded before notes.js, beside the other note scripts.
    assert!(
        position(INDEX_HTML, "<script src=\"notes_save.js\"></script>")
            < position(INDEX_HTML, "<script src=\"notes_compound.js\"></script>")
            && position(INDEX_HTML, "<script src=\"notes_compound.js\"></script>")
                < position(INDEX_HTML, "<script src=\"notes.js\"></script>")
    );
}

/// The drift guard (`commands::tests::fields_read_in`): every field notes_compound.js reads off a
/// compound is one Rust's `CompoundView` sends — a renamed `can_add` would otherwise read
/// `undefined`, and no note could ever be added to.
#[test]
fn notes_compound_js_reads_no_field_the_backend_does_not_send() {
    use crate::commands::tests::fields_read_in;
    use crate::notes::compound::CompoundView;
    let compound = CompoundView {
        root_event_id: "e".into(),
        text: "t".into(),
        byline: "b".into(),
        byline_spoken: "b".into(),
        when: "w".into(),
        state: None,
        parts_label: None,
        parts: vec![],
        can_add: true,
        adds_line: "a".into(),
    };
    let value = serde_json::to_value(&compound).unwrap();
    let available = value.as_object().expect("an object");
    let read = fields_read_in(COMPOUND_JS, "compound");
    assert!(!read.is_empty(), "notes_compound.js reads `compound`");
    for field in read {
        assert!(
            available.contains_key(&field),
            "notes_compound.js reads `compound.{field}`, not sent"
        );
    }
}

/// Fix round 1: a pane read that reopens a here-draft opens the editor INSIDE `renderNotes`, so
/// the notes are drawn after it — drawn before, every note would keep an "Add to this note" that
/// does nothing (the drafts' "Continue here" is drawn after it for the same reason).
#[test]
fn the_notes_are_drawn_after_a_draft_reopens() {
    let render = js_fn(NOTES_JS, "renderNotes");
    let reopen = position(render, "openDraft(draft, claim.staff_id)");
    assert!(
        reopen < position(render, "pane.notes.map(compoundItem)"),
        "{render}"
    );
    assert!(
        reopen < position(render, "listed.map(draftItem)"),
        "{render}"
    );
}
