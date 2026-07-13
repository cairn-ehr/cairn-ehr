//! #173 — the cairn_event_twin dispatcher must be declared in EXACTLY ONE migration
//! (db/005). The prior copy-hazard was that each slice re-declared the whole IF/ELSIF
//! chain, so a stale copy could silently drop a floor check. This is a SOURCE-LEVEL guard
//! (no DB needed): it scans db/*.sql and fails if more than one file declares the function,
//! catching any re-introduction of the copy pattern in every `cargo test` / CI run.
use std::fs;
use std::path::PathBuf;

/// Repo-root db/ directory. CARGO_MANIFEST_DIR is crates/cairn-node; db/ is two levels up.
fn db_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../db")
        .canonicalize()
        .expect("db/ dir")
}

#[test]
fn cairn_event_twin_is_declared_in_exactly_one_migration() {
    let needle = "CREATE OR REPLACE FUNCTION cairn_event_twin(";
    let mut declaring: Vec<String> = Vec::new();
    for entry in fs::read_dir(db_dir()).expect("read db/") {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("sql") {
            continue;
        }
        let sql = fs::read_to_string(&path).expect("read sql");
        if sql.contains(needle) {
            declaring.push(path.file_name().unwrap().to_string_lossy().into_owned());
        }
    }
    declaring.sort();
    assert_eq!(
        declaring,
        vec!["005_submit.sql".to_string()],
        "cairn_event_twin must be declared ONLY in db/005 (#173); found in: {declaring:?}"
    );
}
