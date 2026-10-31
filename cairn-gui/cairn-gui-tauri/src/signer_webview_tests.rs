//! Sign-in slice 1, Task 6: the webview's signer strip and terminal lock.
//!
//! The window's scripts have no test rig (no npm, by design), so these read their source, as the
//! other `*_webview_tests` do. What they pin: every clinical write hands the strip's credential
//! to its command and empties the passphrase on BOTH ways out (a wrong passphrase must not sit in
//! the field for the next person); the strip is labelled; no modal exists; and the old
//! "unlock your signing key first" hint is gone, because with a key hold of 0 no key is ever held
//! beforehand and the hint would be false.
use crate::source_guard::{js_fn, position};

const MAIN_JS: &str = include_str!("../src-ui/main.js");
const HOLDS_JS: &str = include_str!("../src-ui/holds.js");
const LINK_JS: &str = include_str!("../src-ui/link.js");
const UNLINK_JS: &str = include_str!("../src-ui/unlink.js");
const SIGNER_JS: &str = include_str!("../src-ui/signer.js");
const INDEX_HTML: &str = include_str!("../src-ui/index.html");
const STYLE_CSS: &str = include_str!("../src-ui/style.css");

/// The seven functions that send a signed act (the eighth write, `confirm_link`, is
/// `confirmCompared`; `link_records`/`record_different_people` share `sendJudgement`).
const WRITERS: [(&str, &str, &str); 7] = [
    ("main.js", "signOff", MAIN_JS),
    ("main.js", "cease", MAIN_JS),
    ("holds.js", "withhold", HOLDS_JS),
    ("holds.js", "release", HOLDS_JS),
    ("link.js", "sendJudgement", LINK_JS),
    ("unlink.js", "unlinkCompared", UNLINK_JS),
    ("unlink.js", "confirmCompared", UNLINK_JS),
];

#[test]
fn every_write_sends_the_credential_and_clears_the_passphrase_both_ways() {
    for (file, name, src) in WRITERS {
        let body = js_fn(src, name);
        assert!(
            body.contains("credential: signerCredential(),"),
            "{file} {name} must pass the strip's credential: {body}"
        );
        let invoke = position(body, "invoke(");
        let catch = position(body, "catch (failure) {");
        assert!(
            body[invoke..catch].contains("clearSigner(true);"),
            "{file} {name} must clear the strip (and remember the signer) when the act is signed: {body}"
        );
        assert!(
            body[catch + "catch (failure) {".len()..]
                .trim_start()
                .starts_with("clearSigner();"),
            "{file} {name} must clear the strip FIRST in its catch: {body}"
        );
    }
}

/// The ID is kept on any terminal with a key hold, not only while `heldBy` says a key is held:
/// `heldBy` is read before the post-act poll, so right after the FIRST signed act on a held
/// terminal it is still null and the ID would be cleared (final review item 1). Keeping it is
/// safe because an ID with no passphrase never reaches the vault (`signer::decide`).
#[test]
fn clear_signer_empties_the_passphrase_and_keeps_the_id_only_on_a_held_terminal() {
    let clear = js_fn(SIGNER_JS, "clearSigner");
    assert!(
        clear.contains("el(\"signer-passphrase\").value = \"\";"),
        "{clear}"
    );
    assert!(clear.contains("if (keyHoldSecs === 0) {"), "{clear}");
    assert!(!clear.contains("if (!heldBy)"), "{clear}");
    let render = js_fn(SIGNER_JS, "renderSigner");
    assert!(
        render.contains("keyHoldSecs = lock.key_hold_secs || 0;"),
        "{render}"
    );
    assert!(clear.contains("el(\"staff-id\").value = \"\";"), "{clear}");
    let credential = js_fn(SIGNER_JS, "signerCredential");
    assert!(
        credential.contains("return null;"),
        "an empty strip sends null: {credential}"
    );
}

#[test]
fn the_strip_is_labelled_and_the_window_stays_ambient() {
    for id in ["staff-id", "signer-passphrase", "terminal-passphrase"] {
        assert!(
            INDEX_HTML.contains(&format!("<label for=\"{id}\"")),
            "{id} needs a <label for>"
        );
    }
    for needle in [
        "<form id=\"signer\"",
        "list=\"recent-signers\"",
        "autocomplete=\"off\"",
        "id=\"signer-hint\"",
        "id=\"forget-key\"",
        "id=\"terminal-lock\"",
        "<p id=\"lock-state\" role=\"status\"",
        "<script src=\"signer.js\"></script>",
    ] {
        assert!(INDEX_HTML.contains(needle), "index.html lacks {needle}");
    }
    // Always visible: the strip sits outside both surfaces (the front door and the chart are
    // `hidden` until chosen), and a walk found it unreachable when it sat inside the chart.
    assert!(
        position(INDEX_HTML, "<section id=\"session\"")
            < position(INDEX_HTML, "<section id=\"front-door\""),
        "the signer strip precedes, and so is outside, both surfaces"
    );
    for (name, text) in [("index.html", INDEX_HTML), ("signer.js", SIGNER_JS)] {
        assert!(!text.contains("<dialog"), "{name}: no dialogs");
        assert!(!text.contains("confirm("), "{name}: no confirm()");
    }
    // VoiceOver reads an em dash as a pause: none in the sentences signer.js speaks.
    let spoken: Vec<&str> = SIGNER_JS
        .lines()
        .filter(|l| {
            !l.trim_start().starts_with("//")
                && !l.trim_start().starts_with('*')
                && !l.trim_start().starts_with("/**")
        })
        .filter(|l| l.contains('\u{2014}'))
        .collect();
    assert!(spoken.is_empty(), "{spoken:?}");
}

#[test]
fn the_terminal_lock_is_never_shown_in_fixture_mode_and_says_when_no_passphrase_is_set() {
    let render = js_fn(SIGNER_JS, "renderSigner");
    assert!(render.contains("if (lock.mock) {"), "{render}");
    assert!(
        SIGNER_JS.contains("No terminal passphrase is set. Run cairn-node vault-set-terminal-passphrase on the node."),
        "the not-set sentence"
    );
    assert!(SIGNER_JS.contains("\"not-set\""));
}

#[test]
fn the_old_unlock_hint_and_form_are_gone() {
    for (name, text) in [
        ("main.js", MAIN_JS),
        ("link.js", LINK_JS),
        ("unlink.js", UNLINK_JS),
        ("index.html", INDEX_HTML),
        ("style.css", STYLE_CSS),
    ] {
        for gone in [
            "unlock your signing key first",
            "updateLinkLock",
            "keyUnlocked",
            "unlock-form",
        ] {
            assert!(!text.contains(gone), "{name} still has `{gone}`");
        }
    }
}

/// Fix round 1 (M1-M4): the page starts locked, the lock takes focus, a write refreshes the lock
/// state at once, and only a signed act enters the recent-signers list.
#[test]
fn the_page_starts_locked_and_a_write_refreshes_the_lock_state() {
    assert!(
        INDEX_HTML.contains("<main inert>"),
        "main starts inert (M1)"
    );
    let lock = js_fn(SIGNER_JS, "setTerminalLocked");
    assert!(
        lock.contains("el(\"terminal-passphrase\").focus();"),
        "{lock}"
    );
    let clear = js_fn(SIGNER_JS, "clearSigner");
    assert!(clear.contains("void pollLock();"), "{clear}");
    assert!(
        clear.contains("if (signed) rememberSigner(el(\"staff-id\").value);"),
        "{clear}"
    );
    assert!(
        position(clear, "rememberSigner(") < position(clear, "el(\"staff-id\").value = \"\";"),
        "remembered before it is cleared"
    );
    let hint = js_fn(SIGNER_JS, "showSignerHint");
    assert!(
        !hint.contains("rememberSigner"),
        "a typo never enters the list: {hint}"
    );
    let verdicts = js_fn(SIGNER_JS, "tryTerminalUnlock");
    assert!(verdicts.contains("\"fault\""), "{verdicts}");
}

const ENROL_JS: &str = include_str!("../src-ui/enrol.js");

/// Task 7: the enrol panel is labelled, carries the shared-terminal warning, and empties BOTH
/// new-passphrase fields (and the strip, without remembering the superuser) on both ways out.
#[test]
fn the_enrol_panel_is_labelled_warns_about_the_hint_and_clears_both_ways() {
    assert!(INDEX_HTML.contains("<details id=\"enrol-staff\">"));
    assert!(INDEX_HTML.contains("<script src=\"enrol.js\"></script>"));
    for id in [
        "enrol-staff-id",
        "enrol-name",
        "enrol-role",
        "enrol-phone",
        "enrol-email",
        "enrol-passphrase",
        "enrol-passphrase-again",
        "enrol-hint",
    ] {
        assert!(
            INDEX_HTML.contains(&format!("<label for=\"{id}\"")),
            "{id} needs a <label for>"
        );
    }
    for role in ["clinician", "nurse", "superuser"] {
        assert!(INDEX_HTML.contains(&format!("<option value=\"{role}\">")));
    }
    assert!(
        INDEX_HTML.contains("Colleagues at a shared terminal can read this hint."),
        "the hint warning"
    );
    assert!(!ENROL_JS.contains("confirm(") && !INDEX_HTML.contains("<dialog"));
    let body = js_fn(ENROL_JS, "enrolStaff");
    assert!(body.contains("credential: signerCredential()"), "{body}");
    let catch = position(body, "catch (failure) {");
    let after = &body[catch..];
    // Cleared AFTER the try/catch, so one code path covers success and refusal.
    for clear in [
        "el(\"enrol-passphrase\").value = \"\";",
        "el(\"enrol-passphrase-again\").value = \"\";",
        "clearSigner(false);",
    ] {
        assert!(
            after.contains(clear),
            "{clear} missing after the catch: {body}"
        );
    }
    assert!(
        !body.contains("clearSigner(true)"),
        "an enrol is not a signed act"
    );
}
