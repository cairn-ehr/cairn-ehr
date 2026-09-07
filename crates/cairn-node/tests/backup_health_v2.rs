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
    describe_health, export_coverage_after, export_outcome_for_write, read_health, write_health,
    BackupHealth, ExportOutcome,
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
        extra: Default::default(),
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
        extra: Default::default(),
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

/// #500 slice 2c final review, **Critical 2** — an export that seals, verifies, and carries
/// NO unwrap key must NOT advance coverage.
///
/// `backup` reaches that state deliberately: `seal_and_write_local_state_export` warns and
/// carries on when `<key>.unwrap` cannot be loaded (absent on a node provisioned before
/// ADR-0066 decision 5, bit-rotted, or sealed under the other operator secret), because the
/// medium is the load-bearing copy and an optional export must never abort a backup. The
/// artifact that lands is durable, readable, well-formed — and opens nothing.
///
/// Recording `Written` for it was the exact false green [`export_coverage_after`] exists to
/// prevent, and `ExportOutcome::Skipped`'s own doc already named "a load failure" as one of
/// the warn-and-continue paths it covers. The call site simply did not honour it: the FILE
/// landing was mistaken for the export ACHIEVING something. `kit_verdict(Some(N), Some(N))`
/// then returned `Restorable` and `verify-backup` exited 0 over a kit whose every sealed
/// body restores as ciphertext, permanently.
#[test]
fn an_export_carrying_no_custody_key_does_not_advance_coverage() {
    assert_eq!(
        export_outcome_for_write(false, 900),
        ExportOutcome::Skipped,
        "no key carried means the export achieved nothing a restore can use"
    );
    // And the ratchet must then leave the previous figure exactly where it was — the two
    // halves are asserted together because either one alone still permits the false green.
    assert_eq!(
        export_coverage_after(Some(500), export_outcome_for_write(false, 900)),
        Some(500),
        "coverage must stay at the last figure an export genuinely achieved"
    );
    assert_eq!(
        export_coverage_after(None, export_outcome_for_write(false, 900)),
        None,
        "and a keyless export must not manufacture a first coverage claim from nothing"
    );
}

/// The other direction, so the test above cannot pass against a function hardwired to
/// `Skipped` — which would silently freeze coverage forever and make every kit read STALE.
#[test]
fn an_export_carrying_the_custody_key_advances_coverage() {
    assert_eq!(
        export_outcome_for_write(true, 900),
        ExportOutcome::Written(900),
        "a key-bearing export is the only thing that may move the figure"
    );
    assert_eq!(
        export_coverage_after(Some(500), export_outcome_for_write(true, 900)),
        Some(900)
    );
}

/// #500 slice 2c final review, I14 — a v1 sidecar must not be RENDERED as "0 node event(s),
/// 0 clinical event(s)".
///
/// `node_events`/`clinical_events` are `#[serde(default)]`, so a v1 sidecar (whose count
/// lived in the differently-named `event_count`) parses with both at zero. Printing those
/// unqualified told an operator that a multi-megabyte medium holds nothing — a
/// self-contradiction manufactured by a serde default, appearing in exactly the window
/// between upgrading the binary and the first successful new `backup`, which is the window in
/// which `backup` is most likely to be failing. `version` was written and never read by
/// anything; this is what reads it.
#[test]
fn a_v1_sidecar_is_not_rendered_as_zero_events() {
    let v1 = BackupHealth {
        version: 1,
        last_backup_unix: 1_700_000_000,
        medium_path: "/m/backup.cairn".into(),
        medium_bytes: 9_400_000,
        node_events: 0,
        clinical_events: 0,
        clinical_watermark: None,
        export_covers_seq: None,
        extra: Default::default(),
    };
    let line = describe_health(1_700_003_600, &Some(v1));
    assert!(
        !line.contains("0 node event(s)"),
        "a v1 sidecar records no per-plane scope; claiming zero is a fact it never stated: \
         {line}"
    );
    assert!(
        line.contains("per-plane") || line.contains("not recorded"),
        "and it must say WHY the counts are missing, so the operator does not read the \
         medium as empty: {line}"
    );
}

/// The anti-vacuity companion: a v2 sidecar still reports its counts. Without this the test
/// above is satisfied by a `describe_health` that never prints counts at all.
#[test]
fn a_v2_sidecar_still_reports_its_per_plane_counts() {
    let v2 = BackupHealth {
        version: 2,
        last_backup_unix: 1_700_000_000,
        medium_path: "/m/backup.cairn".into(),
        medium_bytes: 9_400_000,
        node_events: 12,
        clinical_events: 8_000,
        clinical_watermark: Some(8_000),
        export_covers_seq: Some(8_000),
        extra: Default::default(),
    };
    let line = describe_health(1_700_003_600, &Some(v2));
    assert!(
        line.contains("12 node event(s)") && line.contains("8000 clinical event(s)"),
        "a v2 sidecar's counts are real and must still be shown: {line}"
    );
}

/// A field written by a NEWER build must survive this build's read-modify-write.
///
/// `main.rs`'s export ceremony does exactly that round trip: `read_health`, advance
/// `export_covers_seq`, `write_health`. Plain serde DROPS unknown fields, so an older binary
/// run once against a newer sidecar — a rollback, a rescue USB, a second node sharing the key
/// directory — would silently erase whatever the newer build had recorded there.
///
/// `deny_unknown_fields` is deliberately NOT the fix (it is what `LocalState` uses one layer
/// over): v1's `event_count` is itself an unknown field to this build, so refusing would
/// break the v1 compatibility the test at the top of this file pins. Preserving is both
/// backward AND forward compatible; refusing is only one of those.
#[test]
fn a_field_from_a_newer_build_survives_a_read_modify_write() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("backup-status.json");
    std::fs::write(
        &path,
        r#"{"version":3,"last_backup_unix":1700000000,"medium_path":"/m/backup.cairn",
            "medium_bytes":4096,"node_events":12,"clinical_events":900,
            "clinical_watermark":900,"export_covers_seq":880,
            "clinical_restorable_through":870}"#,
    )
    .unwrap();

    let mut health = read_health(&path).expect("a newer sidecar must still parse");
    health.export_covers_seq = export_coverage_after(
        health.export_covers_seq,
        export_outcome_for_write(true, 900),
    );
    write_health(&path, &health).unwrap();

    let back = std::fs::read_to_string(&path).unwrap();
    assert!(
        back.contains("clinical_restorable_through"),
        "the newer build's field must still be in the file after this build rewrote it: \
         {back}"
    );
    assert!(
        back.contains("870"),
        "and with its value intact, not merely its name: {back}"
    );
}
