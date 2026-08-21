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
/// # Panics
///
/// On an unreadable directory or entry, naming the path. That is the point: silently
/// contributing nothing is how a guard passes while proving less than it claims.
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
/// The counterpart to [`source_files`]'s loudness, and it exists for the same reason: two of
/// the three original walks skipped an unreadable or non-UTF-8 file with a bare `continue`,
/// which takes every pattern whose only evidence lived in that file out of the checked set
/// while the guard still reports success.
pub fn read_source(path: &Path) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("unreadable source file {}: {e}", path.display()))
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
