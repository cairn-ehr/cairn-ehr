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
    // Both come back only when the press has SETTLED, whatever its outcome: inside `.finally(`.
    let settled = position(press, ".finally(");
    assert!(tracked < settled, "{press}");
    for line in ["noteSignInFlight = false;", "button.disabled = false;"] {
        assert!(settled < position(press, line), "{line} in {press}");
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

/// Principle 3: a confirmation dialog is not a safety mechanism. A sign-off — or a "Co-sign as
/// reviewed" (slice 3c-3a) — is one press; the node refusing a note that changed since it was
/// shown is the safety.
#[test]
fn no_dialog() {
    for dialog in ["confirm(", "alert(", "prompt("] {
        assert!(
            !NOTE_SCRIPTS.contains(dialog),
            "a note script uses {dialog}"
        );
    }
    assert!(!INDEX_HTML.contains("<dialog"));
}

/// Fix round 1 (Important): a sign-off is the first note-pane act in flight with the editor FREE,
/// so every way to open the editor refuses while one is in flight — before anything else it does
/// (the picker included). Otherwise the editor's Sign would reuse the passphrase still in the
/// strip (one typing, two acts), and the two presses would overwrite each other's in-flight
/// flags, so a who's-here switch or the idle re-lock could close the chart with an outcome unsaid.
#[test]
fn nothing_opens_the_editor_while_a_press_is_in_flight() {
    let new_note = js_fn(NOTES_JS, "newNote");
    assert!(
        new_note.starts_with(
            "function newNote() {\n  if (displayedPatient === null || noteDraft !== null || signInFlight()) return;"
        ),
        "{new_note}"
    );
    let add = js_fn(COMPOUND_JS, "addToNote");
    assert!(
        add.starts_with(
            "function addToNote(compound) {\n  if (displayedPatient === null || noteDraft !== null || signInFlight()) return;"
        ),
        "{add}"
    );
    let go = js_fn(NOTES_JS, "continueHere");
    assert!(
        go.starts_with("function continueHere(draft) {\n  if (signInFlight()) return;"),
        "{go}"
    );
}

/// Fix round 1: the editor's Sign and Discard refuse to START while another press is in flight
/// (a draft can reopen into the editor on a re-read during a sign-off): the check comes before
/// the press claims the in-flight flags.
#[test]
fn sign_and_discard_refuse_to_start_while_a_press_is_in_flight() {
    for button in ["note-sign", "note-discard"] {
        let wired = &NOTES_JS[position(
            NOTES_JS,
            &format!("el(\"{button}\").addEventListener(\"click\", () => {{"),
        )..];
        let wired = &wired[..position(wired, "\n});")];
        assert!(
            position(wired, "if (signInFlight()) return;")
                < position(wired, "noteSignInFlight = true;"),
            "{wired}"
        );
    }
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

/// Final review (Important): a who's-here switch waits for a sign-off as for a Sign (they share
/// `noteSigning`), then closes the chart, so the press must SETTLE with its outcome or the
/// clinician never learns it: signed off, signed nothing (a refusal, a wrong passphrase, a node
/// failure), or not known (#814, or a fault in this window mid-press). The picker words each in
/// a sign-off's OWN words, never a Sign's, and none names a patient (the next person may read it).
#[test]
fn a_switch_during_a_sign_off_says_what_the_sign_off_did() {
    use crate::source_guard::js_const;
    const WHOS_HERE_JS: &str = include_str!("../src-ui/whos_here.js");
    let press = js_fn(COMPOUND_JS, "pressSignOff");
    assert!(
        press.contains(
            "noteSigning = signOffNote(compound)\n    .catch((failure) => {\n      console.error(failure);\n      return \"sign-off-unknown\";"
        ),
        "{press}"
    );
    assert!(
        !press.contains("=> null"),
        "the press never settles as null: {press}"
    );
    let sign_off = js_fn(COMPOUND_JS, "signOffNote");
    let caught = position(sign_off, "} catch (failure) {");
    assert!(
        sign_off[..caught].contains("outcome = behind ? \"sign-off-unknown\" : \"signed-off\";"),
        "{sign_off}"
    );
    assert!(
        sign_off[caught..].contains(
            "outcome = behind === \"unknown\" ? \"sign-off-unknown\" : \"not-signed-off\";"
        ),
        "{sign_off}"
    );
    // Said and re-read first, so the chart reads the outcome before the switch takes it away.
    assert!(
        position(sign_off, "await refresh(said, behind);") < position(sign_off, "return outcome;"),
        "{sign_off}"
    );
    assert_eq!(
        js_const(WHOS_HERE_JS, "SIGNED_OFF_LINE"),
        "Your sign-off went through: every part of the note now carries your signature."
    );
    assert_eq!(
        js_const(WHOS_HERE_JS, "NOT_SIGNED_OFF_LINE"),
        "Your sign-off signed nothing \u{2014} open the chart to see where the note stands."
    );
    assert_eq!(
        js_const(WHOS_HERE_JS, "SIGN_OFF_UNKNOWN_LINE"),
        "Whether your sign-off went through is not known \u{2014} check the chart."
    );
    let line = js_fn(WHOS_HERE_JS, "signOutcomeLine");
    for arm in [
        "if (outcome === \"signed-off\") return SIGNED_OFF_LINE;",
        "if (outcome === \"not-signed-off\") return NOT_SIGNED_OFF_LINE;",
        "if (outcome === \"sign-off-unknown\") return SIGN_OFF_UNKNOWN_LINE;",
    ] {
        assert!(line.contains(arm), "{line}");
    }
}

/// Sign-in slice 3c-3a: "Co-sign as reviewed" is drawn only on a compound Rust offers it on
/// (`can_review`: the who's-here claim supervises a writer whose signed part awaits a review),
/// and only while the editor is free, beside Sign off. The press checks again before anything
/// else, as Sign off's does (one strip credential serves one act at a time).
#[test]
fn review_is_offered_only_when_the_view_says_so() {
    let buttons = js_fn(COMPOUND_JS, "withButtons");
    assert!(
        buttons.contains(
            "if (compound.can_review && noteDraft === null) li.append(reviewButton(compound));"
        ),
        "{buttons}"
    );
    assert!(
        position(buttons, "signOffButton(compound)") < position(buttons, "reviewButton(compound)"),
        "{buttons}"
    );
}

/// The review's press mirrors Sign off's: refused while the editor holds a draft or another press
/// is in flight (`signInFlight()`, ADR-0092 decision 17), behind the who's-here gate, then the
/// button down and the press tracked on the note pane's flags until it SETTLES. It sends the part
/// ids the pane showed and the strip's credential, clears the strip both ways out, says Rust's
/// words and reads the chart again on every path.
#[test]
fn review_sits_behind_the_whos_here_gate_and_refuses_in_flight() {
    let press = js_fn(COMPOUND_JS, "pressReview");
    assert!(
        press.starts_with(
            "function pressReview(compound, button) {\n  if (displayedPatient === null || noteDraft !== null || signInFlight()) return;"
        ),
        "{press}"
    );
    let asked = position(press, "if (!claimAsked()) {");
    let ask = position(press, "askWhosHere(() => pressReview(compound, button));");
    let down = position(press, "button.disabled = true;");
    let flagged = position(press, "noteSignInFlight = true;");
    let tracked = position(press, "noteSigning = reviewNote(compound)");
    assert!(
        asked < ask && ask < down && down < flagged && flagged < tracked,
        "{press}"
    );
    assert!(press[ask..down].contains("return;"), "{press}");
    let settled = position(press, ".finally(");
    assert!(tracked < settled, "{press}");
    for line in ["noteSignInFlight = false;", "button.disabled = false;"] {
        assert!(settled < position(press, line), "{line} in {press}");
    }

    let review = js_fn(COMPOUND_JS, "reviewNote");
    let sent = position(review, "invoke(\"note_review_cosign\", {");
    for line in [
        "patientId: patient,",
        "rootEventId: compound.root_event_id,",
        "shownParts: compound.shown_parts,",
        "credential: signerCredential(),",
    ] {
        assert!(sent < position(review, line), "{line} in {review}");
    }
    let caught = position(review, "} catch (failure) {");
    let cleared = position(review, "clearSigner(true);");
    assert!(sent < cleared && cleared < caught, "{review}");
    assert!(review[caught..].contains("clearSigner();"), "{review}");
    let said = position(review, "noteStatus(said);");
    let reread = position(review, "await refresh(said, behind);");
    assert!(caught < said && said < reread, "{review}");
    assert!(
        review[caught..said].contains("said = failureText(failure);"),
        "{review}"
    );
}

/// A list of notes is otherwise a list of identical buttons: the accessible name says WHICH note.
#[test]
fn its_accessible_name_is_co_sign_as_reviewed_plus_the_time() {
    let button = js_fn(COMPOUND_JS, "reviewButton");
    assert!(
        button.contains(
            "cell(\"button\", \"Co-sign as reviewed\", { type: \"button\", \"aria-label\": \"Co-sign as reviewed: \" + compound.when })"
        ),
        "{button}"
    );
    assert!(button.contains("pressReview(compound, button)"), "{button}");
}

/// A who's-here switch waits for a review as for a Sign (they share `noteSigning`), then closes
/// the chart, so the press settles with its OWN outcome, worded by the picker in a review's own
/// words: Sign off's "every part of the note now carries your signature" would be false for a
/// review, which countersigns only the parts the presser's supervisees signed. No line names a
/// patient (the next person may read it).
#[test]
fn a_switch_during_a_review_says_what_the_review_did() {
    use crate::source_guard::js_const;
    const WHOS_HERE_JS: &str = include_str!("../src-ui/whos_here.js");
    let press = js_fn(COMPOUND_JS, "pressReview");
    assert!(
        press.contains(
            "noteSigning = reviewNote(compound)\n    .catch((failure) => {\n      console.error(failure);\n      return \"review-unknown\";"
        ),
        "{press}"
    );
    assert!(!press.contains("=> null"), "{press}");
    let review = js_fn(COMPOUND_JS, "reviewNote");
    let caught = position(review, "} catch (failure) {");
    assert!(
        review[..caught].contains("outcome = behind ? \"review-unknown\" : \"reviewed\";"),
        "{review}"
    );
    assert!(
        review[caught..]
            .contains("outcome = behind === \"unknown\" ? \"review-unknown\" : \"not-reviewed\";"),
        "{review}"
    );
    assert!(
        position(review, "await refresh(said, behind);") < position(review, "return outcome;"),
        "{review}"
    );
    assert_eq!(
        js_const(WHOS_HERE_JS, "REVIEWED_LINE"),
        "Your co-signature went through: the parts your supervisees signed now carry it."
    );
    assert_eq!(
        js_const(WHOS_HERE_JS, "NOT_REVIEWED_LINE"),
        "Your co-signature signed nothing \u{2014} open the chart to see where the note stands."
    );
    assert_eq!(
        js_const(WHOS_HERE_JS, "REVIEW_UNKNOWN_LINE"),
        "Whether your co-signature went through is not known \u{2014} check the chart."
    );
    let line = js_fn(WHOS_HERE_JS, "signOutcomeLine");
    for arm in [
        "if (outcome === \"reviewed\") return REVIEWED_LINE;",
        "if (outcome === \"not-reviewed\") return NOT_REVIEWED_LINE;",
        "if (outcome === \"review-unknown\") return REVIEW_UNKNOWN_LINE;",
    ] {
        assert!(line.contains(arm), "{line}");
    }
}
