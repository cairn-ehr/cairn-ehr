//! #578 review — a quarantine pen row is deleted in exactly ONE place: `cairn_release_pen_row`.
//!
//! # Why this guard exists
//!
//! A pen row can carry a wrapped DEK — a restore pens a sealed record with its key — and on a
//! restored solo node that row may be the last copy of the key in the world. #578 was a `requeue`
//! that deleted such a row because the apply door said `Ok`, and the door says `Ok` on four paths
//! that store no custody at all (`db/052`'s section 4 lists them). Its review then found the SAME
//! bare `DELETE` one screen away, in `pull`'s auto-release, which had never asked either.
//!
//! The fix put the rule in the database: `cairn_release_pen_row` deletes a pen row unless it
//! carries a key whose custody has not landed, and both `requeue` and `pull` release through it.
//! That only holds while nothing ELSE deletes pen rows — and the tempting regression is a one-line
//! "simplification" back to `DELETE FROM sync_quarantine WHERE content_digest = $1`, which reads as
//! obviously correct and passes every behavioural test that never stages a keyed row. Staging one
//! on the pull path needs a peer serving a sealed event without custody while a floor is pinned, so
//! this regression is cheap to make and expensive to test behaviourally; a source guard is the
//! proportionate net.
//!
//! # What it scans, and what it does not
//!
//! Every `db/*.sql` migration and every production `crates/*/src/**/*.rs` file, with comments
//! stripped and the BODY of each `#[cfg(test)]`-gated `mod` skipped (a unit test clearing the pen
//! between arms deletes nothing a node holds). `db/tests/` and `crates/*/tests/` are skipped for the
//! same reason. The match is on the normalised text `delete from sync_quarantine`, across line
//! breaks, so a statement split over two lines is still found.
//!
//! ⚠️ **Only the gated module is skipped, never "everything after the first test gate".** The
//! sibling guards this was modelled on stop at the first `#[cfg(test)] mod` in a file, and in
//! `crates/cairn-node/src/main.rs` that is line ~253 of ~6 500 — `main()` and the whole restore arm
//! come after it (PR #582 review). A module's extent is found by counting braces on comment-stripped
//! lines, which a string literal holding an unbalanced brace could fool; the positive controls in
//! `only_the_release_door_deletes_pen_rows` fail loudly if production code this guard must see ever
//! goes missing from what it scans.
//!
//! It is a regression net against an accidental bypass, not a defence against concealment: SQL
//! assembled at runtime from fragments would pass it. Nothing in this tree does that.

#[path = "common/sources.rs"]
mod sources;

use std::path::Path;

/// (repo-relative file, why a pen row may be deleted there).
const ALLOWED: &[(&str, &str)] = &[(
    "db/052_restore_doors.sql",
    "`cairn_release_pen_row` — THE door. It deletes a pen row unless the row carries a wrapped DEK \
     and `cairn_custody_landed` says custody for its event is not settled. `cairn-sync`'s requeue \
     and pull auto-release both call it.",
)];

/// The statement this guard looks for, in normalised form (lower case, single spaces).
const NEEDLE: &str = "delete from sync_quarantine";

/// True iff this line opens a test module's gate (`#[cfg(test)]` or `#[cfg(any(test, …))]`).
fn is_a_test_gate_attribute(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("#[cfg(test)]")
        || t.starts_with("#[cfg(any(test,")
        || t.starts_with("#[cfg(any(test ,")
}

/// Everything before a comment marker on the line.
fn strip_comment<'a>(line: &'a str, marker: &str) -> &'a str {
    match line.find(marker) {
        Some(i) => &line[..i],
        None => line,
    }
}

/// The lines of a Rust file that are NOT inside a `#[cfg(test)]`-gated `mod`.
///
/// A gate may be followed by further attributes before the `mod` item. `mod tests;` (a module in
/// its own file) is skipped as one line; `mod tests { … }` is skipped to its closing brace, found by
/// counting braces on comment-stripped lines.
fn production_rust_lines<'a>(lines: &[&'a str]) -> Vec<&'a str> {
    let mut kept = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if is_a_test_gate_attribute(lines[i]) {
            let mut item = i + 1;
            while item < lines.len() && {
                let t = lines[item].trim_start();
                t.is_empty() || t.starts_with('#')
            } {
                item += 1;
            }
            if item < lines.len() && lines[item].trim_start().starts_with("mod ") {
                if strip_comment(lines[item], "//").trim_end().ends_with(';') {
                    i = item + 1;
                    continue;
                }
                let (mut depth, mut opened, mut end) = (0i64, false, item);
                while end < lines.len() {
                    for ch in strip_comment(lines[end], "//").chars() {
                        match ch {
                            '{' => {
                                depth += 1;
                                opened = true;
                            }
                            '}' => depth -= 1,
                            _ => {}
                        }
                    }
                    end += 1;
                    if opened && depth <= 0 {
                        break;
                    }
                }
                i = end;
                continue;
            }
        }
        kept.push(lines[i]);
        i += 1;
    }
    kept
}

/// The PRODUCTION text of a file, comments removed, as one normalised string.
///
/// Rust: everything before the first `//` on each line, outside test-gated modules. SQL: everything
/// before the first `--`. A comment that NAMES the statement — `db/021`'s header does — is
/// documentation, not a deletion.
fn production_text(path_display: &str, text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let (scanned, marker) = if path_display.ends_with(".sql") {
        (lines, "--")
    } else {
        (production_rust_lines(&lines), "//")
    };
    scanned
        .iter()
        .map(|l| strip_comment(l, marker))
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn deletes_pen_rows(path_display: &str, text: &str) -> bool {
    production_text(path_display, text).contains(NEEDLE)
}

/// The matcher, on synthetic text — so a guard that silently stopped matching anything cannot pass
/// by finding nothing.
#[test]
fn the_matcher_finds_a_split_statement_and_ignores_comments_and_test_modules() {
    assert!(deletes_pen_rows(
        "x.rs",
        "fn f() { c.execute(\"DELETE FROM\n    sync_quarantine WHERE content_digest = $1\", &[]); }\n"
    ));
    assert!(deletes_pen_rows(
        "x.sql",
        "WITH r AS (delete from SYNC_QUARANTINE where true) SELECT 1;\n"
    ));
    assert!(!deletes_pen_rows(
        "x.rs",
        "// DELETE FROM sync_quarantine was the defect\nfn f() {}\n"
    ));
    assert!(!deletes_pen_rows(
        "x.sql",
        "-- DELETE (on successful requeue) FROM sync_quarantine is legitimate\nSELECT 1;\n"
    ));
    assert!(!deletes_pen_rows(
        "x.rs",
        "fn ships() {}\n#[cfg(test)]\nmod tests {\n    fn t() { q(\"DELETE FROM sync_quarantine\"); }\n}\n"
    ));
    // PRODUCTION CODE AFTER A TEST MODULE IS STILL PRODUCTION — the gap the first version had.
    assert!(deletes_pen_rows(
        "x.rs",
        "#[cfg(test)]\nmod early_tests {\n    fn t() { if x { y() } }\n}\nfn ships() { q(\"DELETE FROM sync_quarantine\"); }\n"
    ));
    // A file-module gate skips one line, not the rest of the file.
    assert!(deletes_pen_rows(
        "x.rs",
        "#[cfg(test)]\nmod tests;\nfn ships() { q(\"DELETE FROM sync_quarantine\"); }\n"
    ));
}

#[test]
fn only_the_release_door_deletes_pen_rows() {
    let root = sources::repo_root();
    let roots = vec![root.join("db"), root.join("crates")];
    let mut found: Vec<String> =
        sources::source_files(&roots, &["target", "tests"], &["sql", "rs"])
            .into_iter()
            .filter_map(|path| {
                let rel = path
                    .strip_prefix(&root)
                    .unwrap_or(Path::new(""))
                    .display()
                    .to_string();
                deletes_pen_rows(&rel, &sources::read_source(&path)).then_some(rel)
            })
            .collect();
    found.sort();

    // POSITIVE CONTROLS: production code this guard must see is in what it scans. Without these, a
    // module-skipping bug that swallowed most of a file would pass by finding nothing there.
    for (file, needle) in [
        ("crates/cairn-node/src/main.rs", "async fn main"),
        ("crates/cairn-sync/src/main.rs", "fn do_pull("),
        ("crates/cairn-sync/src/main.rs", "fn do_requeue("),
    ] {
        let text = sources::read_source(&root.join(file));
        assert!(
            production_text(file, &text).contains(needle),
            "anti-vacuity: `{needle}` in {file} is production code, and the scan no longer sees it \
             — the test-module skipping has swallowed code it must check"
        );
    }

    let allowed: Vec<String> = ALLOWED.iter().map(|(f, _)| (*f).to_string()).collect();
    assert_eq!(
        found, allowed,
        "a quarantine pen row is now deleted somewhere other than `cairn_release_pen_row`.\n\n\
         A pen row may carry a wrapped DEK that is the ONLY copy of a record's key, and the apply \
         door returns OK on paths that store no custody — so a bare DELETE after the door is the \
         #578 defect. Release through `cairn_release_pen_row` (`db/052`) instead; `cairn-sync` has \
         `release_pen_row` for it. If this is genuinely a new, safe deletion site, add it to \
         ALLOWED with the reason, in the same commit."
    );
}
