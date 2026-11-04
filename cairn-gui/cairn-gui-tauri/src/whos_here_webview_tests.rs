//! Sign-in slice 3b-1, Task 6: the webview's "who's here" (`whos_here.js`), the idle re-lock
//! (`idle.js`) and a draft drawn as its writers' parts (`notes.js`).
//!
//! The window's scripts have no test rig (no npm or bundler: main.js's header, issue #332), so,
//! like `notes_webview_tests.rs`, each guard reads a script's source (`crate::source_guard`) and
//! pins the ORDER or the CHOICE a property is. The behaviour was walked headlessly (a stub
//! `invoke` in a browser) when it was written — see the PR that added this file.
use crate::source_guard::{js_fn, position};

const WHOS_HERE_JS: &str = include_str!("../src-ui/whos_here.js");
const IDLE_JS: &str = include_str!("../src-ui/idle.js");
/// The note pane's two scripts as one text: notes.js, and its save queue notes_save.js.
const NOTES_JS: &str = concat!(
    include_str!("../src-ui/notes.js"),
    include_str!("../src-ui/notes_save.js")
);
const SIGNER_JS: &str = include_str!("../src-ui/signer.js");
const MAIN_JS: &str = include_str!("../src-ui/main.js");
const FUNNEL_JS: &str = include_str!("../src-ui/funnel.js");
const INDEX_HTML: &str = include_str!("../src-ui/index.html");

/// Review Focus 1: Nurse A has typed and not yet saved when Dr B presses the hot key. The chart
/// leaves the screen through `closeChart`, which flushes A's text (under A: the save carries the
/// writer the editor captured) and AWAITS every queued save, and only then may a name be picked.
/// The hot key and the button are the same switch.
#[test]
fn switch_awaits_the_save_queue_before_picking() {
    let switch = js_fn(WHOS_HERE_JS, "switchWhosHere");
    assert!(
        position(switch, "await closeChart();") < position(switch, "openPicker("),
        "{switch}"
    );
    // Fix round 1, I4: a Sign in flight settles first, so its outcome is said on the chart it
    // signed, and the pick's key-forgetting never lands mid-sign.
    assert!(
        position(switch, "await signsSettled();") < position(switch, "await closeChart();"),
        "{switch}"
    );
    let close = js_fn(FUNNEL_JS, "closeChart");
    assert!(
        position(close, "await notesSaved();") < position(close, "invoke(\"close_chart\")"),
        "{close}"
    );
    // Fix round 1, I2: Enter on a BLANK search names nobody — the list's head is the person who
    // just left, and a reflexive Enter would claim their name.
    let enter = &WHOS_HERE_JS[position(
        WHOS_HERE_JS,
        "el(\"whos-here-form\").addEventListener(\"submit\"",
    )..];
    assert!(
        position(
            enter,
            "if (el(\"whos-here-search\").value.trim() === \"\") return;"
        ) < position(enter, "matchingEntries()[0]"),
        "{enter}"
    );
    // A1: leaving the picker puts focus back on the page, never on a hidden element.
    let close = js_fn(WHOS_HERE_JS, "closePicker");
    assert!(
        position(close, "document.querySelector(\"main\").inert =") < position(close, ".focus();"),
        "{close}"
    );
    assert!(
        close.contains(
            "el(displayedPatient === null ? \"browse-name\" : \"whos-here-switch\").focus();"
        ),
        "{close}"
    );
    // The only pick is a press in the picker, which opens only after the awaited close.
    assert_eq!(WHOS_HERE_JS.matches("invoke(\"whos_here_pick\"").count(), 1);
    assert!(js_fn(WHOS_HERE_JS, "pick").contains("invoke(\"whos_here_pick\", { staffId })"));
    let key = js_fn(WHOS_HERE_JS, "onHotKey");
    assert!(
        key.contains("(event.ctrlKey || event.metaKey) && event.shiftKey"),
        "{key}"
    );
    assert!(key.contains("void switchWhosHere();"), "{key}");
    assert!(WHOS_HERE_JS.contains(
        "el(\"whos-here-switch\").addEventListener(\"click\", () => void switchWhosHere());"
    ));
}

/// The writer of a save is captured when the editor OPENS and travels with every save and the
/// sign (global constraints). Never re-read from the current claim at save time: a debounced save
/// landing after a switch must still name the person who typed it. Missing, every window-signed
/// note would record its signer as `attested` instead of `authored`, in an immutable record.
#[test]
fn the_writer_is_set_once_at_editor_open() {
    assert!(js_fn(NOTES_JS, "newNote").contains("writer: whosHere().staff_id,"));
    // A reopened draft writes as the claim the pane chose `editable` for (final review item 7):
    // the part it opens on and the writer it saves as can never diverge.
    assert!(js_fn(NOTES_JS, "openDraft").contains("writer: writer,"));
    assert_eq!(
        NOTES_JS.matches("writer: whosHere().staff_id,").count(),
        1,
        "the writer is captured only where an editor opens"
    );
    assert!(
        js_fn(NOTES_JS, "sendDraft").contains("writer: sent.held.writer,"),
        "every save sends the captured writer"
    );
    assert!(
        js_fn(NOTES_JS, "signNote").contains("writer: held.writer,"),
        "the sign sends the captured writer"
    );
    for at_save in [
        "saveNoteNow",
        "sendDraft",
        "signNote",
        "keepUnsavedText",
        "openDraft",
    ] {
        let body = js_fn(NOTES_JS, at_save);
        assert!(
            !body.contains("whosHere("),
            "{at_save} re-reads the claim: {body}"
        );
    }
    // Nobody asked yet: New note asks first, and opens the editor once a claim is made.
    let new_note = js_fn(NOTES_JS, "newNote");
    assert!(
        position(new_note, "askWhosHere(newNote);") < position(new_note, "crypto.randomUUID()"),
        "{new_note}"
    );
    // ...and a here-draft is reopened for writing only once somebody has said who is writing,
    // and only by a pane read for the claim held now, as that claim's writer.
    let render = js_fn(NOTES_JS, "renderNotes");
    assert!(render.contains("const claim = pane.claim;"), "{render}");
    assert!(
        render.contains(
            "if (draft.here && noteDraft === null && readForThisClaim(claim)) openDraft(draft, claim.staff_id);"
        ),
        "{render}"
    );
    assert!(js_fn(NOTES_JS, "readForThisClaim").contains(
        "return claimAsked() && whosHere().kind === claim.kind && whosHere().staff_id === claim.staff_id;"
    ));
}

/// Review Focus 4 / main.js's poll lesson: the idle clock is reset by a person at the keyboard
/// and nothing else — no poll, interval or IPC answer. Listened for in the capture phase, so a
/// keystroke in ANY field (the passphrase field included) counts.
#[test]
fn idle_resets_only_on_user_input() {
    assert!(IDLE_JS.contains(
        "for (const kind of [\"keydown\", \"pointerdown\", \"input\", \"wheel\"]) {\n  \
         document.addEventListener(kind, noteActivity, { capture: true, passive: true });"
    ));
    // `noteActivity` is named only where it is declared and where it is listened for.
    for (file, js) in [
        ("idle.js", IDLE_JS),
        ("main.js", MAIN_JS),
        ("signer.js", SIGNER_JS),
        ("notes.js + notes_save.js", NOTES_JS),
        ("whos_here.js", WHOS_HERE_JS),
    ] {
        let expected = if file == "idle.js" { 2 } else { 0 };
        assert_eq!(js.matches("noteActivity").count(), expected, "{file}");
    }
    assert!(
        !IDLE_JS.contains("setInterval"),
        "the idle clock never polls"
    );
    assert_eq!(
        IDLE_JS.matches("setTimeout(").count(),
        1,
        "only noteActivity arms it"
    );
    assert!(js_fn(IDLE_JS, "noteActivity").contains("setTimeout(idleRelock, idleLockMs)"));
    // The poll hands over the interval; it never arms or pushes back the timer.
    let set = js_fn(IDLE_JS, "setIdleLock");
    assert!(
        !set.contains("setTimeout") && !set.contains("clearTimeout"),
        "{set}"
    );
    assert!(js_fn(MAIN_JS, "renderLockState").contains("setIdleLock(lock.idle_lock_secs);"));
}

/// Review Focus 4: the re-lock fires while a Sign is in flight. It waits for the Sign to settle
/// (a key pressed during the wait is activity, and keeps the terminal open), then closes the
/// chart (its saves awaited), forgets the key and the claim, and shows the terminal lock again.
#[test]
fn relock_waits_for_a_sign_in_flight() {
    let relock = js_fn(IDLE_JS, "idleRelock");
    let settled = position(relock, "await signsSettled();");
    assert!(
        settled < position(relock, "if (idleTimer !== null) return;"),
        "{relock}"
    );
    assert!(
        settled < position(relock, "await closeChart();"),
        "{relock}"
    );
    assert!(
        position(relock, "await closeChart();") < position(relock, "invoke(\"relock\")"),
        "{relock}"
    );
    assert!(
        position(relock, "invoke(\"relock\")") < position(relock, "relockTerminal("),
        "{relock}"
    );
    // The Sign in flight is a promise the re-lock can wait on, and it never rejects.
    assert!(NOTES_JS.contains(
        "el(\"note-sign\").addEventListener(\"click\", () => {\n  noteSignInFlight = true;"
    ));
    assert!(NOTES_JS.contains("    .finally(() => {\n      noteSignInFlight = false;"));
    assert!(js_fn(NOTES_JS, "signsSettled").contains("return noteSigning;"));
    // The lock shows again: the shared unlock is asked for anew.
    let shown = js_fn(SIGNER_JS, "relockTerminal");
    assert!(shown.contains("terminalUnlocked = false;"), "{shown}");
    assert!(shown.contains("renderSigner(lock);"), "{shown}");
}

/// Fix round 2 (I4's residual): Sign pressed, then the hot key at once. The switch waits for the
/// Sign, but `closeChart` then clears the note pane, so its outcome would never be read — a
/// refused or unknown sign could be taken for signed. The outcome of a Sign that was IN FLIGHT
/// when the switch was pressed is captured before `closeChart` and said in the picker, as a
/// status line worded with no patient identifier (the next person may read it). It goes when a
/// person is picked or the picker closes.
#[test]
fn a_sign_settled_during_a_switch_is_said_in_the_picker() {
    let switch = js_fn(WHOS_HERE_JS, "switchWhosHere");
    let in_flight = position(switch, "const signing = signInFlight();");
    let settled = position(switch, "const signed = await signsSettled();");
    assert!(in_flight < settled, "{switch}");
    assert!(
        settled < position(switch, "await closeChart();"),
        "{switch}"
    );
    assert!(
        switch.contains("openPicker(null, signing ? signed : null);"),
        "{switch}"
    );
    let open = js_fn(WHOS_HERE_JS, "openPicker");
    assert!(
        open.contains("setMessage(el(\"whos-here-signed\"), signOutcomeLine(signed));"),
        "{open}"
    );
    assert!(
        js_fn(WHOS_HERE_JS, "closePicker").contains("setMessage(el(\"whos-here-signed\"), \"\");")
    );
    assert!(
        position(
            INDEX_HTML,
            "<p id=\"whos-here-signed\" role=\"status\" hidden></p>"
        ) < position(INDEX_HTML, "<h2 id=\"whos-here-heading\">"),
        "the line sits above the picker's heading"
    );
    // The words: one per outcome, none naming a patient.
    use crate::source_guard::js_const;
    assert_eq!(
        js_const(WHOS_HERE_JS, "SIGNED_LINE"),
        "Your note was signed."
    );
    assert_eq!(
        js_const(WHOS_HERE_JS, "NOT_SIGNED_LINE"),
        "Your note was NOT signed. Open the chart and sign it again."
    );
    // Fix round 3: the refusal's reason lived only in the note pane the switch cleared, so no
    // line may send the clinician to the chart to read why.
    assert!(!WHOS_HERE_JS.contains("to see why"));
    assert_eq!(
        js_const(WHOS_HERE_JS, "SIGN_UNKNOWN_LINE"),
        "Whether your note was signed is not known \u{2014} check the chart."
    );
    let line = js_fn(WHOS_HERE_JS, "signOutcomeLine");
    for arm in [
        "if (outcome === \"signed\") return SIGNED_LINE;",
        "if (outcome === \"not-signed\") return NOT_SIGNED_LINE;",
        "if (outcome === \"unknown\") return SIGN_UNKNOWN_LINE;",
        "return \"\";",
    ] {
        assert!(line.contains(arm), "{line}");
    }
    // signNote answers its outcome on every path that pressed Sign: refused (or a time only
    // partly typed) is "not-signed"; an interrupted commit (#814) is "unknown".
    let sign = js_fn(NOTES_JS, "signNote");
    let caught = &sign[position(sign, "catch (failure) {")..];
    assert!(
        caught.contains(
            "return behindTheRecord(failure) === \"unknown\" ? \"unknown\" : \"not-signed\";"
        ),
        "{sign}"
    );
    assert!(
        sign[..position(sign, "clearTimeout(noteSaveTimer);")].contains("return \"not-signed\";"),
        "a partly typed time signs nothing: {sign}"
    );
    assert!(sign.contains("return report.behind_the_record ? \"unknown\" : \"signed\";"));
    assert!(NOTES_JS.contains(
        "noteSignInFlight = true;\n  noteSigning = signNote()\n    .catch((failure) => {"
    ));
    assert!(js_fn(NOTES_JS, "signInFlight").contains("return noteSignInFlight;"));
}

/// A draft's parts are drawn as read-only blocks through `cell` (3a's rule: never `innerHTML` a
/// note), each headed by its claimed writer, heard as "unverified …". The editor holds only the
/// editable tail, or nothing for a new part (M3: an unknown claim on an unknown tail edits it,
/// because Rust marks that tail `editable`).
#[test]
fn parts_drawn_through_cell() {
    let block = js_fn(NOTES_JS, "partBlock");
    assert!(
        block.contains("cell(\"h3\", part.writer_shown, { \"aria-label\": part.writer_spoken })"),
        "{block}"
    );
    assert!(
        block.contains("cell(\"p\", part.text, { class: \"note-text\" })"),
        "{block}"
    );
    let open = js_fn(NOTES_JS, "openDraft");
    for line in [
        "const editing = Boolean(tail && tail.editable);",
        "el(\"note-parts\").replaceChildren(...above.map(partBlock));",
        "el(\"note-text\").value = editing ? tail.text : \"\";",
    ] {
        assert!(open.contains(line), "{open}");
    }
    assert!(js_fn(NOTES_JS, "draftItem").contains("draft.parts.map(partBlock)"));
    // A signed part's byline is HEARD in its spoken form, "unverified Nurse A" (#973): the shown
    // words, with their bare `?`, are hidden from a screen reader, the spoken ones from the eye
    // (a paragraph's aria-label is not read out). Both are drawn through `cell`.
    let item = js_fn(NOTES_JS, "noteItem");
    for line in [
        "cell(\"span\", note.byline, { \"aria-hidden\": \"true\" }),",
        "cell(\"span\", note.byline_spoken, { class: \"visually-hidden\" }),",
    ] {
        assert!(item.contains(line), "{item}");
    }
    assert!(js_fn(NOTES_JS, "closeEditor").contains("el(\"note-parts\").replaceChildren();"));
    for (file, js) in [
        ("notes.js + notes_save.js", NOTES_JS),
        ("whos_here.js", WHOS_HERE_JS),
        ("idle.js", IDLE_JS),
    ] {
        for markup in [
            "innerHTML",
            "outerHTML",
            "insertAdjacentHTML",
            "document.write",
        ] {
            assert!(!js.contains(markup), "{file} uses {markup}");
        }
        for store in ["localStorage", "sessionStorage", "indexedDB"] {
            assert!(!js.contains(store), "{file} uses {store}");
        }
    }
    // A draft held elsewhere offers "Continue here", which moves it, then re-reads the pane.
    let go_on = js_fn(NOTES_JS, "continueHere");
    assert!(
        position(go_on, "invoke(\"note_draft_take\"")
            < position(go_on, "await readNotes(patient);"),
        "{go_on}"
    );
}

/// On every pick the signer strip's staff ID becomes the claim's (empty for "Don't know" and on
/// the re-lock), and after an act empties it on a shared terminal it is filled again from the
/// claim (G2, design §3.6). The passphrase is never SET by who's here, and is always CLEARED when
/// the picker opens and on the re-lock (fix round 1, I1): Nurse A's typed-but-unused secret must
/// not sit beside Dr B's ID.
#[test]
fn staff_id_prefilled_passphrase_never() {
    let open = js_fn(WHOS_HERE_JS, "openPicker");
    assert!(
        position(open, "el(\"signer-passphrase\").value = \"\";")
            < position(open, "el(\"whos-here-picker\").hidden = false;"),
        "{open}"
    );
    assert!(js_fn(SIGNER_JS, "relockTerminal").contains("el(\"signer-passphrase\").value = \"\";"));
    let cleared = js_fn(SIGNER_JS, "clearSigner");
    assert!(
        position(cleared, "el(\"staff-id\").value = \"\";")
            < position(cleared, "el(\"staff-id\").value = claimedStaffId();"),
        "{cleared}"
    );
    assert!(js_fn(WHOS_HERE_JS, "claimedStaffId")
        .contains("return (currentClaim && currentClaim.staff_id) || \"\";"));
    let take = js_fn(WHOS_HERE_JS, "takeClaim");
    assert!(
        take.contains("el(\"staff-id\").value = claim.staff_id || \"\";"),
        "{take}"
    );
    assert!(js_fn(WHOS_HERE_JS, "pick").contains("takeClaim(picked.claim);"));
    assert!(js_fn(SIGNER_JS, "relockTerminal").contains("el(\"staff-id\").value = \"\";"));
    for (file, js, clears) in [("whos_here.js", WHOS_HERE_JS, 1), ("idle.js", IDLE_JS, 0)] {
        assert_eq!(
            js.matches("signer-passphrase").count(),
            clears,
            "{file} touches the passphrase other than to clear it"
        );
        assert!(!js.contains("credential"), "{file} handles a credential");
    }
    // The picker asks for a name, never a passphrase.
    let picker = &INDEX_HTML[position(INDEX_HTML, "<section id=\"whos-here-picker\"")..];
    let picker = &picker[..position(picker, "</section>")];
    assert!(!picker.contains("password"), "{picker}");
}

/// The bar: full-width, the name large, a polite live region that announces a change (and is
/// written only when the words change, so the 10-second poll never re-announces it). The picker
/// is a region, never a dialog, with "Don't know / just reading". Nothing asks to confirm.
#[test]
fn the_bar_is_aria_live() {
    for needle in [
        "<p id=\"whos-here-name\" role=\"status\" aria-live=\"polite\">",
        "<button id=\"whos-here-switch\" type=\"button\" aria-keyshortcuts=\"Control+Shift+H Meta+Shift+H\">Who's here?</button>",
        "<section id=\"whos-here-picker\" aria-labelledby=\"whos-here-heading\" hidden>",
        "<label for=\"whos-here-search\">",
        "<button id=\"whos-here-unknown\" type=\"button\">Don't know / just reading</button>",
        "<button id=\"whos-here-return\" type=\"button\">",
    ] {
        assert!(INDEX_HTML.contains(needle), "index.html lacks {needle}");
    }
    let show = js_fn(WHOS_HERE_JS, "showClaim");
    assert!(
        show.contains("if (bar.textContent !== claim.words) bar.textContent = claim.words;"),
        "{show}"
    );
    // The poll only seeds the claim at launch; a later poll can never undo a pick.
    assert!(js_fn(WHOS_HERE_JS, "seedWhosHere")
        .contains("if (currentClaim === null) showClaim(claim);"));
    assert!(js_fn(MAIN_JS, "renderLockState").contains("seedWhosHere(lock.whos_here);"));
    for js in [WHOS_HERE_JS, IDLE_JS] {
        for dialog in ["confirm(", "alert(", "prompt(", "<dialog"] {
            assert!(!js.contains(dialog), "uses {dialog}");
        }
    }
    assert!(!INDEX_HTML.contains("<dialog"));
    // Loaded after notes.js, before funnel.js's boot (which may open a chart at once).
    let at = |s: &str| position(INDEX_HTML, &format!("<script src=\"{s}\"></script>"));
    assert!(at("notes.js") < at("whos_here.js"));
    assert!(at("whos_here.js") < at("idle.js"));
    assert!(at("idle.js") < at("funnel.js"));
}

/// The drift guard every webview file carries: every field whos_here.js reads off a payload is one
/// the Rust type sends. A renamed `staff_id` would read `undefined` — and a note would be written
/// with no writer at all.
#[test]
fn whos_here_js_reads_no_field_the_backend_does_not_send() {
    use crate::commands::tests::fields_read_in;
    use crate::funnel::view::ChartHeaderView;
    use crate::whos_here::{claim_view, Claim, PickView, StaffEntryView};
    let keys = |v: serde_json::Value| -> std::collections::BTreeSet<String> {
        v.as_object().expect("an object").keys().cloned().collect()
    };
    let header = || ChartHeaderView {
        patient_id: "p".into(),
        name: "n".into(),
        born: "b".into(),
        trust: "t".into(),
    };
    let named = Claim::Named {
        staff_id: "s".into(),
        display_name: "d".into(),
    };
    let picked = PickView {
        claim: claim_view(&named),
        offer: Some(header()),
    };
    let entry = StaffEntryView {
        staff_id: "s".into(),
        display_name: "d".into(),
        role: "r".into(),
        recent: true,
    };
    for (binding, available) in [
        ("picked", keys(serde_json::to_value(&picked).unwrap())),
        (
            "claim",
            keys(serde_json::to_value(claim_view(&named)).unwrap()),
        ),
        ("entry", keys(serde_json::to_value(&entry).unwrap())),
        ("offer", keys(serde_json::to_value(header()).unwrap())),
    ] {
        let read = fields_read_in(WHOS_HERE_JS, binding);
        assert!(!read.is_empty(), "whos_here.js no longer reads `{binding}`");
        for field in read {
            assert!(
                available.contains(&field),
                "whos_here.js reads `{binding}.{field}`, not sent"
            );
        }
    }
}
