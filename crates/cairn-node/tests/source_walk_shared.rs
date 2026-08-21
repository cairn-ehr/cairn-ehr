//! #452 — the shared source walk's own tests, in ONE home.
//!
//! `tests/common/sources.rs` is pulled into three guard binaries with `#[path]`. Its self-tests
//! deliberately do NOT live inside it: Cargo compiles `tests/*.rs` with `--test`, which sets
//! `cfg(test)`, so a `#[cfg(test)] mod tests` inside the shared module would be compiled and RUN
//! once per including binary — three identical copies of every assertion, three times the wall
//! clock, and three places a failure has to be read. One home instead.
//!
//! What these pin is the walk's *loudness* and its *scope*, because those are the two properties
//! the three former copies disagreed about, and both fail in the direction that looks green.
#[path = "common/sources.rs"]
mod sources;

use sources::{read_source, repo_root, source_files};

/// The walk finds files by extension.
///
/// Asserted against this repository rather than a synthetic fixture tree: `db/` is guaranteed to
/// hold `.sql` files and no `.rs` files, which makes the extension filter observable in both
/// directions without building and cleaning up a temporary directory.
#[test]
fn walks_by_extension() {
    let sql = source_files(&[repo_root().join("db")], &["target"], &["sql"]);
    assert!(!sql.is_empty(), "db/ holds .sql files");
    assert!(
        sql.iter()
            .all(|p| p.extension().is_some_and(|e| e == "sql")),
        "the extension filter must exclude everything else"
    );

    let none = source_files(&[repo_root().join("db")], &["target"], &["rs"]);
    assert!(
        none.is_empty(),
        "db/ holds no .rs files, so the filter must return nothing: {none:?}"
    );
}

/// A skipped directory name is skipped at every depth.
///
/// Asserted as a DIFFERENCE between two walks rather than as an absolute count, so it cannot
/// pass by the skip list doing nothing at all — which is how a scope guard goes quietly wrong.
#[test]
fn skip_dirs_removes_files_that_would_otherwise_be_found() {
    let root = repo_root().join("crates/cairn-node");
    let with_tests = source_files(std::slice::from_ref(&root), &["target"], &["rs"]);
    let without = source_files(&[root], &["target", "tests"], &["rs"]);

    assert!(
        without.len() < with_tests.len(),
        "skipping tests/ must remove files: {} without vs {} with",
        without.len(),
        with_tests.len()
    );
    assert!(
        !without
            .iter()
            .any(|p| p.components().any(|c| c.as_os_str() == "tests")),
        "no file under a skipped directory may survive the walk"
    );
}

/// Two roots are both walked — the multi-root case every caller but one relies on.
#[test]
fn every_root_is_walked() {
    let db = repo_root().join("db");
    let crates = repo_root().join("crates");
    let both = source_files(&[db.clone(), crates.clone()], &["target"], &["rs", "sql"]);

    assert!(
        both.iter().any(|p| p.starts_with(&db)),
        "the db/ root contributed nothing"
    );
    assert!(
        both.iter().any(|p| p.starts_with(&crates)),
        "the crates/ root contributed nothing"
    );
}

/// Output order is deterministic, so a failing guard names its offenders identically twice.
#[test]
fn output_is_sorted() {
    let files = source_files(&[repo_root().join("db")], &["target"], &["sql"]);
    let mut expected = files.clone();
    expected.sort();
    assert_eq!(files, expected, "source_files must return sorted paths");
}

/// An unreadable root PANICS naming the path rather than contributing nothing.
///
/// This is the defect the shared walk exists to stop repeating: `event_log_row_by_name.rs`
/// returned early on a `read_dir` error, so a mistyped or moved root produced an empty file list
/// and a guard that passed by examining nothing.
#[test]
#[should_panic(expected = "unreadable source directory")]
fn a_missing_root_is_loud() {
    source_files(
        &[repo_root().join("no-such-directory-exists-here")],
        &["target"],
        &["rs"],
    );
}

/// And so does an unreadable file — the same defect one level down (`else { continue }`).
#[test]
#[should_panic(expected = "unreadable source file")]
fn a_missing_file_is_loud() {
    read_source(&repo_root().join("no-such-file-exists-here.rs"));
}

/// `repo_root()` really is the repository root, checked against landmarks it does not choose.
///
/// Without this, a change to the crate layout could silently retarget every guard that walks from
/// here at a subdirectory — each of them would then scan a smaller tree and stay green, which is
/// the `git ls-files`-scoping trap PR #448 hit from the other direction.
#[test]
fn repo_root_is_the_repository_root() {
    let root = repo_root();
    for landmark in ["CLAUDE.md", "CONTRIBUTING.md", "Cargo.toml"] {
        assert!(
            root.join(landmark).is_file(),
            "{} is not the repo root: {landmark} is missing",
            root.display()
        );
    }
    assert!(root.join("db").is_dir(), "db/ missing from the repo root");
}
