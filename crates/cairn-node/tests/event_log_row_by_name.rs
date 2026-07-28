//! Source-level guard (#296): no test may synthesize an `event_log` row POSITIONALLY.
//!
//! WHAT WENT WRONG. `born_sealed_schema.rs` used to build a synthetic row as a positional
//! `ROW(a, b, c, …)::event_log` literal, with the element order transcribed by hand from
//! `\d event_log`. Positional composite literals bind by physical attribute order, so that
//! test was hostage to the attribute order of a SHARED test database — and one
//! `cairn-sync` test dropped `event_log.seq` and let the migration re-add it, which
//! `ADD COLUMN IF NOT EXISTS` does at the END of the attribute list. From then on `seq`
//! sat after db/040's `clock_grade`, the literal's 23rd element (a `clock_grade` string)
//! landed in `seq bigint`, and the run failed with
//! `invalid input syntax for type bigint: "unknown"` — in a different crate, far from the
//! cause, and only on the SECOND run against the same database. That is the whole origin of
//! the long-carried "recreate the test databases" gotcha.
//!
//! WHY A GUARD AND NOT JUST THE FIX. Both halves are fixed (the construction is now
//! `jsonb_populate_record`, which binds by NAME; the polluting test now RENAMES the column
//! instead of dropping it, so position survives). But the trap is re-armed by a single
//! future `ROW(...)::event_log`, and it fails far from where it is introduced. This test
//! costs nothing, needs no database, and fails in the file that reintroduces it.
//!
//! Scope: `event_log` specifically — the widest composite in the tree, the one that keeps
//! gaining columns, and the only one this has actually bitten. Only EXECUTABLE source is
//! scanned (`crates/`, `db/`): `docs/` holds superseded plan records whose whole value is
//! being an unedited account of what was done at the time, including the mistakes.
use std::fs;
use std::path::Path;

/// Walk `crates/` and `db/`, returning every `.rs`/`.sql` path (skipping `target/`).
fn source_files(root: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            if p.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            source_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs" || x == "sql") {
            out.push(p);
        }
    }
}

/// True when this line is a comment (Rust `//`, `//!`, SQL `--`) rather than executable
/// text. The fix's own explanatory comments name the banned pattern verbatim — that is the
/// documentation, and it must not trip its own guard.
fn is_commentary(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("//") || t.starts_with("--")
}

#[test]
fn no_positional_event_log_row_construction() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/cairn-node -> repo root")
        .to_path_buf();
    let mut files = Vec::new();
    source_files(&repo.join("crates"), &mut files);
    source_files(&repo.join("db"), &mut files);
    assert!(!files.is_empty(), "found no source files to scan");

    // This file names the banned pattern in a string literal (the matcher) and in the
    // failure message, so it must exempt itself — the same self-exemption db/041's
    // drugref source guard needs, and for the same reason: a guard that describes what it
    // forbids will always match itself.
    let self_path = Path::new(file!())
        .file_name()
        .expect("this file has a name");
    let mut offenders = Vec::new();
    for f in &files {
        if f.file_name() == Some(self_path) {
            continue;
        }
        let Ok(text) = fs::read_to_string(f) else {
            continue;
        };
        // A positional construction always ends `)::event_log`, and always starts with a
        // `ROW(` or a bare `(` argument list. Matching the CAST is what makes this precise:
        // `jsonb_populate_record(NULL::event_log, …)` names the type as an ARGUMENT, never
        // as a trailing cast of a value list, so the by-name form cannot false-positive.
        for (i, line) in text.lines().enumerate() {
            if is_commentary(line) {
                continue;
            }
            if line.contains(")::event_log") {
                offenders.push(format!(
                    "{}:{}: {}",
                    f.strip_prefix(&repo).unwrap_or(f).display(),
                    i + 1,
                    line.trim()
                ));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "positional `…)::event_log` construction found — bind by COLUMN NAME instead \
         (jsonb_populate_record(NULL::event_log, jsonb_build_object(…))), because a \
         positional composite literal silently binds the wrong value into the wrong column \
         as soon as event_log's physical attribute order shifts (#296):\n{}",
        offenders.join("\n")
    );
}
