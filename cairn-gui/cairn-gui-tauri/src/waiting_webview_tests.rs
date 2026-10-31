//! Guards on the waiting bar (#875 option 2, `src-ui/waiting.js`): every command goes through
//! it, it shows only after 2 s, and its end is the moment the window's connection cuts a stuck
//! command off. The script has no test rig of its own (no npm or bundler: main.js's header,
//! issue #332), so each guard reads the source (`crate::source_guard`). Its look, and
//! VoiceOver's single announcement, are RUNBOOK §14's human pass.
use crate::source_guard::{js_fn, position};
use cairn_gui_live::NodeConnection;

const WAITING_JS: &str = include_str!("../src-ui/waiting.js");
const INDEX_HTML: &str = include_str!("../src-ui/index.html");
/// Every script index.html loads (`every_command_goes_through_the_bar` checks it against the
/// page, so a new script cannot go unscanned).
const SCRIPTS: [(&str, &str); 12] = [
    ("main.js", include_str!("../src-ui/main.js")),
    ("waiting.js", include_str!("../src-ui/waiting.js")),
    ("chart_stale.js", include_str!("../src-ui/chart_stale.js")),
    ("duplicates.js", include_str!("../src-ui/duplicates.js")),
    ("enrol.js", include_str!("../src-ui/enrol.js")),
    ("funnel.js", include_str!("../src-ui/funnel.js")),
    ("holds.js", include_str!("../src-ui/holds.js")),
    ("link.js", include_str!("../src-ui/link.js")),
    ("signer.js", include_str!("../src-ui/signer.js")),
    ("stop_reasons.js", include_str!("../src-ui/stop_reasons.js")),
    ("unlink.js", include_str!("../src-ui/unlink.js")),
    ("worklist.js", include_str!("../src-ui/worklist.js")),
];

/// The number a top-level `const name = 12_345;` declares, underscores dropped.
fn js_number(src: &str, name: &str) -> u128 {
    let start = position(src, &format!("const {name} = ")) + name.len() + 9;
    let end = start + position(&src[start..], ";");
    src[start..end].replace('_', "").parse().expect("a number")
}

/// The bar runs out exactly when the connection cuts a stuck command off: a bar that ended
/// earlier would say "Reconnecting…" over a command still allowed to answer, and one that ended
/// later would promise time the connection does not give.
#[test]
fn the_bar_ends_when_the_connection_cuts_off() {
    assert_eq!(
        js_number(WAITING_JS, "NODE_ANSWERS_WITHIN_MS"),
        NodeConnection::ANSWER_WITHIN.as_millis()
    );
    assert_eq!(
        js_number(WAITING_JS, "WAIT_SHOWN_AFTER_MS"),
        2_000,
        "the maintainer's 2 s"
    );
}

/// Every command goes through the one wrapper: main.js binds `invoke` to it, and no script
/// reaches Tauri's own invoke past it, so no command can wait without the bar.
#[test]
fn every_command_goes_through_the_bar() {
    let main = SCRIPTS[0].1;
    assert!(main.contains("const invoke = watchWaits(window.__TAURI__.core.invoke);"));
    for (name, src) in SCRIPTS {
        assert_eq!(
            src.matches("__TAURI__").count(),
            usize::from(name == "main.js"),
            "{name} reaches Tauri past the bar"
        );
    }
    assert!(
        position(INDEX_HTML, "<script src=\"waiting.js\">")
            < position(INDEX_HTML, "<script src=\"main.js\">"),
        "waiting.js loads first, so main.js can wrap with it"
    );
    let opener = "<script src=\"";
    let mut loaded: Vec<&str> = INDEX_HTML
        .match_indices(opener)
        .map(|(at, _)| {
            let name = &INDEX_HTML[at + opener.len()..];
            &name[..position(name, "\"")]
        })
        .collect();
    let mut scanned: Vec<&str> = SCRIPTS.iter().map(|(name, _)| *name).collect();
    loaded.sort_unstable();
    scanned.sort_unstable();
    assert_eq!(loaded, scanned, "SCRIPTS is every script index.html loads");
}

/// The bar is never drawn for a wait under WAIT_SHOWN_AFTER_MS: the wrapper's timer of that
/// length is cancelled by the answer, `drawWait` shows nothing for a wait under the delay, and
/// re-arms itself for that wait instead, since a timer can fire a little early (PR #892 review).
#[test]
fn the_bar_shows_only_after_the_delay() {
    let wrapper = js_fn(WAITING_JS, "watchWaits");
    let timer = position(wrapper, "setTimeout(drawWait, WAIT_SHOWN_AFTER_MS)");
    let sent = position(wrapper, "await send(...args)");
    assert!(timer < sent, "the timer starts before the command is sent");
    assert!(
        position(wrapper, "clearTimeout(showTimer)") > sent,
        "an answer cancels it"
    );
    let draw = js_fn(WAITING_JS, "drawWait");
    let gate = position(draw, "waited < WAIT_SHOWN_AFTER_MS");
    assert!(
        gate < position(draw, "bar.hidden = false"),
        "drawWait shows nothing for a wait under the delay"
    );
    assert!(
        position(
            draw,
            "drawAgainTimer = setTimeout(drawWait, WAIT_SHOWN_AFTER_MS - waited)"
        ) < position(draw, "bar.hidden = false"),
        "a wait not yet at the delay is drawn when it gets there"
    );
    assert!(
        position(wrapper, "try {\n        drawWait();") > position(wrapper, "} finally {"),
        "a bar that cannot be drawn never replaces the command's own result"
    );
}

/// The line carries the meaning and is a live region; the emptying bar is a second channel,
/// hidden from assistive technology, so VoiceOver hears one sentence, not a stream of values.
#[test]
fn the_line_is_spoken_and_the_bar_is_not() {
    let bar = position(INDEX_HTML, "<div id=\"node-wait\" hidden>");
    let markup = &INDEX_HTML[bar..bar + position(&INDEX_HTML[bar..], "</div>\n      </div>")];
    assert!(markup.contains("<p id=\"node-wait-line\" role=\"status\" aria-live=\"polite\"></p>"));
    assert!(markup.contains("<div id=\"node-wait-track\" aria-hidden=\"true\">"));
    assert!(
        markup.contains("id=\"node-wait-fill\""),
        "drawWait's third element"
    );
    let draw = js_fn(WAITING_JS, "drawWait");
    assert!(draw.contains("\"Waiting for the node…\""));
    assert!(
        draw.contains("\"Reconnecting…\""),
        "a bar that runs out says so, rather than ending on a wait still in progress"
    );
}
