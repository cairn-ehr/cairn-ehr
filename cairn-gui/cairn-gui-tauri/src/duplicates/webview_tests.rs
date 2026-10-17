//! Guards on the webview's half of the banner's failure states (#745, PR #798 review).
//!
//! Every fix here lives in JavaScript — which banner is drawn after a failed read, which re-read
//! follows a refusal, what is said when no list is on screen — and the webview has no test rig of
//! its own (no npm, by design: `docs/developers/durable-rules.md`). So each guard reads the
//! script's source (`crate::source_guard`) and pins the ORDER or the CHOICE that the fix is.
//! Reverting any of these fixes leaves every behavioural test green; these turn red.
use super::view::DuplicateSection;
use crate::source_guard::{js_const, js_fn, position};
use std::collections::BTreeSet;

const DUPLICATES_JS: &str = include_str!("../../src-ui/duplicates.js");
const MAIN_JS: &str = include_str!("../../src-ui/main.js");
const LINK_JS: &str = include_str!("../../src-ui/link.js");

/// The only section duplicates.js builds itself (`emptyBanner`) carries exactly the fields the
/// backend sends. `renderDuplicates` reads a payload's fields without checking them, so a field
/// Rust adds and the window reads would make a hand-built section THROW — and the drawing-fault
/// fallback is itself a hand-built section, so the throw would escape into `refresh` and be
/// misreported as "Could not read the chart" (#745 point 4, type-design review of PR #798).
/// `silent` is the one field JS decides here, and it is Rust's rule for an empty section: silent
/// only when there is no error line to say.
#[test]
fn the_one_section_the_window_builds_has_every_field_the_backend_sends() {
    let built = js_fn(DUPLICATES_JS, "emptyBanner");
    let literal = &built[position(built, "return {") + "return {".len()..];
    let literal = &literal[..literal.find('}').expect("the literal closes")];
    let keys: BTreeSet<String> = literal
        .split(',')
        .filter_map(|pair| pair.split(':').next())
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty())
        .collect();
    let sent: BTreeSet<String> = serde_json::to_value(DuplicateSection::checked_none_open())
        .unwrap()
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    assert_eq!(keys, sent, "emptyBanner's literal: {literal}");
    assert!(literal.contains("silent: !error"), "{literal}");
    // ONE place builds a section: a second literal is a second copy of the shape to drift. Code
    // lines only — prose ("Never silent: …") is not a literal.
    let code_lines_setting_silent = DUPLICATES_JS
        .lines()
        .map(str::trim_start)
        .filter(|line| !line.starts_with('*') && !line.starts_with("//"))
        .filter(|line| line.contains("silent:"))
        .count();
    assert_eq!(code_lines_setting_silent, 1);
    assert!(js_fn(DUPLICATES_JS, "clearDuplicates").contains("emptyBanner(null)"));
}

/// A banner that fails to DRAW says so on itself and keeps what it could still say: its own error
/// line and check lines ("Duplicate check status unknown" is a safety line, not decoration). The
/// fault goes to the console too, so whoever debugs it has the stack (PR #798 review).
#[test]
fn a_banner_that_fails_to_draw_keeps_its_own_lines_and_logs_the_fault() {
    let draw = js_fn(DUPLICATES_JS, "drawDuplicates");
    let caught = &draw[position(draw, "catch (failure)")..];
    assert!(caught.contains("console.error(failure)"), "{draw}");
    assert!(caught.contains("salvagedBanner(section,"), "{draw}");
    let salvaged = js_fn(DUPLICATES_JS, "salvagedBanner");
    assert!(salvaged.contains("section.check_lines"), "{salvaged}");
    assert!(salvaged.contains("section.error"), "{salvaged}");
}

/// #745 points 1, 2 and 4. A failed chart read still draws the banner — read on its own, AFTER
/// the outcome is said — and a fault drawing the banner inside `render` is said on the banner,
/// never thrown into `refresh`'s "Could not read the chart".
#[test]
fn a_failed_chart_read_still_draws_the_banner() {
    let refresh = js_fn(MAIN_JS, "refresh");
    let caught = &refresh[position(refresh, "catch (e)")..];
    assert!(
        position(caught, "sayReadFailed(lead, e);")
            < position(caught, "await redrawBannerAlone(patient);"),
        "{caught}"
    );
    let render = js_fn(MAIN_JS, "render");
    assert!(
        render.contains("drawDuplicates(pane.duplicates)"),
        "{render}"
    );
    assert!(!render.contains("renderDuplicates("), "{render}");
}

/// After a failed FIRST read the sign-off button must not go on saying "Loading…" (PR #798
/// review): nothing is loading any more, and a clinician would wait for a read that never comes.
#[test]
fn a_failed_first_read_does_not_leave_sign_off_saying_loading() {
    let refresh = js_fn(MAIN_JS, "refresh");
    let caught = &refresh[position(refresh, "catch (e)")..];
    let unread = &caught[position(caught, "if (renderedPatient === null)")..];
    assert!(
        unread.contains("el(\"sign-off\").textContent ="),
        "{caught}"
    );
}

/// #745 point 3 and the I2 fix of PR #798's review. A decided (`"never"`) refusal of a comparison
/// or a judgement re-reads the BANNER — the window has no reload control — and never the whole
/// chart: the banner is the only stale thing, and a whole re-read is heavier and rebuilds every
/// row under the clinician (since #799 a typed stop reason and its caret survive it; focus on
/// anything else the re-read rebuilds — a Stop or Review button, a link line — does not).
#[test]
fn a_decided_refusal_re_reads_the_banner_never_the_whole_chart() {
    let compare = js_fn(LINK_JS, "compare");
    // The `"never"` arm alone, up to the `after_reread` arm after it (#797), which DOES re-read
    // the whole chart: a refusal that says the screen is behind the record.
    let never_at = position(compare, "failure.retry === \"never\"");
    let reread_at = position(compare, "} else if (reread");
    let decided = &compare[never_at..reread_at];
    assert!(
        decided.contains("await redrawBannerAlone(patientId);"),
        "{compare}"
    );
    assert!(!decided.contains("refresh("), "{compare}");
    assert!(
        never_at < reread_at && reread_at < position(compare, "refresh("),
        "the only whole-chart re-read is the after_reread arm's: {compare}"
    );

    let send = js_fn(LINK_JS, "sendJudgement");
    assert!(
        send.contains("const decided = Boolean(failure && failure.retry === \"never\");"),
        "{send}"
    );
    assert!(
        send.contains("else if (decided && touched) await redrawBannerAlone(displayedPatient);"),
        "{send}"
    );
}

/// PR #798 review I1. With no medication list drawn (`renderedPatient` null) there is no record on
/// screen to compare with, and the backend would be sent no chart: `compare` refuses before it
/// clears the panel or sends anything, and the banner offers no Review it could only refuse.
#[test]
fn nothing_is_compared_without_a_drawn_list() {
    let compare = js_fn(LINK_JS, "compare");
    let guard = position(compare, "if (patientId === null)");
    assert!(guard < position(compare, "clearComparison();"), "{compare}");
    assert!(
        guard < position(compare, "invoke(\"compare_records\""),
        "{compare}"
    );

    let item = js_fn(DUPLICATES_JS, "duplicateItem");
    assert!(
        position(item, "if (renderedPatient === null)")
            < position(item, "createElement(\"button\")"),
        "{item}"
    );
}

/// The two sentences the window words itself about a missing list must be TRUE in every case
/// that reaches them — a read that failed, one still in flight, and a list read but not drawable
/// (#801, PR #802 review) — and must name the way out where there is one: nothing re-reads a
/// failed first read by itself (#797 tracks a control); a drawing fault is the window's own.
#[test]
fn the_windows_own_sentences_about_a_missing_list_are_true() {
    let no_list = js_const(DUPLICATES_JS, "NO_LIST_TO_COMPARE");
    for part in [
        "could not be read or shown",
        "still being read",
        "nothing was done",
    ] {
        assert!(no_list.contains(part), "{no_list}");
    }
    // Single-quoted in the script (it quotes the control's name), which `js_const` cannot read.
    let at = DUPLICATES_JS
        .find("const REVIEW_WAITS_FOR_LIST =")
        .expect("declared");
    let waits = &DUPLICATES_JS[at..at + DUPLICATES_JS[at..].find(";\n").expect("closes")];
    for part in [
        "could not be read or shown",
        "still being read",
        "If it could not be read, press \"Read this chart again\"",
    ] {
        assert!(waits.contains(part), "{waits}");
    }
}
