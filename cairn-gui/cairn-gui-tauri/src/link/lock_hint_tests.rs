//! #782 — the lock state must never be lost to script load order. (The verdict buttons' "(unlock
//! your signing key first)" hint this once protected is gone: with a key hold of 0 no key is
//! held beforehand, and `signer_webview_tests` pins its absence. What stays is the same race for
//! the signer strip and the terminal lock.)
//!
//! # The race
//!
//! `main.js` asks the backend for the signing key's lock state (`pollLock` → `renderLockState`)
//! and hands the answer to `renderSigner`, which `signer.js` defines. The scripts are classic
//! `<script>` tags, `main.js` first and `signer.js` second. The first poll used to start while
//! `main.js` itself was loading. If its answer landed before `signer.js` had run, `renderLockState`
//! found no `renderSigner` (a ReferenceError, caught as "could not check the signing key"), and the
//! strip and terminal lock showed their defaults until the next poll, up to `LOCK_POLL_MS` later. A headless walk with an instant `lock_state` reproduced it
//! (R6's Task 8), and showed the fix (PR #792).
//!
//! # The fix these tests pin
//!
//! The first poll starts on `DOMContentLoaded`. The browser fires that event only after every
//! parser-inserted classic script has run, so `renderSigner` exists by then. The second test
//! keeps the page that way. An `async` script may run AFTER `DOMContentLoaded`, which reopens the
//! race. A `defer` script would run before the event, but out of step with the classic ones
//! around it. A module gets its own scope, so `main.js` could not see `renderSigner` at all.
//! None of the three belongs in this page.
//!
//! `main.js` and `index.html` are untyped, so these tests read their source, like the
//! field-name drift guards beside them.
//!
//! # What the first test can and cannot see
//!
//! It is a line heuristic, not a parser. It reads only lines that start in column 0, so it
//! catches the regression in the shape it had (a bare top-level `void pollLock();`). It does NOT
//! catch a load-time call nested inside an indented body, such as an immediately-invoked
//! `(async () => {\n  await pollLock();\n})();`. Those lines look exactly like the calls inside
//! event handlers, which are fine because they run only after the page has loaded. A reviewer
//! adding top-level code that runs at load must apply the rule by hand: the lock is first polled
//! from `DOMContentLoaded`, nowhere else.

const MAIN_JS: &str = include_str!("../../src-ui/main.js");
const INDEX_HTML: &str = include_str!("../../src-ui/index.html");

/// The three top-level lines of `main.js` that may name `pollLock`: its definition, the first poll
/// (deferred to `DOMContentLoaded`), and the timer. A top-level line is one that starts in
/// column 0; a comment line is not code.
const TOP_LEVEL_POLL_LOCK: [&str; 3] = [
    "async function pollLock() {",
    "document.addEventListener(\"DOMContentLoaded\", () => void pollLock());",
    "setInterval(pollLock, LOCK_POLL_MS);",
];

#[test]
fn the_first_lock_poll_waits_for_every_script_to_load() {
    let top_level: Vec<&str> = MAIN_JS
        .lines()
        .filter(|line| !line.starts_with(char::is_whitespace) && !line.starts_with("//"))
        .filter(|line| line.contains("pollLock"))
        .collect();
    assert_eq!(
        top_level, TOP_LEVEL_POLL_LOCK,
        "main.js may poll the lock at load time only from DOMContentLoaded: a poll started while \
         main.js is still loading can land before signer.js defines renderSigner (#782)"
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
        tags.iter().any(|t| t.contains("src=\"signer.js\"")),
        "index.html loads signer.js: {tags:?}"
    );
    for tag in tags {
        for word in ["async", "defer", "type=\"module\""] {
            assert!(
                !tag.contains(word),
                "{tag} is `{word}`: async can run after DOMContentLoaded, defer out of step with \
                 the classic scripts, and a module outside their shared scope — each can cost \
                 the lock hint again (#782)"
            );
        }
    }
}
