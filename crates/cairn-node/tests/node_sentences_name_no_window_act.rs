//! #797 / #806 — a node's refusal sentence says only what happened; it never names a WINDOW act.
//!
//! # Why this guard exists
//!
//! The node's sentences reach more than one surface: the reference window shows them, and so
//! does the CLI. A remedy written into the sentence ("reload the chart", "refresh the list",
//! "review it again") names a control only ONE of those surfaces has — and after #797 the window
//! has no "reload" at all: a refusal over a changed record is typed `RefusalScope::Stale`, and the
//! window re-reads by itself. So the remedy is the SCOPE's job, and each surface words its own
//! (the window acts on `Retry::AfterReread`; the CLI prints `db_diagnosis::cli_remedy`'s line).
//!
//! The window's own tree has the mirror guard, `no_window_sentence_says_reload_the_chart`
//! (`cairn-gui-tauri/src/read_again_webview_tests.rs`), which cannot see this crate.
//!
//! # What it scans
//!
//! Every `crates/cairn-node/src/**/*.rs` line that is not a comment (`//`, `///`, `//!`), for a
//! fixed list of phrases. Plain text matching: a phrase split across a `\` line continuation
//! would pass, so the phrases are short enough that no sentence here splits one.

use std::path::{Path, PathBuf};

/// Phrases that name an act only a window can perform.
const WINDOW_ACTS: [&str; 6] = [
    "reload the chart",
    "refresh the list",
    "refresh and try again",
    "review it again",
    "compare again",
    "close the chart",
];

/// Every `.rs` file under `dir`, recursively. Pure apart from reading the directory.
fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir).expect("source dir reads") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            found.extend(rust_files(&path));
        } else if path.extension().is_some_and(|e| e == "rs") {
            found.push(path);
        }
    }
    found
}

/// The `file:line: phrase` of every non-comment line of `text` holding a window act.
fn window_acts_in(file: &str, text: &str) -> Vec<String> {
    text.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim_start().starts_with("//"))
        .flat_map(|(n, line)| {
            let lower = line.to_lowercase();
            WINDOW_ACTS
                .iter()
                .filter(move |phrase| lower.contains(*phrase))
                .map(move |phrase| format!("{file}:{}: {phrase}", n + 1))
        })
        .collect()
}

#[test]
fn no_node_sentence_names_a_window_act() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let files = rust_files(&src);
    // Anti-vacuity: the walk found the crate's sources, not an empty directory.
    assert!(files.len() > 50, "only {} source files found", files.len());
    let hits: Vec<String> = files
        .iter()
        .flat_map(|path| {
            let text = std::fs::read_to_string(path).expect("source reads");
            window_acts_in(&path.display().to_string(), &text)
        })
        .collect();
    assert!(
        hits.is_empty(),
        "a node sentence names a window act — say only what happened (\"… — nothing was done\") \
         and let the refusal's scope carry the remedy:\n{}",
        hits.join("\n")
    );
}

/// The matcher itself: a comment may name a window act (it explains the rule), a sentence may not.
#[test]
fn a_comment_may_name_a_window_act_and_a_sentence_may_not() {
    let text = "// the window's \"review it again\" button\n    \"… — nothing was done; Review it again\",";
    assert_eq!(window_acts_in("f.rs", text), ["f.rs:2: review it again"]);
}
