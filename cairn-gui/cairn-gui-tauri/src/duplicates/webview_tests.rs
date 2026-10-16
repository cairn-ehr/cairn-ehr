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
        position(caught, "say(") < position(caught, "await redrawBannerAlone(patient);"),
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
/// chart, whose redraw rebuilds every row and would wipe a half-typed stop reason (#799).
#[test]
fn a_decided_refusal_re_reads_the_banner_never_the_whole_chart() {
    let compare = js_fn(LINK_JS, "compare");
    let decided = &compare[position(compare, "failure.retry === \"never\"")..];
    assert!(
        decided.contains("await redrawBannerAlone(patientId);"),
        "{compare}"
    );
    assert!(!compare.contains("refresh("), "{compare}");

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

/// The two sentences the window words itself about a missing list must be TRUE in both cases
/// that reach them — a read that failed, and one still in flight — and must name the way out,
/// since nothing re-reads a failed first read by itself (PR #798 review; #797 tracks a control).
#[test]
fn the_windows_own_sentences_about_a_missing_list_are_true() {
    let no_list = js_const(DUPLICATES_JS, "NO_LIST_TO_COMPARE");
    for part in ["could not be read", "still being read", "nothing was done"] {
        assert!(no_list.contains(part), "{no_list}");
    }
    let waits = js_const(DUPLICATES_JS, "REVIEW_WAITS_FOR_LIST");
    for part in [
        "could not be read",
        "still being read",
        "close the chart and open it again",
    ] {
        assert!(waits.contains(part), "{waits}");
    }
}
