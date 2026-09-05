//! Every place that decides whether a shredded body's key may TRAVEL, named.
//!
//! Not style: this is the wire-level half of the crypto-shred guarantee, and before db/051
//! it had two spellings in two crates. A third would be silent — every caller keeps
//! working, and only the one that drifts stops filtering.
//!
//! NAME, NEVER COUNT (the house rule a count cannot satisfy: it cannot separate "one site
//! moved" from "one site added and one deleted"). The allow-list below is the inventory of
//! every legitimate mention, each with the reason it is not a second definition of the
//! travel predicate. When this fails: if you added a CALLER, select from
//! `event_custody_surviving` / `cairn_clinical_page` instead. If you added a genuinely new
//! decision site, add it here WITH its reason, in the same commit.

#[path = "common/sources.rs"]
mod sources;

use std::path::Path;

/// (repo-relative file, why this mention is not a rival definition).
///
/// LEARNED, not guessed: this is the file list `cargo test -p cairn-node --test
/// shred_predicate_has_one_home` actually printed under `Found:` (minus the two call
/// sites Task 3 moved onto db/051 — `localstate_read.rs` and `cairn-sync/src/main.rs` —
/// which drop out of the scan once they stop spelling the predicate themselves). Order
/// matters: `found` is sorted, `ALLOWED` is compared unsorted, so this list is kept in
/// the same alphabetical order the scan produces.
const ALLOWED: &[(&str, &str)] = &[
    (
        "crates/cairn-node/tests/born_sealed_schema.rs",
        "schema smoke test: asserts the table exists and that cairn_agent has no direct \
         DML on it — a privilege check, not a decision about any row's travel",
    ),
    (
        "crates/cairn-node/tests/common/mod.rs",
        "shared fixture cleanup: TRUNCATEs the table between tests so one run cannot \
         leak state into the next — decides nothing about any single row",
    ),
    (
        "crates/cairn-node/tests/dr_clinical_guarantee_gap.rs",
        "the ADR-0066/#500 guarantee-gap suite: asserts the export's behaviour (a \
         shredded body's DEK never reaches LocalState) and stages the one state that can \
         fire the filter for defense-in-depth coverage — a consumer of the guarantee, \
         never a second definition of it",
    ),
    (
        "crates/cairn-node/tests/localstate.rs",
        "fixture cleanup: TRUNCATEs the table between tests, same shape as common/mod.rs",
    ),
    (
        "crates/cairn-node/tests/medication.rs",
        "fixture cleanup: TRUNCATEs the table between tests, same shape as common/mod.rs",
    ),
    (
        "crates/cairn-node/tests/medication_attestation.rs",
        "fixture cleanup: TRUNCATEs the table between tests, same shape as common/mod.rs",
    ),
    (
        "crates/cairn-node/tests/medication_authorship.rs",
        "fixture cleanup: TRUNCATEs the table between tests, same shape as common/mod.rs",
    ),
    (
        "crates/cairn-node/tests/medication_coding.rs",
        "fixture cleanup: TRUNCATEs the table between tests, same shape as common/mod.rs",
    ),
    (
        "crates/cairn-node/tests/medication_coding_overlay.rs",
        "fixture cleanup: TRUNCATEs the table between tests, same shape as common/mod.rs",
    ),
    (
        "crates/cairn-node/tests/medication_dose.rs",
        "fixture cleanup: TRUNCATEs the table between tests, same shape as common/mod.rs",
    ),
    (
        "crates/cairn-node/tests/medication_patient_consistency.rs",
        "fixture cleanup: TRUNCATEs the table between tests, same shape as common/mod.rs",
    ),
    (
        "crates/cairn-node/tests/medication_reconciliation.rs",
        "fixture cleanup: TRUNCATEs the table between tests, same shape as common/mod.rs",
    ),
    (
        "crates/cairn-node/tests/restore_inherits_custody.rs",
        "fixture cleanup: TRUNCATEs the table between tests, same shape as common/mod.rs",
    ),
    (
        "crates/cairn-node/tests/seal_apply.rs",
        "the shred-execution test: asserts cairn_execute_shred's OWN writes to this table \
         (the INSERT recording a shred, its idempotent re-run, the rebuild-from-log path) \
         — verifies the executor's bookkeeping, never whether a key travels",
    ),
    (
        "crates/cairn-node/tests/seal_submit.rs",
        "fixture cleanup: TRUNCATEs the table between tests, same shape as common/mod.rs",
    ),
    (
        "crates/cairn-node/tests/shred_cli.rs",
        "the shred CLI's own test: asserts the CLI wrote the expected row and that \
         re-running it is idempotent — verifies the CLI's write, never whether a key \
         travels",
    ),
    (
        "crates/cairn-node/tests/shred_predicate_has_one_home.rs",
        "this guard's own source: the table name appears in its matching logic and its \
         failure message, never as a decision about any row",
    ),
    (
        "crates/cairn-sync/tests/clinical_pull.rs",
        "cross-crate custody tests: TRUNCATEs the table for fixture reset, asserts serve \
         behaviour by querying it, and one test manually INSERTs a row to stage the single \
         state that isolates the serve-side filter (see that test's own doc) — all verify \
         behaviour, none redefine the filter",
    ),
    (
        "db/005_submit.sql",
        "the LOCAL submit door's own anti-resurrection check (same shape as db/020's \
         remote one): NOT EXISTS(erasure_shred_log) decides whether to CREATE a custody \
         row for an already-shredded target on first write, not whether an existing key \
         travels",
    ),
    (
        "db/020_apply_remote_event.sql",
        "the remote apply door's anti-resurrection check: NOT EXISTS(erasure_shred_log) \
         decides whether to CREATE a custody row for an already-shredded target, not \
         whether an existing key travels",
    ),
    (
        "db/037_born_sealed.sql",
        "the shred executor and its schema: CREATE TABLE, the INSERT that records a shred \
         (cairn_execute_shred and the idempotent rebuild from the append-only log), and \
         the REVOKE/GRANT access floor — decide whether a shred is recorded and who may \
         read the table, never whether an existing key travels",
    ),
    (
        "db/051_clinical_capture_source.sql",
        "THE definition: event_custody_surviving is the one filter every caller inherits",
    ),
    (
        "db/tests/051_clinical_capture_source_test.sql",
        "the SQL-layer test for db/051 itself: stages a shredded row to prove the view \
         and function exclude it — verifies the definition, is not a second one",
    ),
];

#[test]
fn the_travel_filter_has_one_definition_and_every_other_mention_is_named() {
    let root = sources::repo_root();
    let roots = vec![root.join("db"), root.join("crates")];
    let mut found: Vec<String> = Vec::new();
    for path in sources::source_files(&roots, &["target"], &["sql", "rs"]) {
        let text = sources::read_source(&path);
        let mentions = text.lines().any(|line| {
            let l = line.trim();
            !l.starts_with("--") && !l.starts_with("//") && l.contains("erasure_shred_log")
        });
        if mentions {
            let rel = path.strip_prefix(&root).unwrap_or(Path::new("")).display();
            found.push(rel.to_string());
        }
    }
    found.sort();
    found.dedup();
    let allowed: Vec<String> = ALLOWED.iter().map(|(f, _)| (*f).to_string()).collect();
    assert_eq!(
        found, allowed,
        "the inventory of files deciding anything about erasure_shred_log has changed.\n\
         Found: {found:#?}\nAllowed: {allowed:#?}"
    );
}
