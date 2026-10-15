//! #782 — the verdict buttons' "(unlock your signing key first)" hint must never be lost to
//! script load order.
//!
//! # The race
//!
//! `main.js` asks the backend for the signing key's lock state (`pollLock` → `renderLockState`)
//! and hands the answer to `updateLinkLock`, which `link.js` defines. The scripts are classic
//! `<script>` tags, `main.js` first and `link.js` fifth. The first poll used to start while
//! `main.js` itself was loading. If its answer landed before `link.js` had run, `renderLockState`
//! found no `updateLinkLock` and skipped it, and `link.js` then started from its own default
//! (`keyUnlocked = false`). The verdict buttons showed the wrong lock state until the next poll,
//! up to `LOCK_POLL_MS` later. A headless walk with an instant `lock_state` reproduced it
//! (R6's Task 8).
//!
//! # The fix these tests pin
//!
//! The first poll starts on `DOMContentLoaded`. The browser fires that event only after every
//! classic, synchronous script in the page has run, so `updateLinkLock` exists by then. That
//! guarantee holds only while no script tag is `defer`, `async` or a module, so the second test
//! pins that too.
//!
//! `main.js` and `index.html` are untyped, so these tests read their source, like the
//! field-name drift guards beside them.

const MAIN_JS: &str = include_str!("../../src-ui/main.js");
const INDEX_HTML: &str = include_str!("../../src-ui/index.html");

#[test]
fn the_first_lock_poll_waits_for_every_script_to_load() {
    assert!(
        !MAIN_JS
            .lines()
            .any(|line| line.trim() == "void pollLock();"),
        "main.js must not start the first lock poll while it is itself loading: link.js, which \
         defines updateLinkLock, has not run yet"
    );
    let at = MAIN_JS
        .find("document.addEventListener(\"DOMContentLoaded\"")
        .expect("main.js starts the first lock poll on DOMContentLoaded");
    let listener = &MAIN_JS[at..at + MAIN_JS[at..].find(';').expect("the call ends")];
    assert!(
        listener.contains("pollLock()"),
        "the DOMContentLoaded listener runs the first lock poll: {listener}"
    );
}

#[test]
fn every_script_is_classic_and_synchronous_so_domcontentloaded_follows_them_all() {
    let tags: Vec<&str> = INDEX_HTML
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("<script"))
        .collect();
    assert!(
        tags.iter().any(|t| t.contains("src=\"link.js\"")),
        "index.html loads link.js: {tags:?}"
    );
    for tag in tags {
        for word in ["defer", "async", "type=\"module\""] {
            assert!(
                !tag.contains(word),
                "{tag} is `{word}`: DOMContentLoaded would no longer wait for it, and the first \
                 lock poll could again land before updateLinkLock exists (#782)"
            );
        }
    }
}
