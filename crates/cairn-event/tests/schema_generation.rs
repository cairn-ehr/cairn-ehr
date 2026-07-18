//! Issue #188 — the guard that keeps `SCHEMA_GENERATION` honest.
//!
//! `cairn_event::schema_generation::SCHEMA_GENERATION` is one hand-written line, and the
//! whole #188 downgrade guard is built on it being right. This is a SOURCE-LEVEL guard
//! (no DB needed, like `twin_dispatch_single_source.rs`): it reads `db/` and fails if the
//! constant is not the newest migration's prefix. Adding `db/039_*.sql` without bumping
//! the constant is therefore a CI failure rather than a silent drift — the same
//! register-and-guard discipline ADR-0048 uses for the twin-check registry.
use std::fs;
use std::path::PathBuf;

use cairn_event::schema_generation::{newest_migration_prefix, SCHEMA_GENERATION};

/// Repo-root `db/` directory. `CARGO_MANIFEST_DIR` is `crates/cairn-event`; `db/` is two
/// levels up.
fn db_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../db")
        .canonicalize()
        .expect("db/ dir")
}

/// Every `db/NNN_*.sql` file name in the repo, in no particular order.
fn migration_names() -> Vec<String> {
    fs::read_dir(db_dir())
        .expect("read db/")
        .filter_map(|entry| {
            let path = entry.expect("entry").path();
            if path.extension().and_then(|e| e.to_str()) != Some("sql") {
                return None;
            }
            path.file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_owned())
        })
        .collect()
}

#[test]
fn schema_generation_is_the_newest_migration_in_db() {
    let names = migration_names();
    let newest = newest_migration_prefix(names.iter().map(String::as_str))
        .expect("db/ contains numbered migrations");
    assert_eq!(
        SCHEMA_GENERATION, newest,
        "SCHEMA_GENERATION ({SCHEMA_GENERATION}) is stale: db/ now goes up to {newest}. \
         Bump the constant in crates/cairn-event/src/schema_generation.rs in the same \
         commit that adds the migration — the #188 downgrade guard reads it, and both \
         loaders must report the same generation for one git build."
    );
}
