//! ADR-0026 slice D — integration tests for the sealed local-state export.
//! DB-gated tests need CAIRN_TEST_PG (local PG with cairn_pgx installed); offline tests
//! always run.

use cairn_node::db;
use cairn_node::localstate::{
    apply_local_state, establish_lsk, from_cbor, localstate_path_for, lsk_sidecar_path_for,
    parse_container, read_local_state, seal_local_state, serialize_container, serialize_sidecar,
    to_cbor, unseal_local_state_rec, CustodyKeyDestination, LocalState,
};
use tempfile::tempdir;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// The round-trip's READ half, over a node that genuinely holds nothing to export.
///
/// The emptiness asserted here is **correct, not a defect**. `reset_node_federation_tables`
/// runs first, so there is no `event_dek` custody to carry; and `None` is passed for the
/// unwrap secret, which is what a caller that could not load `<key>.unwrap` supplies. An
/// empty bundle is the right answer to both, and applying one must be a clean noop.
///
/// **What this test is NOT.** It is not evidence that the export is empty in general —
/// since #495/ADR-0066 a provisioned node's export carries its surviving `event_dek`
/// custody and its unwrap secret. That property is pinned where it can actually be
/// observed, against a database holding real custody, in
/// `dr_clinical_guarantee_gap.rs::the_export_carries_the_unwrap_secret_and_the_surviving_dek`.
/// If this test ever stops resetting the federation tables, it stops being true.
///
/// (Two earlier framings of this test were wrong in turn — first "no clinical surface yet,
/// so the tier's bundle is empty", falsified by ADR-0052's born-sealed bodies; then "the
/// emptiness is a defect", falsified by ADR-0066. The name was already changed once off
/// `..._is_empty_at_the_federation_tier` for the first of those, because a test name is a
/// claim. The stable claim is the one above: this node has nothing, so it exports nothing.)
#[tokio::test]
async fn read_local_state_returns_the_empty_bundle() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let conn = db::connect_and_load_schema(&base).await.unwrap();
    db::reset_node_federation_tables(&conn).await.ok();
    // The CLINICAL custody plane too, and this line is load-bearing since #495. Before it,
    // `read_local_state` ignored the database entirely, so "no custody" needed no setup.
    // Now the answer depends on `event_dek` — and `reset_node_federation_tables` truncates
    // only the FEDERATION tables (`node_event`, `local_node`, `sync_cursor`, `hlc_state`,
    // `node_event_quarantine`), never this one. These integration binaries share one
    // serialized database, so a sibling suite's leftover custody row would otherwise make
    // this test fail depending on binary order.
    conn.batch_execute("TRUNCATE event_dek, erasure_shred_log CASCADE")
        .await
        .expect("clearing the custody plane so this node genuinely holds nothing");

    // `None`: the caller could not load an unwrap secret. The export is still built — see
    // `read_local_state`'s doc for why that is a warn, not an abort.
    let ls = read_local_state(&conn, None)
        .await
        .expect("read must succeed");
    assert!(
        ls.is_empty(),
        "a node with no custody and no secret to carry exports an empty bundle"
    );
    // Applying an empty bundle is a clean noop. Since ADR-0066 decision 4 the applier
    // INSTALLS a carried unwrap key, so it needs a destination to install it at — but this
    // bundle carries none, so nothing is written and the tempdir path below is never touched.
    // Asserting that is the point: "no secret to install" must stay distinguishable from
    // "installed something", and the report is where a caller reads the difference.
    let dir = tempdir().unwrap();
    let report = apply_local_state(
        &conn,
        &ls,
        &CustodyKeyDestination::Sealed {
            path: &dir.path().join("node.key.unwrap"),
            op_pass: "op-pass-for-a-bundle-that-carries-nothing",
            recovery_code: "recovery-code-for-a-bundle-that-carries-nothing",
        },
    )
    .await
    .expect("applying an empty bundle is a noop");
    assert_eq!(
        report.unwrap_key_installed, None,
        "an empty bundle installs no custody key — and must say so rather than claim one"
    );
    assert_eq!(report.episode_deks_carried, 0);
    assert!(
        !dir.path().join("node.key.unwrap").exists(),
        "nothing may be written when there is nothing to install"
    );
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
