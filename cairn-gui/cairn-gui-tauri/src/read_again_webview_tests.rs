//! Guards on #797's "Read this chart again" control: it is always in the chart view and
//! re-reads the chart, every sentence that sends the clinician to it quotes its label exactly,
//! and no sentence sends them to a "reload" the window does not have. Split out of
//! `reread_webview_tests.rs` (which keeps the stale-refusal re-read) to keep both under 500 lines.
//!
//! Like that file, the script and markup have no test rig of their own (no npm or bundler:
//! main.js's header, issue #332), so each guard reads the source (`crate::source_guard`).
use crate::source_guard::{js_fn, position};

const CHART_STALE_JS: &str = include_str!("../src-ui/chart_stale.js");
const MAIN_JS: &str = include_str!("../src-ui/main.js");
const FUNNEL_JS: &str = include_str!("../src-ui/funnel.js");
const INDEX_HTML: &str = include_str!("../src-ui/index.html");

/// The name of the control, as the clinician reads it. Every sentence that sends the clinician to
/// it quotes it (`every_sentence_that_names_the_control_quotes_its_label`); this is the one
/// place the guards below agree on, so renaming the button without them (or the reverse) is red.
const READ_AGAIN_LABEL: &str = "Read this chart again";

const DUPLICATES_JS: &str = include_str!("../src-ui/duplicates.js");
const RECORD_LINKS_RS: &str = include_str!("link/record_links.rs");

/// The control that reads the chart again is in the page whatever state the chart is in: a plain
/// button inside `#chart-view`, under the identity header, outside every list that can fail to
/// draw — so a failed read always leaves a way out. Its label is the name the sentences quote, and
/// its click is wired to `refresh()` (in chart_stale.js: main.js is at its 500-line budget).
#[test]
fn the_read_again_control_is_always_there_and_re_reads() {
    let view = position(INDEX_HTML, "<section id=\"chart-view\"");
    let identity_end = view + position(&INDEX_HTML[view..], "</header>");
    let button = view
        + position(
            &INDEX_HTML[view..],
            "<button id=\"reread\" type=\"button\">",
        );
    assert!(
        button > identity_end,
        "the button sits after the identity header"
    );
    let after = &INDEX_HTML[button..];
    let text = &after[after.find('>').unwrap() + 1..position(after, "</button>")];
    assert_eq!(text, READ_AGAIN_LABEL);
    // Still inside #chart-view: the section closes only after the button.
    let section_end = view + position(&INDEX_HTML[view..], "</section>");
    assert!(button < section_end, "the button is inside #chart-view");
    // A plain button with visible text names itself; an aria-label that differed would lie.
    assert!(!INDEX_HTML[button..button + 80].contains("aria-label"));
    // Classic scripts only: `refresh` and `el` are globals the click handler needs.
    for tag in INDEX_HTML.match_indices("<script") {
        let line = &INDEX_HTML[tag.0..tag.0 + position(&INDEX_HTML[tag.0..], ">")];
        for banned in ["defer", "async", "module"] {
            assert!(!line.contains(banned), "{line}");
        }
    }
    assert!(CHART_STALE_JS
        .contains("el(\"reread\").addEventListener(\"click\", () => void refresh(rereadLead()));"));
}

/// The button's re-read keeps what the outcome line says (#797 final review, M2): its lead is
/// the line as it stands, so a change it names is ADDED to the line, and a partial sign-off
/// report ("Signed 2 …; 1 line(s) could NOT be signed …") is never replaced (ADR-0060: partial
/// completion is reported, never implied). Less a read failure the press is retrying: the lead
/// is then what that failure was said after, so pressing again never piles one "Could not read
/// the chart" onto another — and a re-read that draws takes the failure down (it is no longer
/// true), keeping what came before it.
#[test]
fn read_again_keeps_the_outcome_line_but_not_a_failure_it_retries() {
    let failed = js_fn(CHART_STALE_JS, "sayReadFailed");
    let line = position(
        failed,
        "const line = sentences(lead, \"Could not read the chart: \" + error);",
    );
    let said = position(failed, "say(line);");
    let kept = position(failed, "failedRead = { line, lead };");
    assert!(line < said && said < kept, "{failed}");

    let shows = js_fn(CHART_STALE_JS, "showsFailedRead");
    assert!(
        shows.contains("return failedRead !== null && failedRead.line === line;"),
        "{shows}"
    );
    let lead = js_fn(CHART_STALE_JS, "rereadLead");
    assert!(
        lead.contains("const line = el(\"outcome\").textContent;"),
        "{lead}"
    );
    assert!(
        lead.contains("return showsFailedRead(line) ? failedRead.lead : line;"),
        "{lead}"
    );

    // The one place a failed chart read is said, so `failedRead` always knows the line.
    let refresh = js_fn(MAIN_JS, "refresh");
    let caught = &refresh[position(refresh, "catch (e)")..];
    assert!(caught.contains("sayReadFailed(lead, e);"), "{caught}");
    assert!(!MAIN_JS.contains("\"Could not read the chart: \""));

    // A drawn re-read with nothing else to say takes a standing failure down, to its lead.
    let after = js_fn(CHART_STALE_JS, "sayAfterReread");
    let changed = position(after, "if (readAgain || pane.set_change)");
    let cleared = position(
        after,
        "else if (showsFailedRead(el(\"outcome\").textContent)) say(sentences(lead));",
    );
    assert!(changed < cleared, "{after}");
}

/// The only re-read sent with NO lead is the first read of a chart (funnel.js's `enterChart`),
/// right after `clearChart`, which empties the held set: it sends no seen set, so Rust names no
/// change (`seen_set` is `None`), and there is no outcome line yet to keep. Every other
/// `refresh` call passes a lead.
#[test]
fn the_only_lead_less_re_read_is_a_chart_s_first() {
    let scripts = [
        ("main.js", MAIN_JS),
        ("chart_stale.js", CHART_STALE_JS),
        ("link.js", include_str!("../src-ui/link.js")),
        ("unlink.js", include_str!("../src-ui/unlink.js")),
        ("duplicates.js", include_str!("../src-ui/duplicates.js")),
        ("worklist.js", include_str!("../src-ui/worklist.js")),
        ("funnel.js", FUNNEL_JS),
    ];
    // Every CALL with nothing between its parentheses, wherever it sits (`void refresh()`,
    // `await refresh()`, inside an arrow). A comment line naming `refresh()`, a declaration, or
    // a longer name ending in "refresh" does not count.
    let is_call = |js: &str, at: usize| {
        let line_start = js[..at].rfind('\n').map_or(0, |n| n + 1);
        let before = js[line_start..at].trim_start();
        let commented = before.starts_with("//") || before.starts_with('*');
        let named = js[..at].ends_with(|c: char| c.is_alphanumeric() || c == '_');
        !commented && !named && !js[..at].ends_with("function ")
    };
    let bare: Vec<&str> = scripts
        .iter()
        .flat_map(|(file, js)| {
            js.match_indices("refresh()")
                .filter(move |(at, _)| is_call(js, *at))
                .map(move |_| *file)
        })
        .collect();
    assert_eq!(bare, ["funnel.js"], "lead-less re-reads");
    let enter = js_fn(FUNNEL_JS, "enterChart");
    assert!(
        position(enter, "clearChart();") < position(enter, "void refresh();"),
        "{enter}"
    );
    assert!(js_fn(MAIN_JS, "clearChart").contains("holdCharts(null);"));
}

/// Each sentence that sends the clinician to the control quotes its label exactly.
#[test]
fn every_sentence_that_names_the_control_quotes_its_label() {
    let quoted = format!("\"{READ_AGAIN_LABEL}\"");
    let pieces = |src: &str, name: &str| {
        let start = position(src, &format!("const {name} ="));
        let end = start + position(&src[start..], ";\n");
        src[start..end].to_string()
    };
    assert!(pieces(CHART_STALE_JS, "LIST_OUT_OF_DATE").contains(&quoted));
    assert!(pieces(CHART_STALE_JS, "LIST_MAY_BE_OUT_OF_DATE").contains(&quoted));
    assert!(pieces(DUPLICATES_JS, "REVIEW_WAITS_FOR_LIST").contains(&quoted));
    assert!(pieces(DUPLICATES_JS, "NO_LIST_TO_COMPARE").contains(&quoted));
    // The Rust sentences say `press \"…\"` (an escaped quote).
    let escaped = format!("press \\\"{READ_AGAIN_LABEL}\\\"");
    assert_eq!(
        RECORD_LINKS_RS.matches(&escaped).count(),
        4,
        "record_links.rs's read-failure sentences that send the clinician to the button. A new \
         one that does so legitimately: raise this count AND add its own test of the wording; a \
         lower count means a sentence lost its way out"
    );
    assert!(include_str!("duplicates/view.rs").contains(&escaped));
    let chart_set = include_str!("chart_set/mod.rs");
    let unreadable = position(chart_set, "const UNREADABLE: &str =");
    let unreadable = &chart_set[unreadable..unreadable + position(&chart_set[unreadable..], ";\n")];
    assert!(unreadable.contains(&escaped), "{unreadable}");
}

/// This window has a control for reading a chart again, and the sentences name it. Neither
/// "reload the chart" (no such control ever existed) nor "refresh and try again" (the stale case
/// that now re-reads by itself) may be said to a clinician: not in a Rust sentence, not in a
/// script, not in the page. Test files are exempt, since they must quote the phrase to forbid it.
#[test]
fn no_window_sentence_says_reload_the_chart() {
    fn walk(dir: &std::path::Path, hits: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).expect("source dir reads") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                walk(&path, hits);
                continue;
            }
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            // `.html` too: the page's own text (a label, a hint) is said to the clinician as well.
            let is_source = [".js", ".rs", ".html"]
                .iter()
                .any(|ext| name.ends_with(ext));
            if !is_source || name.ends_with("_tests.rs") || name == "tests.rs" {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("source reads");
            for phrase in ["reload the chart", "refresh and try again"] {
                if text.contains(phrase) {
                    hits.push(format!("{}: {phrase}", path.display()));
                }
            }
        }
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut hits = Vec::new();
    walk(&root.join("src"), &mut hits);
    walk(&root.join("src-ui"), &mut hits);
    assert!(hits.is_empty(), "{hits:?}");
}
