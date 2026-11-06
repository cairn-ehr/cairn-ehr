//! Sign-in slice 3c-2, Task 6: the webview's "Sign off this note" (`notes_compound.js`), the
//! editor's "also covers" line and the ids Sign sends with it (`notes.js`), and the `student`
//! enrolment role (`index.html`).
//!
//! The window's scripts have no test rig (no npm or bundler: main.js's header, issue #332), so,
//! like `notes_compound_webview_tests.rs`, each guard reads the script's source
//! (`crate::source_guard`) and pins the ORDER or the CHOICE a property is. The behaviour was walked
//! headlessly (a stub `invoke` in a browser) when it was written — see the slice's PR.
use crate::source_guard::{js_fn, position};

const COMPOUND_JS: &str = include_str!("../src-ui/notes_compound.js");
/// Every note script, as one text: a new `invoke("note_sign"` anywhere must carry the strip's ids.
const NOTE_SCRIPTS: &str = concat!(
    include_str!("../src-ui/notes.js"),
    include_str!("../src-ui/notes_save.js"),
    include_str!("../src-ui/notes_discard.js"),
    include_str!("../src-ui/notes_compound.js")
);
const NOTES_JS: &str = include_str!("../src-ui/notes.js");
const INDEX_HTML: &str = include_str!("../src-ui/index.html");

/// "Sign off this note" is drawn only on a compound Rust says needs it (`can_sign_off`), and only
/// while the editor is free, as "Add to this note" is. The press checks again: a button drawn
/// before the editor opened, or while a Sign or Discard is in flight, signs nothing (one strip
/// credential serves one act at a time).
#[test]
fn sign_off_is_offered_only_when_the_view_says_so() {
    let buttons = js_fn(COMPOUND_JS, "withButtons");
    assert!(
        buttons.contains(
            "if (compound.can_sign_off && noteDraft === null) li.append(signOffButton(compound));"
        ),
        "{buttons}"
    );
    let press = js_fn(COMPOUND_JS, "pressSignOff");
    assert!(
        position(
            press,
            "if (displayedPatient === null || noteDraft !== null || signInFlight()) return;"
        ) < position(press, "signOffNote(compound)"),
        "{press}"
    );
    // Both shapes of compound get their buttons.
    let item = js_fn(COMPOUND_JS, "compoundItem");
    assert!(
        item.contains("return withButtons(noteItem(compound), compound);"),
        "{item}"
    );
    assert!(item.contains("return withButtons(li, compound);"), "{item}");
}

/// The press sends the part ids the pane SHOWED, verbatim (the node refuses `Changed` if the note
/// now holds others), and the strip's credential; the strip is cleared both ways out, as
/// notes_discard.js does — remembered as a signer only on the success path.
#[test]
fn sign_off_sends_the_shown_parts_and_the_strips_credential() {
    let sign_off = js_fn(COMPOUND_JS, "signOffNote");
    let sent = position(sign_off, "invoke(\"note_sign_off\", {");
    for line in [
        "patientId: patient,",
        "rootEventId: compound.root_event_id,",
        "shownParts: compound.shown_parts,",
        "credential: signerCredential(),",
    ] {
        assert!(sent < position(sign_off, line), "{line} in {sign_off}");
    }
    let caught = position(sign_off, "} catch (failure) {");
    let cleared = position(sign_off, "clearSigner(true);");
    assert!(sent < cleared && cleared < caught, "{sign_off}");
    assert!(
        sign_off[caught..].contains("clearSigner();"),
        "a refusal clears the strip too: {sign_off}"
    );
}

/// Signing off needs somebody to have said who is here, as Add does: the picker asks first and
/// the press goes once it is answered. Only then is the button taken down and the press tracked
/// on the note pane's in-flight flags, so idle.js's re-lock and whos_here.js's switch wait for it
/// as for a Sign. Its outcome is said on the status line and the pane read again on EVERY path —
/// success, refusal and unknown outcome — after the strip is cleared.
#[test]
fn sign_off_sits_behind_the_whos_here_gate() {
    let press = js_fn(COMPOUND_JS, "pressSignOff");
    let asked = position(press, "if (!claimAsked()) {");
    let ask = position(press, "askWhosHere(() => pressSignOff(compound, button));");
    let down = position(press, "button.disabled = true;");
    let flagged = position(press, "noteSignInFlight = true;");
    let tracked = position(press, "noteSigning = signOffNote(compound)");
    assert!(
        asked < ask && ask < down && down < flagged && flagged < tracked,
        "{press}"
    );
    assert!(
        press[ask..down].contains("return;"),
        "nothing is sent before the picker is answered: {press}"
    );
    let settled = &press[tracked..];
    for line in ["noteSignInFlight = false;", "button.disabled = false;"] {
        assert!(settled.contains(line), "{line} in {press}");
    }
    let sign_off = js_fn(COMPOUND_JS, "signOffNote");
    let caught = position(sign_off, "} catch (failure) {");
    let said = position(sign_off, "noteStatus(said);");
    let reread = position(sign_off, "await refresh(said, behind);");
    assert!(
        caught < said && said < reread,
        "one path after the try: {sign_off}"
    );
    // Rust's words, never prefixed here: a refusal is already a whole sentence.
    assert!(
        sign_off[caught..said].contains("said = failureText(failure);"),
        "{sign_off}"
    );
}

/// A list of notes is otherwise a list of identical buttons: the accessible name says WHICH note.
/// The label and its prefix are this file's only words.
#[test]
fn its_accessible_name_is_sign_off_this_note_plus_the_time() {
    let button = js_fn(COMPOUND_JS, "signOffButton");
    assert!(
        button.contains(
            "cell(\"button\", \"Sign off this note\", { type: \"button\", \"aria-label\": \"Sign off this note: \" + compound.when })"
        ),
        "{button}"
    );
    assert!(
        button.contains("pressSignOff(compound, button)"),
        "{button}"
    );
}

/// The strip's line: Rust's words below "Adding to …", set as text and shown only when there is
/// one; it comes from the compound when Add opens the editor and from the draft when one reopens.
/// The ids it names live with the draft (a fork copies them, `{ ...sent.held }`), and EVERY Sign
/// sends them, so what the strip showed is what Sign vouches; a new note sends none.
#[test]
fn the_covers_line_is_text_and_sign_sends_its_ids() {
    let line = "<p id=\"note-covers\" hidden></p>";
    assert!(
        position(INDEX_HTML, "<p id=\"note-adds-to\" hidden></p>") < position(INDEX_HTML, line)
            && position(INDEX_HTML, line) < position(INDEX_HTML, "<textarea id=\"note-text\""),
        "the line is in the editor, below \"Adding to …\", above the text box"
    );
    let show = js_fn(NOTES_JS, "showCovers");
    assert!(
        show.contains("covers.textContent = line || \"\";"),
        "{show}"
    );
    assert!(show.contains("covers.hidden = !line;"), "{show}");
    assert!(
        js_fn(COMPOUND_JS, "addToNote").contains(
            "startNote(compound.root_event_id, compound.adds_line, compound.covers_line, compound.covers);"
        )
    );
    let start = js_fn(NOTES_JS, "startNote");
    assert!(start.contains("covers: covers || [],"), "{start}");
    assert!(start.contains("showCovers(coversLine);"), "{start}");
    let open = js_fn(NOTES_JS, "openDraft");
    assert!(open.contains("covers: draft.covers,"), "{open}");
    assert!(open.contains("showCovers(draft.covers_line);"), "{open}");
    assert!(js_fn(NOTES_JS, "closeEditor").contains("showCovers(null);"));
    assert!(js_fn(NOTES_JS, "signNote").contains("alsoVouches: held.covers,"));
    let mut calls = 0;
    for (at, _) in NOTE_SCRIPTS.match_indices("invoke(\"note_sign\"") {
        let args = &NOTE_SCRIPTS[at..at + position(&NOTE_SCRIPTS[at..], "});")];
        assert!(args.contains("alsoVouches: "), "{args}");
        calls += 1;
    }
    assert!(calls > 0, "note_sign is invoked");
    for markup in ["innerHTML", "outerHTML", "insertAdjacentHTML"] {
        assert!(
            !NOTE_SCRIPTS.contains(markup),
            "a note script uses {markup}"
        );
    }
}

/// Principle 3: a confirmation dialog is not a safety mechanism. A sign-off is one press; the node
/// refusing a note that changed since it was shown is the safety.
#[test]
fn no_dialog() {
    for dialog in ["confirm(", "alert(", "prompt("] {
        assert!(
            !COMPOUND_JS.contains(dialog),
            "notes_compound.js uses {dialog}"
        );
    }
    assert!(!INDEX_HTML.contains("<dialog"));
}

/// A student is enrolled as one, after nurse in the list, so the node can word "(student)" and
/// "not co-signed" (the vault already grants `clinical` to every role but superuser/admin). A
/// re-key never changes a role, so the re-key form offers it too — or a student who forgot their
/// passphrase could not be re-keyed here.
#[test]
fn the_student_role_is_offered() {
    for id in ["enrol-role", "rekey-role"] {
        let select = &INDEX_HTML[position(INDEX_HTML, &format!("<select id=\"{id}\">"))..];
        let select = &select[..position(select, "</select>")];
        assert!(
            position(select, "<option value=\"nurse\">nurse</option>")
                < position(select, "<option value=\"student\">student</option>"),
            "{select}"
        );
    }
}
