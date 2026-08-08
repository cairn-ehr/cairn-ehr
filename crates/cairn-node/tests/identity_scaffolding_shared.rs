//! #120 — the identity integration tests must share ONE copy of their scaffolding.
//!
//! `identity_identify.rs` landed the *third* near-verbatim copy of the identity test
//! scaffolding, copied from `identity_dispute.rs`, itself copied from
//! `identity_linkage.rs` — and the copies had already drifted (the third file's
//! `submit_dispute` had silently dropped the reason/resolution parameter its original
//! carries). Copying is the path of least resistance, so removing the copies without a
//! guard just resets the clock until copy #4.
//!
//! This is a SOURCE-LEVEL guard (no DB needed), the same idiom as
//! `twin_dispatch_single_source.rs` (#173) and `paper_parity_plan_section.rs` (#217): it
//! fails if a bound test file declares a helper that `tests/common/mod.rs` already
//! provides. It runs in every `cargo test` / CI pass.
//!
//! ## What it guards, and what it deliberately does NOT
//!
//! The helper list is DERIVED from `common/mod.rs` rather than restated here, so a helper
//! added there is guarded automatically — a hand-maintained mirror would reintroduce
//! exactly the drift this file exists to prevent.
//!
//! Subtracted from that derived list is [`REPO_WIDE`]: `cs` / `db_msg` / `setup` are
//! project-wide test idioms, declared in 62 / 23 / 27 of this directory's files at the
//! time of writing. They are duplicated far beyond the identity cluster, and binding only
//! `identity_*.rs` against them would be arbitrary — it would flag an identity suite for
//! writing the same four lines that fifty other suites write legitimately. Unifying those
//! is real work with a much wider blast radius, tracked separately. What remains is the
//! set that genuinely belongs to this cluster: the submit path and the projection readers.
//!
//! ## What it binds
//!
//! Every `identity_*.rs`, plus the identity-surface files named in [`ALSO_BOUND`]. Binding
//! by filename prefix ALONE would let copy #4 escape by being called something else —
//! `john_doe.rs` is precisely such a file (§5.4 is the identity surface; it is simply not
//! named `identity_*`), so it is bound explicitly.

use std::fs;
use std::path::{Path, PathBuf};

/// Helpers that `common/mod.rs` provides but this guard does NOT bind, because they are
/// project-wide test idioms rather than identity-cluster copies. See the module header for
/// the counts. Removing an entry here means committing to unify that helper everywhere.
const REPO_WIDE: [&str; 3] = ["fn cs(", "fn db_msg(", "async fn setup("];

/// Files the conversion covered: each must import the shared module. Pinned by name so a
/// rename or accidental deletion fails LOUDLY rather than silently shrinking the scanned
/// set — the anti-vacuity lesson from `paper_parity_plan_section.rs`.
const BOUND: [&str; 4] = [
    "identity_dispute.rs",
    "identity_identify.rs",
    "identity_linkage.rs",
    "john_doe.rs",
];

/// Identity-surface test files NOT named `identity_*`, bound explicitly so the guard's
/// coverage is a deliberate list rather than an accident of naming.
const ALSO_BOUND: [&str; 1] = ["john_doe.rs"];

/// This crate's `tests/` directory. Same `CARGO_MANIFEST_DIR` idiom as
/// `twin_dispatch_single_source.rs`'s `db_dir()`.
fn tests_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .canonicalize()
        .expect("crates/cairn-node/tests/ dir")
}

/// The helper signatures `common/mod.rs` publishes, minus [`REPO_WIDE`].
///
/// Derived by reading the module: every `pub fn` / `pub async fn` line, with `pub `
/// stripped and everything from the argument list onward dropped, leaving a needle like
/// `async fn trust_of(`. `pub struct EventSpec` is not a function and is correctly
/// ignored. Deriving rather than restating is the point — see the module header.
fn guarded_helpers(dir: &Path) -> Vec<String> {
    let src = fs::read_to_string(dir.join("common/mod.rs")).expect("read tests/common/mod.rs");
    src.lines()
        .filter_map(|line| {
            let line = line.trim_start().strip_prefix("pub ")?;
            if !(line.starts_with("fn ") || line.starts_with("async fn ")) {
                return None;
            }
            // Keep the name and the opening paren: `async fn trust_of(c: &Client…` becomes
            // `async fn trust_of(`, which is a precise declaration needle.
            let end = line.find('(')?;
            Some(line[..=end].to_string())
        })
        .filter(|needle| !REPO_WIDE.contains(&needle.as_str()))
        .collect()
}

/// Test files this guard binds: every `identity_*.rs` plus [`ALSO_BOUND`], sorted.
/// Directories (`tests/common/`) and non-Rust files are skipped by the extension check.
fn bound_files(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .expect("read tests/")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("rs"))
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .filter(|n| n.starts_with("identity_") || ALSO_BOUND.contains(&n.as_str()))
        .collect();
    names.sort();
    names
}

/// Which of `needles` the source declares itself.
///
/// Line-start matching (after trimming indentation and any visibility prefix) is what
/// keeps this honest: these helpers are top-level items, while doc comments start `///`
/// and call sites sit inside a function body after a `let`/`assert!`/etc. Neither can
/// trip it, and a declaration pasted into an indented `mod` block still does.
fn locally_declared<'a>(src: &str, needles: &'a [String]) -> Vec<&'a str> {
    needles
        .iter()
        .map(String::as_str)
        .filter(|needle| {
            src.lines().any(|line| {
                let line = line.trim_start();
                // `pub `, `pub(crate) `, `pub(super) ` — any visibility a paste may carry.
                let line = match line.strip_prefix("pub") {
                    Some(rest) => rest.trim_start_matches(|c| c != ' ').trim_start(),
                    None => line,
                };
                line.starts_with(needle)
            })
        })
        .collect()
}

#[test]
fn bound_tests_do_not_redeclare_shared_scaffolding() {
    let dir = tests_dir();
    let needles = guarded_helpers(&dir);
    let files = bound_files(&dir);

    // Anti-vacuity: a guard with no needles, or one that scans none of the files it was
    // written for, passes while checking nothing.
    assert!(
        !needles.is_empty(),
        "derived no helper needles from tests/common/mod.rs — the derivation or the \
         module's shape changed (#120)"
    );
    for expected in BOUND {
        assert!(
            files.iter().any(|f| f == expected),
            "expected {expected} among the bound files — if it was renamed, update BOUND \
             (a guard that scans nothing passes vacuously)"
        );
    }

    let mut offenders: Vec<String> = Vec::new();
    for name in &files {
        let src = fs::read_to_string(dir.join(name)).expect("read test source");
        let dupes = locally_declared(&src, &needles);
        if !dupes.is_empty() {
            offenders.push(format!("{name}: {dupes:?}"));
        }
    }

    assert!(
        offenders.is_empty(),
        "these tests re-declare scaffolding that tests/common/mod.rs already provides \
         (#120) — import it with `mod common;` instead of copying:\n  {}",
        offenders.join("\n  ")
    );
}

#[test]
fn bound_tests_import_the_shared_module() {
    let dir = tests_dir();
    for name in BOUND {
        let src = fs::read_to_string(dir.join(name)).expect("read test source");
        assert!(
            src.lines()
                .any(|l| l.trim_start().starts_with("mod common;")),
            "{name} must pull in the shared scaffolding with `mod common;` (#120)"
        );
    }
}

/// The derivation is pinned so a change to `common/mod.rs`'s shape cannot silently empty
/// it — the failure mode would be a guard that passes because it checks nothing.
#[test]
fn derivation_finds_the_expected_helpers() {
    let mut got = guarded_helpers(&tests_dir());
    got.sort();
    assert_eq!(
        got,
        vec![
            // common/mod.rs now also carries non-identity scaffolding (#288's
            // `medication_setup` and `attestation_count`, for the medication read-path +
            // sign-off test suites; #344 review round 2's `enroll_human`/`submit_attested`,
            // lifted from `identity_repudiate.rs` for the search-funnel suite's
            // suppressing-mode repudiation test). Their presence here does not mean they
            // are identity-specific — only that an identity-bound file (BOUND above) must
            // not declare its own copy of them. See the module header's widened first
            // paragraph. `register_pair` joined them in #345's review: the matching suites
            // (`apply_proposal.rs`, `auto_apply.rs`) had written it identically.
            "async fn attestation_count(",
            "async fn enroll_human(",
            "async fn medication_setup(",
            "async fn person_chart_trust(",
            "async fn register_pair(",
            "async fn submit_attested(",
            "async fn submit_registration(",
            "async fn submit_signed(",
            "async fn submit_signed_with_id(",
            "async fn trust_of(",
        ],
        "the guarded set should be common/mod.rs's public helpers minus REPO_WIDE"
    );
}

/// The matcher itself, pinned against synthetic sources so its verdict cannot regress
/// regardless of what the real files happen to contain (the anti-vacuity lesson from
/// `paper_parity_plan_section.rs`).
#[test]
fn matcher_distinguishes_declarations_from_mentions() {
    let needles = vec!["async fn trust_of(".to_string(), "fn db_msg(".to_string()];
    let found = |src: &str| locally_declared(src, &needles);

    // Declarations — bare, `pub`, `pub(crate)`, and indented (a paste inside a `mod`).
    assert_eq!(
        found("async fn trust_of(c: &Client) {"),
        ["async fn trust_of("]
    );
    assert_eq!(
        found("pub async fn trust_of(c: &Client) {"),
        ["async fn trust_of("]
    );
    assert_eq!(
        found("pub(crate) async fn trust_of(c: &Client) {"),
        ["async fn trust_of("]
    );
    assert_eq!(
        found("    fn db_msg(e: &Error) -> String {"),
        ["fn db_msg("]
    );

    // A doc comment naming a helper is NOT a declaration.
    assert!(found("/// Mirrors `async fn trust_of(` in common.").is_empty());
    // Neither is a call site inside a function body.
    assert!(found("    let t = trust_of(&c, p).await;").is_empty());
    // Nor a similarly-named helper that is not the shared one.
    assert!(found("async fn trust_of_person(c: &Client) {").is_empty());

    // A file importing the module rather than copying it is clean.
    assert!(found("mod common;\nuse common::{db_msg, trust_of};").is_empty());
}
