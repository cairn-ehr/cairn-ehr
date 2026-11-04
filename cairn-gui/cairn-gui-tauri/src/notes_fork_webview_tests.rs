//! Sign-in slice 3b-1: what the note editor keeps when a save finds its draft gone on without
//! this window — moved to another terminal (#958), or signed in another window (final review I1,
//! #945) — pinned against the source of `notes_save.js` and `notes.js`, as
//! `whos_here_webview_tests.rs` does (no npm or bundler: main.js's header, issue #332). The
//! behaviour was walked headlessly (a stub `invoke` in a browser) in the final fix wave.
use crate::source_guard::{js_fn, position};

/// The note pane's two scripts as one text: notes.js, and its save queue notes_save.js.
const NOTES_JS: &str = concat!(
    include_str!("../src-ui/notes.js"),
    include_str!("../src-ui/notes_save.js")
);

/// The one line that decides a save's answer forks: moved; committed by the quarantine timer;
/// or signed or discarded, unless this window's own press (Sign or Discard) may have made it.
const FORKS: &str = "if (view.moved || view.committed || ((view.spent || view.discarded) && !sent.held.mayBeSigned)) return keepUnsavedText(sent, view.fork);";

/// Review Focus 2: the old terminal saves after another took the draft. The save answers
/// `moved`, and what the node lacks is kept as a NEW draft (a fresh id, the same writer), saved
/// in queue order. Fix round 1, I3: only what was typed AFTER the last answered save is new — the
/// rest went on with the moved note — so three branches, each saying its own true sentence:
/// the remainder is kept; nothing is new (the editor closes); the text above was edited (all of
/// it is kept, and the sentence says the saved part is also in the moved note).
#[test]
fn moved_forks_a_new_draft() {
    let send = js_fn(NOTES_JS, "sendDraft");
    let forks = FORKS;
    assert!(
        position(send, forks) < position(send, "noteUnsaved = !kept;"),
        "{send}"
    );
    // Only an answered save says what the node holds.
    assert!(send[..position(send, "catch (failure) {")]
        .contains("if (view.saved) sent.held.savedText = sent.narrative;"));
    let split = js_fn(NOTES_JS, "unsavedRemainder");
    for line in [
        "if (!text.startsWith(savedText)) return { kept: text, said: forkWords.everything };",
        "const rest = text.slice(savedText.length);",
        "if (rest.trim() === \"\") return { kept: null, said: forkWords.nothing_new };",
        "return { kept: rest, said: forkWords.kept };",
    ] {
        assert!(split.contains(line), "{split}");
    }
    let keep = js_fn(NOTES_JS, "keepUnsavedText");
    assert!(keep.contains("const text = onScreen ? el(\"note-text\").value : sent.narrative;"));
    assert!(keep.contains(
        "const { kept, said } = unsavedRemainder(text, sent.held.savedText, forkWords);"
    ));
    let nothing_new = position(keep, "if (kept === null) {");
    assert!(nothing_new < position(keep, "closeEditor();"), "{keep}");
    for line in [
        "{ ...sent.held, draftId: crypto.randomUUID(), savedText: \"\", mayBeSigned: false, keptOnScreen: false, forkedTo: null };",
        "el(\"note-text\").value = kept;",
        "return sendDraft({ held: fork, narrative: kept, statedTime: sent.statedTime });",
        "noteStatus(said);",
    ] {
        assert!(keep.contains(line), "{keep}");
    }
    // An older save of the same draft, still queued once the editor's newer text was kept,
    // keeps nothing again: no second copy.
    assert!(
        position(keep, "if (sent.held.keptOnScreen) return true;")
            < position(keep, "unsavedRemainder("),
        "{keep}"
    );
    assert!(
        !NOTES_JS.contains("MOVED_NOTHING_KEPT"),
        "every moved sentence is Rust's"
    );
}

/// Final review I1 (#945): a save answering `spent` while this window's own Sign cannot have
/// signed it — the draft was continued and signed in another window — keeps the text typed here
/// since the last save as a new draft, exactly as a move does. A Sign marks its draft
/// `mayBeSigned` BEFORE it is sent, so the save an unknown-outcome Sign sends next never forks
/// text that Sign carried; a Sign refused outright (it recorded nothing) takes the mark back, so
/// the save its refusal sends forks what another window's spend lacks.
#[test]
fn a_draft_signed_in_another_window_forks_a_new_draft() {
    let sign = js_fn(NOTES_JS, "signNote");
    let marked = position(sign, "held.mayBeSigned = true;");
    assert!(
        position(sign, "const wasMaybeSigned = Boolean(held.mayBeSigned);") < marked,
        "{sign}"
    );
    assert!(marked < position(sign, "invoke(\"note_sign\""), "{sign}");
    let caught = &sign[position(sign, "catch (failure) {")..];
    assert!(
        position(
            caught,
            "if (behindTheRecord(failure) !== \"unknown\") {\n      held.mayBeSigned = wasMaybeSigned;"
        ) < position(caught, "void saveNoteNow();"),
        "{sign}"
    );
    assert!(
        !NOTES_JS.contains("give it a way out (#945)"),
        "the stale-refusal shortcut is gone: the save it sends now forks"
    );
}

/// PR #966 review: a refusal that wrote nothing (elsewhere, no chart, no such writer) is NOT
/// "saved". Only a save the node took — or a spend this window's own Sign may have made, whose
/// text went with that Sign — clears the editor's unsaved mark and resolves true; anything else
/// leaves the text marked as only on screen, so nothing downstream (a Sign's unknown-outcome
/// branch, the close's flush) treats it as kept.
#[test]
fn a_refusal_that_wrote_nothing_leaves_the_editor_unsaved() {
    let send = js_fn(NOTES_JS, "sendDraft");
    let answered = &send[..position(send, "catch (failure) {")];
    for line in [
        "const kept = view.saved || view.spent || view.discarded;",
        "if (kept) sent.held.statedTime = asked;",
        "noteUnsaved = !kept;",
        "return kept;",
    ] {
        assert!(answered.contains(line), "{send}");
    }
    assert!(
        !answered.contains("noteUnsaved = false;"),
        "an answer is not a save: {send}"
    );
}

/// PR #966 review: two queued saves of a moved (or signed-elsewhere) draft both answering after
/// the editor closed must not make two new drafts holding the same words. The first answer's new
/// draft is remembered on the old one (`forkedTo`); a later, newer answer refills it.
#[test]
fn later_answers_refill_the_one_fork_never_a_second() {
    let keep = js_fn(NOTES_JS, "keepUnsavedText");
    let reuse = position(keep, "const fork = sent.held.forkedTo ||");
    assert!(reuse < position(keep, "crypto.randomUUID()"), "{keep}");
    assert!(
        position(keep, "sent.held.forkedTo = fork;")
            < position(keep, "return sendDraft({ held: fork,"),
        "{keep}"
    );
}

/// Slice 3b-2a, Review Focus 1: a save answering `committed` (the quarantine timer put the draft
/// into the record unsigned) keeps the unsaved text as a new draft EVEN while this window's Sign
/// has marked the draft `mayBeSigned`: a timer's commit is never this window's Sign, so that text
/// went with no note. Only `spent` is excused by the mark. The sentence it says is the save's
/// `fork` words (Rust's, `notes_words.rs`), on the status line the editor shows (`noteStatus`);
/// its visibility was walked headlessly in a browser with a stub `invoke` (see the slice's PR).
#[test]
fn a_timer_commit_forks_even_after_a_sign_press() {
    let send = js_fn(NOTES_JS, "sendDraft");
    let line = &send[position(send, FORKS)..][..FORKS.len()];
    let excused = position(
        line,
        "((view.spent || view.discarded) && !sent.held.mayBeSigned)",
    );
    assert!(
        position(line, "view.committed ||") < excused,
        "committed is never excused by mayBeSigned: {line}"
    );
    let keep = js_fn(NOTES_JS, "keepUnsavedText");
    assert!(keep.contains("noteStatus(said);"), "{keep}");
}

/// PR #984 review (critical): after a press (Sign note or Discard draft) whose outcome was not
/// known, the editor stays live; text typed then is not in what the press carried. A later save of
/// it answering `spent` or `discarded` must keep that text as a new draft, never treat it as gone
/// with the press. So each press remembers the text it carries (`pressedText`, given back with the
/// mark on a refusal that recorded nothing), each save read after a press carries that text
/// (`pressCarried`), and a save whose text differs from it forks with the press's own words.
#[test]
fn text_typed_after_an_unknown_press_is_kept_never_lost() {
    for (script, name, verb) in [
        (NOTES_JS, "signNote", "note_sign"),
        (
            include_str!("../src-ui/notes_discard.js"),
            "discardNote",
            "note_discard",
        ),
    ] {
        let press = js_fn(script, name);
        let carried = position(press, "held.pressedText = el(\"note-text\").value;");
        assert!(
            position(press, "held.mayBeSigned = true;") < carried,
            "{press}"
        );
        assert!(
            carried < position(press, &format!("invoke(\"{verb}\"")),
            "{press}"
        );
        assert!(
            position(press, "const wasPressedText = held.pressedText;")
                < position(press, "held.mayBeSigned = true;"),
            "{press}"
        );
        let caught = &press[position(press, "catch (failure) {")..];
        assert!(
            caught.contains("held.pressedText = wasPressedText;"),
            "{press}"
        );
    }
    let save = js_fn(NOTES_JS, "saveNoteNow");
    assert!(
        save.contains("pressCarried: held.mayBeSigned ? held.pressedText : null,"),
        "{save}"
    );
    let send = js_fn(NOTES_JS, "sendDraft");
    let after = position(
        send,
        "if (pressTook && typeof sent.pressCarried === \"string\" && sent.narrative !== sent.pressCarried) {",
    );
    assert!(
        send.contains("const pressTook = (view.spent || view.discarded) && sent.held.mayBeSigned;"),
        "{send}"
    );
    // After the forks of a draft gone on without this window, before anything counts as kept.
    assert!(position(send, FORKS) < after, "{send}");
    assert!(after < position(send, "const kept ="), "{send}");
    let branch = &send[after..position(send, "const kept =")];
    for line in [
        "sent.held.savedText = sent.pressCarried;",
        "return keepUnsavedText(sent, view.pressed_fork);",
    ] {
        assert!(branch.contains(line), "{send}");
    }
}
