//! ADR-0059 decision 4 — honest degradation, proven by construction.
//!
//! A node without drugref must still read, sync, list and reconcile a CODED medication.
//! The strongest possible proof of that is structural: no drugref code exists in this
//! tree at all, so drugref-absent is the ONLY configuration every other test runs under.
//! A mocked absence could drift; this cannot.
//!
//! When a later slice adds the §9 advisory-tier drugref lookup, this guard must be
//! narrowed deliberately (to the trusted surface — db/ and the floor path), never simply
//! deleted: the load-bearing invariant is that the FLOOR and the PROJECTIONS never depend
//! on a drug database, not that no client code exists anywhere.
use std::fs;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

/// Every `.sql` under db/ and every `.rs` under crates/*/src — the trusted surface.
fn trusted_sources() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![repo_root().join("db"), repo_root().join("crates")];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("read dir") {
            let p = entry.expect("dir entry").path();
            if p.is_dir() {
                // tests/ may legitimately NAME drugref in prose; src/ and db/ may not.
                if p.file_name().is_some_and(|n| n == "target" || n == "tests") {
                    continue;
                }
                stack.push(p);
            } else if matches!(
                p.extension().and_then(|e| e.to_str()),
                Some("sql") | Some("rs")
            ) {
                out.push(p);
            }
        }
    }
    out
}

/// A mention inside a comment is fine (the ADR is cited all over db/041); an executable
/// reference is not. Crude but sufficient: flag a drugref mention on a line that is not a
/// comment.
fn offending_lines(path: &Path) -> Vec<String> {
    let text = fs::read_to_string(path).expect("read source");
    text.lines()
        .filter(|l| l.to_lowercase().contains("drugref"))
        .filter(|l| {
            let t = l.trim_start();
            !(t.starts_with("--")
                || t.starts_with("//")
                || t.starts_with("*")
                || t.starts_with("#"))
        })
        // A seeded registry row and the system token itself are DATA, not a dependency.
        .filter(|l| {
            !l.contains("'drugref-moiety'")
                && !l.contains("drugref-clinical-drug")
                && !l.contains("drugref-product")
                && !l.contains("\"drugref-moiety\"")
        })
        // Human-readable prose that names drugref inside a diagnostic message is not an
        // executable reference either — it explains *why* a rule exists, same as a comment
        // would, but the rule lives in a RAISE EXCEPTION / assert! message string so the
        // leading-comment-marker check above can't catch it. Each exclusion below is the
        // exact phrase from one specific message, not a blanket "any prose" allowance, so a
        // real dependency (e.g. a call or a URL) still trips the guard.
        .filter(|l| {
            // db/041_medication_coding.sql: explains the uuid-format constraint by noting
            // drugref moiety ids happen to be UUIDv5 — does not call or query drugref.
            !l.contains("a drugref moiety id is a UUIDv5")
            // crates/cairn-event/src/medication/assert.rs: names the honest-degradation
            // reader ("drugref-less") in a test's assert! failure message.
            && !l.contains("drugref-less")
        })
        .map(|l| l.trim().to_string())
        .collect()
}

#[test]
fn the_trusted_surface_never_calls_drugref() {
    let mut offenders: Vec<String> = Vec::new();
    for path in trusted_sources() {
        for line in offending_lines(&path) {
            offenders.push(format!("{}: {line}", path.display()));
        }
    }
    assert!(
        offenders.is_empty(),
        "the in-DB floor and the projections must never depend on a drug database \
         (ADR-0059 decision 4 — a coded medication reads, syncs and reconciles without \
         drugref). Offenders:\n{}",
        offenders.join("\n")
    );
}
