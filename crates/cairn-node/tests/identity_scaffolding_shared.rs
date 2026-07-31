//! #120 — the identity integration tests must share ONE copy of their scaffolding.
//!
//! `identity_identify.rs` landed the *third* near-verbatim copy of `db_msg` / `setup` /
//! `submit_patient_created` / `trust_of` / `person_chart_trust`, copied from
//! `identity_dispute.rs`, itself copied from `identity_linkage.rs` — and the copies had
//! already drifted (the third file's `submit_dispute` had silently dropped the
//! reason/resolution parameter its original carries). Copying is the path of least
//! resistance, so removing the copies without a guard just resets the clock until copy #4.
//!
//! This is a SOURCE-LEVEL guard (no DB needed), the same idiom as
//! `twin_dispatch_single_source.rs` (#173) and `paper_parity_plan_section.rs` (#217): it
//! scans `tests/identity_*.rs` and fails if a bound file declares a helper that
//! `tests/common/mod.rs` already provides. It runs in every `cargo test` / CI pass.
//!
//! FORWARD-ONLY, and deliberately opt-OUT rather than opt-in: every `identity_*.rs` is
//! bound unless it is named in `EXEMPT`, so a NEW identity test file is caught the moment
//! it copies scaffolding instead of importing it. The exemptions are the two files whose
//! `setup()` genuinely differs in shape (see `EXEMPT`), not a general escape hatch.

use std::fs;
use std::path::{Path, PathBuf};

/// Helper declarations that now live in `tests/common/mod.rs`. A bound file re-declaring
/// any of these is starting copy N+1. Matched against the START of each trimmed source
/// line (a leading `pub ` is stripped first), so a doc comment or a *call* mentioning the
/// name is not a false positive — only a top-level declaration is.
const SHARED_HELPERS: [&str; 7] = [
    "fn cs()",
    "fn db_msg(",
    "async fn setup(",
    "async fn submit_signed(",
    "async fn submit_patient_created(",
    "async fn trust_of(",
    "async fn person_chart_trust(",
];

/// Identity test files NOT bound by this guard, each for a stated structural reason.
/// Being on this list is not permission to copy — it records that the shared `setup()`
/// does not fit the file's needs today:
///
/// - `identity_repudiate.rs` — its `setup()` returns TWO enrolled signers (a repudiation
///   needs a second actor), so it cannot call the single-signer shared `setup()`.
/// - `identity_evidence_text.rs` — its `setup()` neither truncates the identity overlay
///   tables nor returns the same tuple shape.
///
/// Converting either one means deleting its entry here, not widening the list.
const EXEMPT: [&str; 2] = ["identity_repudiate.rs", "identity_evidence_text.rs"];

/// Files the conversion covered. Pinned by name so a rename or an accidental deletion
/// makes the guard fail LOUDLY rather than silently scanning an empty set and passing —
/// the anti-vacuity check `paper_parity_plan_section.rs` calls out.
const BOUND: [&str; 3] = [
    "identity_dispute.rs",
    "identity_identify.rs",
    "identity_linkage.rs",
];

/// This crate's `tests/` directory. Same `CARGO_MANIFEST_DIR` idiom as
/// `twin_dispatch_single_source.rs`'s `db_dir()`.
fn tests_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .canonicalize()
        .expect("crates/cairn-node/tests/ dir")
}

/// Every `identity_*.rs` directly in `tests/`, sorted. Directories (`tests/common/`) and
/// non-Rust files are skipped by the extension check.
fn identity_test_files(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .expect("read tests/")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("rs"))
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .filter(|n| n.starts_with("identity_"))
        .collect();
    names.sort();
    names
}

/// The shared helpers a source file declares itself.
///
/// Line-start matching (after trimming indentation and any `pub `) is what keeps this
/// honest: these helpers are all top-level items at column 0, while doc comments start
/// `///` and call sites are indented inside a function body, so neither can trip it.
fn locally_declared(src: &str) -> Vec<&'static str> {
    SHARED_HELPERS
        .iter()
        .copied()
        .filter(|needle| {
            src.lines().any(|line| {
                let line = line.trim_start();
                let line = line.strip_prefix("pub ").unwrap_or(line);
                line.starts_with(needle)
            })
        })
        .collect()
}

#[test]
fn identity_tests_do_not_redeclare_shared_scaffolding() {
    let dir = tests_dir();
    let files = identity_test_files(&dir);

    // Anti-vacuity: the scan must actually see the files the conversion covered.
    for expected in BOUND {
        assert!(
            files.iter().any(|f| f == expected),
            "expected {expected} in tests/ — if it was renamed, update BOUND (a guard that \
             scans nothing passes vacuously)"
        );
    }

    let mut offenders: Vec<String> = Vec::new();
    for name in files.iter().filter(|f| !EXEMPT.contains(&f.as_str())) {
        let src = fs::read_to_string(dir.join(name)).expect("read test source");
        let dupes = locally_declared(&src);
        if !dupes.is_empty() {
            offenders.push(format!("{name}: {dupes:?}"));
        }
    }

    assert!(
        offenders.is_empty(),
        "these identity tests re-declare scaffolding that tests/common/mod.rs already \
         provides (#120) — import it with `mod common;` instead of copying:\n  {}",
        offenders.join("\n  ")
    );
}

#[test]
fn bound_identity_tests_import_the_shared_module() {
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

#[test]
fn exemptions_name_real_files() {
    let dir = tests_dir();
    for name in EXEMPT {
        assert!(
            dir.join(name).is_file(),
            "EXEMPT names {name}, which does not exist — a stale exemption is a silent hole \
             in the guard; delete the entry (#120)"
        );
    }
}

/// The matcher itself, pinned against synthetic sources so its verdict cannot regress
/// regardless of what the real files happen to contain (the anti-vacuity lesson from
/// `paper_parity_plan_section.rs`).
#[test]
fn matcher_distinguishes_declarations_from_mentions() {
    // A declaration — with and without `pub`, at column 0 and indented.
    assert_eq!(locally_declared("fn cs() -> Option<String> {"), ["fn cs()"]);
    assert_eq!(
        locally_declared("pub async fn trust_of(c: &Client) {"),
        ["async fn trust_of("]
    );

    // A doc comment naming a helper is NOT a declaration.
    assert!(locally_declared("/// Mirrors `fn cs()` in common.").is_empty());
    // Neither is a call site inside a function body.
    assert!(locally_declared("    let t = trust_of(&c, p).await;").is_empty());
    // Nor a similarly-named helper that is not the shared one.
    assert!(locally_declared("async fn setup_node(c: &Client) {").is_empty());

    // A file importing the module rather than copying it is clean.
    assert!(locally_declared("mod common;\nuse common::{cs, setup, trust_of};").is_empty());
}
