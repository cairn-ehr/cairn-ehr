//! #500 slice 2c Task 10 — `BackupHealth` v2 reports per-plane scope, not one composite
//! `event_count`. DB-free: everything here is a pure function or a sidecar read/write
//! against a tempdir, so it needs no `CAIRN_TEST_PG` and runs in every `cargo test`.
//!
//! WHY THIS FILE EXISTS AS ITS OWN TARGET (not folded into `backup.rs`'s `#[cfg(test)]
//! mod tests`): the golden pin below is the property this task is actually FOR — a round
//! trip through one encoder/decoder pair cannot catch a mirrored rename (2a's 19-of-19
//! lesson, repeated in this slice's design doc §9 test 7). Renaming `clinical_watermark`
//! on both the struct and every call site stays green under `PartialEq`; it fails only
//! against a literal string.

use cairn_node::backup::{
    export_coverage_after, read_health, write_health, BackupHealth, ExportOutcome,
};

/// A v1 sidecar written by yesterday's binary must still READ. An operator upgrading must
/// not lose their health record — and "health may only ever under-claim" means the missing
/// fields become None/0, never a parse failure that reads as "no backup ever ran".
#[test]
fn a_v1_sidecar_still_reads_with_the_new_fields_absent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("backup-status.json");
    std::fs::write(
        &path,
        r#"{"version":1,"last_backup_unix":1700000000,"medium_path":"/m/backup.cairn",
            "event_count":42,"medium_bytes":4096}"#,
    )
    .unwrap();

    let health = read_health(&path).expect("a v1 sidecar must still parse");
    assert_eq!(health.last_backup_unix, 1_700_000_000);
    // `node_events` is ALSO new in v2 (v1's field was the differently-named `event_count`,
    // left un-migrated on purpose — see the struct doc), so it defaults exactly like the
    // other three new fields. Asserted explicitly: a literal-JSON test that checks three of
    // four new-field defaults and assumes the fourth is fine on faith is not actually a test
    // of the fourth.
    assert_eq!(
        health.node_events, 0,
        "absent means zero known, never a parse failure"
    );
    assert_eq!(
        health.clinical_events, 0,
        "absent means zero known, never a parse failure"
    );
    assert_eq!(
        health.export_covers_seq, None,
        "a v1 sidecar knows nothing about export coverage, and None is that honest answer — \
         0 would claim the export covers seq 0, which is a claim it never made"
    );
}

#[test]
fn a_v2_sidecar_round_trips_every_field() {
    let health = BackupHealth {
        version: 2,
        last_backup_unix: 1_700_000_000,
        medium_path: "/m/backup.cairn".into(),
        medium_bytes: 8192,
        node_events: 7,
        clinical_events: 41_204,
        clinical_watermark: Some(91_338),
        export_covers_seq: Some(91_338),
    };
    // Bound to `dir`, not chained: `tempdir().unwrap().path().join(..)` drops the `TempDir`
    // guard at the end of the statement, which deletes the directory from disk before
    // `write_health` ever runs — a bug in the brief's literal Step-1 code, caught by running
    // this test rather than trusting the snippet (`RUST_BACKTRACE` showed a plain `NotFound`).
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("backup-status.json");
    write_health(&path, &health).unwrap();
    assert_eq!(read_health(&path).unwrap(), health);
}

/// The rule that keeps a stale kit detectable: a skipped export must NOT advance the field.
/// If it did, `verify-backup` would compare the medium against a coverage figure no export
/// ever achieved — the precise-untruth composite this whole programme exists to end.
#[test]
fn a_skipped_export_leaves_the_previous_coverage_untouched() {
    let previous = Some(500_i64);
    assert_eq!(
        export_coverage_after(previous, ExportOutcome::Skipped),
        previous
    );
    assert_eq!(
        export_coverage_after(previous, ExportOutcome::Written(900)),
        Some(900)
    );
}

/// A node with no coverage history yet (fresh sidecar, or a v1 upgrade) stays `None` on a
/// skip — there is nothing to preserve, and a skip must not manufacture a claim from thin air.
#[test]
fn a_skipped_export_with_no_prior_coverage_stays_none() {
    assert_eq!(export_coverage_after(None, ExportOutcome::Skipped), None);
}

/// Golden pin (design doc §9 test 7 / house rule 6's sibling for renames rather than crypto
/// values): the EXACT serialized field names, so a mirrored rename of `clinical_watermark`
/// (or any other field) on both the writer and this assertion together is the only way to
/// keep this test green — and a rename on just one side, which is the actual failure mode
/// slice 2a hit 19 times in a row, fails HERE even though `PartialEq` round-trips fine.
#[test]
fn the_v2_shape_is_pinned_field_by_field() {
    let health = BackupHealth {
        version: 2,
        last_backup_unix: 1_700_000_000,
        medium_path: "/m/backup.cairn".into(),
        medium_bytes: 8192,
        node_events: 7,
        clinical_events: 41_204,
        clinical_watermark: Some(91_338),
        export_covers_seq: Some(91_338),
    };
    let json = serde_json::to_string_pretty(&health).unwrap();
    assert_eq!(
        json, PINNED_V2_JSON,
        "a field rename must fail HERE, not just round-trip through PartialEq"
    );
}

const PINNED_V2_JSON: &str = r#"{
  "version": 2,
  "last_backup_unix": 1700000000,
  "medium_path": "/m/backup.cairn",
  "medium_bytes": 8192,
  "node_events": 7,
  "clinical_events": 41204,
  "clinical_watermark": 91338,
  "export_covers_seq": 91338
}"#;
