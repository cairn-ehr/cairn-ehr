//! ADR-0026 slice D — integration tests for the sealed local-state export.
//! DB-gated tests need CAIRN_TEST_PG (local PG with cairn_pgx installed); offline tests
//! always run.

use cairn_node::db;
use cairn_node::localstate::{
    apply_local_state, establish_lsk, from_cbor, localstate_path_for, lsk_sidecar_path_for,
    parse_container, read_local_state, seal_local_state, serialize_container, serialize_sidecar,
    to_cbor, unseal_local_state_rec, LocalState,
};
use tempfile::tempdir;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// The export comes back empty. **The assertion is still true; its old justification was
/// not** — this test used to say "no clinical surface yet => the bundle is empty", a
/// sentence ADR-0052 falsified when it made every clinical body born-sealed. The emptiness
/// is now a **defect** (#495), pinned properly — against a database that genuinely holds
/// custody — by `dr_clinical_guarantee_gap.rs`. Kept here as the round-trip's read half.
#[tokio::test]
async fn read_local_state_is_empty_at_the_federation_tier() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let conn = db::connect_and_load_schema(&base).await.unwrap();
    db::reset_node_federation_tables(&conn).await.ok();

    let ls = read_local_state(&conn).await.expect("read must succeed");
    assert!(
        ls.is_empty(),
        "the export is empty — a DEFECT since ADR-0052, not the tier's shape (#495)"
    );
    // Applying an empty bundle is a clean noop (the seam the clinical tier extends).
    apply_local_state(&conn, &ls)
        .await
        .expect("applying an empty bundle is a noop");
}

#[test]
fn export_then_restore_roundtrips_an_empty_bundle_offline() {
    // Pure/offline slice of the round-trip (no DB): seal an empty bundle under an LSK,
    // write the CAIRNL1 sibling, then unseal it via the recovery code and apply-check it.
    let dir = tempdir().unwrap();
    let medium = dir.path().join("cairn.medium");
    let op = "op-pass";
    let code = "AB12C-D34EF";

    let wraps = establish_lsk(op, code).unwrap();
    let bundle = to_cbor(&LocalState::empty());
    let sealed = seal_local_state(&wraps, op, &bundle).unwrap();
    let export_path = localstate_path_for(&medium);
    std::fs::write(&export_path, serialize_container(&sealed)).unwrap();

    // Restore side: read the sibling, unseal with the OLD recovery code, decode, check empty.
    let bytes = std::fs::read(&export_path).unwrap();
    let parsed = parse_container(&bytes).unwrap();
    let plaintext = unseal_local_state_rec(&parsed, code).expect("recovery code must unseal");
    let restored = from_cbor(&plaintext).unwrap();
    assert!(restored.is_empty(), "an empty bundle restores empty");
}

#[test]
fn sidecar_written_atomically_is_readable() {
    // The `.lsk` escrow the CLI writes must parse back (guards the serialize/atomic-write pair).
    let dir = tempdir().unwrap();
    let key = dir.path().join("node.key");
    let wraps = establish_lsk("op", "REC-CODE").unwrap();
    cairn_node::fsio::atomic_write(
        &lsk_sidecar_path_for(&key),
        &serialize_sidecar(&wraps),
        Some(0o600),
    )
    .unwrap();
    let back = std::fs::read(lsk_sidecar_path_for(&key)).unwrap();
    assert!(cairn_node::localstate::parse_sidecar(&back).is_ok());
}

#[test]
fn corrupt_container_parses_as_error_not_panic() {
    // A bit-rotted export sibling must surface as Err so restore can WARN+skip
    // (honest degradation) rather than bailing an already-restored node.
    let garbage = b"CAIRNL1\nnot valid cbor at all";
    assert!(cairn_node::localstate::parse_container(garbage).is_err());
    assert!(cairn_node::localstate::parse_container(b"no magic here").is_err());
}
