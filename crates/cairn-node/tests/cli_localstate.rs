//! ADR-0026 slice D — CLI-level regression tests for the two slice-D review fixes, spawning
//! the real `cairn-node` binary (via `CARGO_BIN_EXE_cairn-node`) so the `main.rs` orchestration
//! — which the library-level tests cannot reach — is exercised end to end. No extra test
//! dependency: the binary path is the one Cargo already builds.
//!
//!   1. `seal-key` re-establishes the local-state escrow under its FRESH secrets instead of
//!      bailing on a pre-existing `.lsk` (offline — `seal-key` touches no DB).
//!   2. `backup` DEGRADES (warn + skip, exit 0) when the local-state export cannot be sealed,
//!      rather than aborting an already-written event backup (DB-gated — `backup` reads events).

use cairn_node::fsio::atomic_write;
use cairn_node::keystore::{self, key_at_rest_state, KeyAtRest};
use cairn_node::localstate::{establish_lsk, lsk_sidecar_path_for, serialize_sidecar};
use std::process::Command;

/// A `Command` for the freshly-built `cairn-node` binary under test.
fn cairn_node() -> Command {
    Command::new(env!("CARGO_BIN_EXE_cairn-node"))
}

/// Write a `.lsk` sidecar beside `key` under the given secrets (stands in for a node that
/// already ran `establish-local-state-key` before the path under test).
fn write_existing_escrow(key: &std::path::Path, op: &str, code: &str) -> Vec<u8> {
    let wraps = establish_lsk(op, code).unwrap();
    let bytes = serialize_sidecar(&wraps);
    atomic_write(&lsk_sidecar_path_for(key), &bytes, Some(0o600)).unwrap();
    bytes
}

/// Finding 2 (offline): `seal-key` mints a FRESH recovery code, so it must re-wrap the LSK
/// under it. Before the fix, an existing `.lsk` (e.g. from an earlier `establish-local-state-key`
/// on the still-plaintext key) made `seal-key` BAIL *after* resealing the key — leaving the LSK
/// desynced under the old code and the command erroring. It must now succeed and replace the
/// sidecar. `seal-key` connects to no DB, so this runs everywhere (a dummy `--conn` satisfies clap).
#[test]
fn seal_key_re_establishes_a_pre_existing_escrow_instead_of_bailing() {
    let dir = tempfile::tempdir().unwrap();
    let key = dir.path().join("node.key");

    // A plaintext key that already carries a `.lsk` under OLD secrets.
    keystore::generate_plaintext(&key).unwrap();
    let old_sidecar = write_existing_escrow(&key, "old-op", "OLD-CODE");

    let out = cairn_node()
        .args(["--conn", "postgresql://unused", "--key"])
        .arg(&key)
        .args(["seal-key", "--passphrase", "new-op"])
        .output()
        .unwrap();

    // The regression: before the fix this exited non-zero (bailed on the existing sidecar)
    // even though the signing key had already been resealed.
    assert!(
        out.status.success(),
        "seal-key must succeed with a pre-existing escrow; stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // The signing key is now sealed (seal-key actually completed its primary job).
    assert!(
        matches!(key_at_rest_state(&key), KeyAtRest::Sealed { .. }),
        "seal-key must leave the key sealed"
    );
    // The escrow was re-established under the new secrets (sidecar bytes changed), so the LSK
    // travels with the just-resealed key rather than staying under the old recovery code.
    let new_sidecar = std::fs::read(lsk_sidecar_path_for(&key)).unwrap();
    assert_ne!(
        old_sidecar, new_sidecar,
        "the `.lsk` must be re-wrapped under seal-key's fresh secrets"
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("re-established"),
        "seal-key must report it replaced the stale sidecar"
    );
}

/// Finding 1 (DB-gated): on a node with a `.lsk`, a `backup` whose passphrase cannot unwrap the
/// LSK (wrong/typo'd passphrase, or none in an unattended run) must still SUCCEED — the event
/// medium is the load-bearing copy and is already written; the optional export is warn+skipped.
/// Before the fix the `?` aborted backup with a non-zero exit *after* the medium was written.
/// Needs CAIRN_TEST_PG (a DB with `cairn_pgx` + the node schema); skips otherwise.
#[tokio::test]
async fn backup_degrades_when_the_export_cannot_be_sealed() {
    let Some(base) = std::env::var("CAIRN_TEST_PG").ok() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    use cairn_node::{db, identity};
    let _guard = db::test_serial_guard(&base).await.unwrap();

    let dir = tempfile::tempdir().unwrap();
    let key = dir.path().join("node.key");

    // Provision a real node (so `node_event` holds the genesis the binary will back up), then
    // give it a `.lsk` whose op-pass is "right-op".
    let a = db::connect_and_load_schema(&base).await.unwrap();
    db::reset_node_federation_tables(&a).await.ok();
    let (sk, kid) = keystore::generate_plaintext(&key).unwrap();
    identity::provision(&a, &sk, &kid, "A", "127.0.0.1:7912").await.unwrap();
    write_existing_escrow(&key, "right-op", "RIGHT-CODE");

    let medium = dir.path().join("cairn.medium");
    // A WRONG op-pass cannot unwrap the LSK, so the export seal fails deterministically (no tty
    // prompt involved). The event backup must still complete and the command exit 0.
    let out = cairn_node()
        .args(["--conn", &base, "--key"])
        .arg(&key)
        .args(["backup", "--to"])
        .arg(&medium)
        .args(["--passphrase", "wrong-op"])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "backup must exit 0 even when the optional export can't be sealed; stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(medium.exists(), "the event medium (load-bearing copy) must be written");
    assert!(
        !cairn_node::localstate::localstate_path_for(&medium).exists(),
        "a failed seal must NOT leave a partial export sibling"
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("local-state export skipped"),
        "backup must warn that it skipped the export"
    );
}
