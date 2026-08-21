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
///
/// Over a FIXTURE tree with a known answer, not over `db/` compared against a sorted copy of
/// itself. That earlier shape detected a deleted `out.sort()` only when `read_dir` happened to
/// return `db/` unsorted — true on APFS today, filesystem-defined in general, so its detection
/// rate depended on the machine (#456 review). Here the files are created in an order that is
/// deliberately not their sorted order, and the expected sequence is written out.
#[test]
fn output_is_sorted() {
    let tmp = std::env::temp_dir().join(format!("cairn-sort-order-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("b")).expect("fixture: b/");
    // Created c, a, b/a — sorted order is a, b/a, c. Any of the three orderings a filesystem
    // might hand back is wrong, so the assertion cannot pass by accident.
    for (path, body) in [
        ("c.sql", "SELECT 3;\n"),
        ("a.sql", "SELECT 1;\n"),
        ("b/a.sql", "SELECT 2;\n"),
    ] {
        std::fs::write(tmp.join(path), body).expect("fixture: file");
    }

    let files = source_files(std::slice::from_ref(&tmp), &["target"], &["sql"]);
    let relative: Vec<String> = files
        .iter()
        .map(|p| {
            p.strip_prefix(&tmp)
                .expect("walk results sit under the fixture root")
                .to_string_lossy()
                .into_owned()
        })
        .collect();

    // Clean up BEFORE asserting, so a failure leaves nothing behind in /tmp.
    std::fs::remove_dir_all(&tmp).expect("fixture teardown");

    assert_eq!(
        relative,
        vec!["a.sql", "b/a.sql", "c.sql"],
        "source_files must return sorted paths regardless of creation or read_dir order"
    );
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

/// A symlink is neither descended into nor collected (#452) — the walk's HEADLINE property.
///
/// This is the fix `sources.rs` exists for, and until the PR #456 review it had **no test at
/// all**: swapping `DirEntry::file_type` back for `Path::is_dir` — one word, in two places —
/// passed every other test in this file and all three guard binaries. It could not be caught
/// incidentally either, because the repository contains no symlink (`find crates db extensions
/// -type l` is empty), so the tree supplies no coverage of its own. A guard's most important
/// property being the one nothing checks is the exact species this PR is about.
///
/// The directory case is the load-bearing half: `Path::is_dir` follows a symlink, so one
/// pointing at an ancestor makes the walk unbounded — a hang in a required check, which
/// presents as CI flakiness rather than as a defect.
///
/// Deliberately built with a link to a SIBLING and not to an ancestor. An ancestor link would
/// demonstrate the hang, and a regression would then *hang CI* instead of failing it — a worse
/// outcome than the bug. A sibling link terminates either way: if the walk follows it, the
/// marker file below appears and this fails fast, naming it.
///
/// Unix-only because `std::os::unix::fs::symlink` is; the same property holds on Windows, and
/// no contributor rig or CI runner in this project is Windows today.
#[cfg(unix)]
#[test]
fn symlinks_are_neither_followed_nor_collected() {
    use std::os::unix::fs::symlink;

    // A hand-rolled temporary directory rather than a `tempfile` dev-dependency: one test does
    // not justify a new crate in the supply chain (house rule 1 asks for a licence check on
    // every dependency, and this needs none). Named by process id so a parallel `cargo test`
    // of another binary cannot collide.
    let tmp = std::env::temp_dir().join(format!("cairn-source-walk-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let walked = tmp.join("walked");
    let elsewhere = tmp.join("elsewhere");
    std::fs::create_dir_all(&walked).expect("fixture: walked/");
    std::fs::create_dir_all(&elsewhere).expect("fixture: elsewhere/");
    std::fs::write(tmp.join("real.rs"), "fn main() {}\n").expect("fixture: real.rs");
    std::fs::write(elsewhere.join("marker.rs"), "fn m() {}\n").expect("fixture: marker.rs");
    symlink(&elsewhere, walked.join("link_to_dir")).expect("fixture: dir symlink");
    symlink(tmp.join("real.rs"), walked.join("link_to_file.rs")).expect("fixture: file symlink");

    let found = source_files(std::slice::from_ref(&walked), &["target"], &["rs"]);

    // Clean up BEFORE asserting: a failing assertion must not leave a symlinked tree in
    // /tmp for the next run to trip over.
    std::fs::remove_dir_all(&tmp).expect("fixture teardown");

    assert!(
        found.is_empty(),
        "a symlinked directory must not be descended into and a symlinked file must not be \
         collected — `DirEntry::file_type` does not follow symlinks, `Path::is_dir` does. \
         Found: {found:?}"
    );
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
