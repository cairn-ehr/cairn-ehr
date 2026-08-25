//! #452 — one recursive source walk for the source-inspection guards, not three.
//!
//! # Why this is shared, and why the reason is not "DRY"
//!
//! Three guard tests each rolled their own walk over `crates/`, `db/` and `extensions/`
//! looking for a forbidden or required pattern in the text. The copies were close enough to
//! look interchangeable and different enough that a fix landing in one missed the other two:
//!
//! * `no_drugref_dependency.rs` used `path.is_dir()`, which **follows symlinks**. A symlink
//!   pointing at an ancestor makes the walk unbounded — a hang in a required check, which
//!   presents as CI flakiness rather than as a defect.
//! * `event_log_row_by_name.rs` swallowed an unreadable directory (`let Ok(..) else
//!   { return }`) and an unreadable file (`else { continue }`). Either one contributes
//!   nothing and the guard still passes: a silent failure *inside a guard*, which is the
//!   exact species PR #448 existed to remove.
//! * `db_gate_actually_ran.rs` had already fixed both, and was the third copy.
//!
//! So the shared function is not a tidy-up. It is the mechanism by which those two fixes stop
//! being one file's private knowledge. Everything here fails **loudly**: a guard that examines
//! fewer files than it thinks reports the same green as one that examined them all.
//!
//! # Why a leaf module and not `common/mod.rs`
//!
//! `tests/common/mod.rs` is DB scaffolding — it opens with `cairn_event`, `tokio_postgres` and
//! `uuid` imports. A `mod common;` in a pure source-inspection binary would drag that whole
//! dependency surface into a test that needs only `std`. Callers pull this in directly:
//!
//! ```ignore
//! #[path = "common/sources.rs"]
//! mod sources;
//! ```
//!
//! That also keeps the helper out of `identity_scaffolding_shared.rs`'s derivation, which
//! reads `common/mod.rs` specifically and is scoped to the *identity cluster*. A generic file
//! walker registered there would muddy a guard about something else entirely.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

/// Every file under `roots` whose extension is in `exts`, skipping any directory whose own
/// name is in `skip_dirs`.
///
/// Three parameters rather than a fixed policy because the three call sites genuinely differ:
/// different roots (`crates/` alone vs `crates/`+`db/`+`extensions/`), different skip sets
/// (`target` alone vs `target`+`tests`, since `tests/` may legitimately NAME what `src/` may
/// not), and different extensions (`.rs` alone vs `.rs`+`.sql`). Collapsing those into one
/// hardcoded policy would silently widen or narrow a guard's scope.
///
/// `skip_dirs` matches a directory's OWN name, not a path fragment: `"target"` skips every
/// `target/` at any depth, which is what every caller means by it.
///
/// # Symlinks
///
/// Uses [`std::fs::DirEntry::file_type`], which does **not** follow symlinks, rather than
/// `Path::is_dir`, which does. A symlink to an ancestor would otherwise make this walk
/// unbounded. A symlink to a *file* is likewise reported as a symlink, so it is neither
/// descended into nor collected — deliberate: a guard should read the source tree as
/// committed, and no repo file is a symlink today.
///
/// Both halves are pinned by `source_walk_shared.rs`'s
/// `symlinks_are_neither_followed_nor_collected`. They had to be: because the repository holds
/// no symlink, the tree itself supplies no coverage, so this one-word difference from
/// `Path::is_dir` was invisible to every other test until the PR #456 review mutated it.
///
/// # Panics
///
/// On an unreadable directory or entry, naming the path. That is the point: silently
/// contributing nothing is how a guard passes while proving less than it claims.
///
/// Of the four panic sites here, the `read_dir` one is pinned by `a_missing_root_is_loud`. The
/// per-entry two (a failing `DirEntry`, a failing `file_type()`) are not: reaching them needs a
/// directory mutated between the `read_dir` and the iteration, which no portable fixture can
/// stage. They are written in the same loud shape as their tested sibling rather than left to
/// a `continue`; the absence of coverage is stated instead of implied (#456 review).
pub fn source_files(roots: &[PathBuf], skip_dirs: &[&str], exts: &[&str]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack: Vec<PathBuf> = roots.to_vec();

    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("unreadable source directory {}: {e}", dir.display()));
        for entry in entries {
            let entry =
                entry.unwrap_or_else(|e| panic!("unreadable entry under {}: {e}", dir.display()));
            let file_type = entry
                .file_type()
                .unwrap_or_else(|e| panic!("no file type for {}: {e}", entry.path().display()));
            let path = entry.path();

            if file_type.is_dir() {
                let name = entry.file_name();
                if !skip_dirs.iter().any(|s| name == std::ffi::OsStr::new(s)) {
                    stack.push(path);
                }
            } else if file_type.is_file() && has_extension(&path, exts) {
                out.push(path);
            }
        }
    }

    // Sorted so a failure message lists offenders in the same order on every machine and in
    // every run. `read_dir` order is filesystem-defined, so without this a guard that fails
    // on three files reports them in a different order each time — which makes a diff between
    // two CI runs unreadable for no reason.
    out.sort();
    out
}

/// Does `path` end in one of `exts` (each given WITHOUT the dot, e.g. `"rs"`)?
///
/// Pulled out as its own pure function so the extension policy is one readable line rather
/// than a `matches!` arm buried in the walk.
fn has_extension(path: &Path, exts: &[&str]) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| exts.contains(&e))
}

/// Read a source file, panicking loudly and naming the path on failure.
///
/// The counterpart to [`source_files`]'s loudness, and it exists for the same reason: one of
/// the three original walks (`event_log_row_by_name.rs`) skipped an unreadable or non-UTF-8
/// file with a bare `continue`, which takes every pattern whose only evidence lived in that
/// file out of the checked set while the guard still reports success. The other two already
/// panicked here — the module header above has the per-file breakdown, and the first cut of
/// this sentence said "two of three", contradicting it (#456 review).
pub fn read_source(path: &Path) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("unreadable source file {}: {e}", path.display()))
}

/// The repository trees that hold shipping crates, each scanned one level deep for
/// `<crate>/src/`. Not derived from `Cargo.toml`'s `members`, deliberately: that list is the
/// build graph, and its `exclude` entries (`extensions/cairn_pgx`, `cairn-gui`) ship too.
///
/// Adding a tree here widens every source-derived guard at once, which is the point.
const PRODUCTION_TREES: &[&str] = &["crates", "extensions", "cairn-gui"];

/// Every shipped `.rs` surface in the repository: `<tree>/<crate>/src/**/*.rs` under each
/// of [`PRODUCTION_TREES`]. This is what "production code" means to a guard like
/// `unwrap_secret_is_not_derived.rs` (#495/ADR-0066) — the code that ships, as opposed
/// to a `tests/` directory (this one included) or a `benches/` harness, neither of which
/// is ever linked into anything a node runs.
///
/// **`crates/` alone was not "the code that ships", and the gap mattered.** The first
/// version swept only `root/crates/*`, which is the CARGO WORKSPACE — and the two trees
/// `Cargo.toml` deliberately `exclude`s ship anyway: `extensions/cairn_pgx` is the pgrx
/// extension that runs INSIDE Postgres (the unbypassable floor, principle 12) and takes a
/// path dependency on `cairn-event`, and `cairn-gui` is the reference UI. A re-coupling of
/// custody to identity in either would have passed every source guard with the allow-list
/// untouched. Workspace membership is a build-graph fact; "does it ship" is the question the
/// guards actually ask, and they are not the same question.
///
/// Crate directories are discovered by LISTING each tree rather than hardcoded by name, so a
/// new crate is swept in automatically instead of silently sitting outside every
/// source-derived guard until someone remembers to add it. A tree that does not exist is
/// skipped rather than panicking, so a partial checkout does not fail the guards — the
/// anti-vacuity floor in each guard is what catches a sweep that has collapsed.
///
/// `tests`/`benches` join `target` in the skip list defensively: no crate's `src/` tree nests
/// either today (both sit as SIBLINGS of `src/`, never inside it), but a guard whose stated
/// scope is "production" should not start including one silently if that ever changes.
///
/// Uses `DirEntry::file_type()` rather than `Path::is_dir()` for the same reason
/// [`source_files`] does — it does not follow symlinks — even though the one-level scan
/// here (over `crates/` only) cannot itself loop unbounded; the deeper recursive walk is
/// `source_files`'s, which already carries that guarantee.
///
/// Returns an iterator rather than a borrowed slice or a `Vec` a caller must remember to
/// call `.iter()` on: a guard's anti-vacuity check (`…count() > 50`) reads naturally
/// against an iterator, and every path is already owned (built fresh from `root` on each
/// call), so there is no borrow for a `Vec` to usefully preserve here.
pub fn production_rust_files(root: &Path) -> impl Iterator<Item = PathBuf> {
    let mut crate_src_dirs: Vec<PathBuf> = Vec::new();
    for tree in PRODUCTION_TREES {
        let tree_dir = root.join(tree);
        // A tree that is not present is not an error: a shallow or partial checkout should
        // not panic every source-derived guard. Each guard's own anti-vacuity floor is what
        // notices a sweep that has genuinely collapsed.
        let Ok(entries) = std::fs::read_dir(&tree_dir) else {
            continue;
        };
        for entry in entries {
            let entry = entry
                .unwrap_or_else(|e| panic!("unreadable entry under {}: {e}", tree_dir.display()));
            let file_type = entry
                .file_type()
                .unwrap_or_else(|e| panic!("no file type for {}: {e}", entry.path().display()));
            if !file_type.is_dir() {
                continue;
            }
            let src = entry.path().join("src");
            if src.is_dir() {
                crate_src_dirs.push(src);
            }
        }
    }
    crate_src_dirs.sort();

    source_files(&crate_src_dirs, &["target", "tests", "benches"], &["rs"]).into_iter()
}

/// The repository root, derived from this crate's manifest directory.
///
/// `CARGO_MANIFEST_DIR` is `crates/cairn-node`, so the root is two levels up. Canonicalized,
/// because callers use it to `strip_prefix` absolute walk results for display.
pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("crates/cairn-node/../.. is the repo root")
}
