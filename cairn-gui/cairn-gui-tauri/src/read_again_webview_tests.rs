//! Guards on #797's "Read this chart again" control: it is always in the chart view and
//! re-reads the chart, every sentence that sends the clinician to it quotes its label exactly,
//! and no sentence sends them to a "reload" the window does not have. Split out of
//! `reread_webview_tests.rs` (which keeps the stale-refusal re-read) to keep both under 500 lines.
//!
//! Like that file, the script and markup have no test rig of their own (no npm or bundler:
//! main.js's header, issue #332), so each guard reads the source (`crate::source_guard`).
use crate::source_guard::position;

const CHART_STALE_JS: &str = include_str!("../src-ui/chart_stale.js");
const INDEX_HTML: &str = include_str!("../src-ui/index.html");

/// The name of the control, as the clinician reads it. Three sentences quote it; this is the one
/// place the guards below agree on, so renaming the button without them (or the reverse) is red.
const READ_AGAIN_LABEL: &str = "Read this chart again";

const DUPLICATES_JS: &str = include_str!("../src-ui/duplicates.js");
const RECORD_LINKS_RS: &str = include_str!("link/record_links.rs");

/// The control that reads the chart again is in the page whatever state the chart is in: a plain
/// button inside `#chart-view`, under the identity header, outside every list that can fail to
/// draw — so a failed read always leaves a way out. Its label is the name the sentences quote, and
/// its click is wired to `refresh()` (in chart_stale.js: main.js has no room left, #332).
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
        .contains("el(\"reread\").addEventListener(\"click\", () => void refresh());"));
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
    assert!(pieces(DUPLICATES_JS, "REVIEW_WAITS_FOR_LIST").contains(&quoted));
    // record_links.rs words its four read-failure sentences with `press \"…\"` (an escaped quote).
    let escaped = format!("press \\\"{READ_AGAIN_LABEL}\\\"");
    assert_eq!(RECORD_LINKS_RS.matches(&escaped).count(), 4);
    assert!(
        include_str!("duplicates/view.rs").contains(&format!("press \\\"{READ_AGAIN_LABEL}\\\""))
    );
}

/// This window has a control for reading a chart again, and the sentences name it. Neither
/// "reload the chart" (no such control ever existed) nor "refresh and try again" (the stale case
/// that now re-reads by itself) may be said to a clinician: not in a Rust sentence, not in a
/// script. Test files are exempt, since they must quote the phrase to forbid it.
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
            let is_source = name.ends_with(".js") || name.ends_with(".rs");
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
