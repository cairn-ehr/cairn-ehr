//! ADR-0026 slice B — backup-as-cold-peer (export + self-verify) end-to-end against a
//! real node. DB-gated: needs CAIRN_TEST_PG (a database with `cairn_pgx` + the node
//! schema). Proves the round trip a solo clinic's durability story rests on:
//!   provision -> author -> back up the real node_event set -> the medium self-verifies
//!   -> a bit-rotted medium is caught -> backup health is recorded honestly.
//!
//! The APPLY/restore-into-a-DB half (and the new-identity `supersede` ceremony) is slice
//! C; this exercises only the export + verification + health surface.

use cairn_node::{backup, db, identity, keystore};

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// Provision a node and author one peer.added, so node_event holds two real signed
/// events. Returns the connected client.
async fn provisioned_node(base: &str, keydir: &std::path::Path) -> tokio_postgres::Client {
    let a = db::connect_and_load_schema(base).await.unwrap();
    db::reset_node_federation_tables(&a).await.ok();
    let (sk, kid) = keystore::generate_plaintext(&keydir.join("node.key")).unwrap();
    identity::provision(&a, &sk, &kid, "A", "127.0.0.1:7912")
        .await
        .unwrap();

    // A second event so the medium holds more than the genesis (exercises framing of
    // multiple events). Author a peer.added against a self-referential bundle.
    let id = identity::load_local(&a).await.unwrap();
    let bundle = cairn_event::PairingBundle {
        node_id_hex: id.node_id_hex.clone(),
        pubkey_hex: id.pubkey_hex.clone(),
        address: "127.0.0.1:7913".into(),
        fingerprint: cairn_event::short_fingerprint(&id.pubkey_hex).unwrap(),
        nonce: "n".into(),
        hlc: cairn_event::Hlc {
            wall: 0,
            counter: 0,
            node_origin: id.node_id_hex.clone(),
        },
    };
    identity::author_peer(&a, &sk, &kid, &id.node_id_hex, &bundle, Some("peer"))
        .await
        .unwrap();
    a
}

/// The happy path: the exported medium holds exactly the node's event set, every event
/// self-verifies, and backup health is recorded with the right count.
#[tokio::test]
async fn backup_exports_a_self_verifying_medium_and_records_health() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let a = provisioned_node(&base, dir.path()).await;

    // read_event_set returns the real signed bytes, and each one verifies.
    let events = backup::read_event_set(&a).await.unwrap();
    assert_eq!(events.len(), 2, "genesis + one peer.added");
    assert!(
        backup::verify_events(&events).all_intact(),
        "every real node_event must verify"
    );

    // Back up to a medium beside a health sidecar. No marker_key → an UNSIGNED self-marker
    // (still parses to the exact event set; the signed path is exercised below + in restore).
    let medium = dir.path().join("cairn.medium");
    let health_path = backup::health_path_for(&dir.path().join("node.key"));
    let report = backup::backup_to(&a, &medium, &health_path, 1_000, None)
        .await
        .unwrap();
    assert_eq!(report.event_count, 2);
    assert_eq!(
        report.marker,
        backup::WrittenMarker::Unsigned,
        "no key → unsigned marker"
    );

    // The medium on disk parses and every event verifies (self-verifying by construction).
    let bytes = std::fs::read(&medium).unwrap();
    let parsed = backup::parse_medium(&bytes).unwrap();
    assert_eq!(
        parsed, events,
        "medium holds exactly the node's event set, in order"
    );
    assert!(
        backup::verify_medium_bytes(&bytes).unwrap().all_intact(),
        "the freshly written medium must fully self-verify"
    );

    // Health was recorded (proves backup_to's read-after-write verify passed) and is honest.
    let health = backup::read_health(&health_path).expect("health sidecar must exist after backup");
    assert_eq!(health.event_count, 2);
    assert_eq!(health.last_backup_unix, 1_000);
    assert!(
        backup::describe_health(1_000, &Some(health)).starts_with("just now"),
        "a backup at now reads as fresh"
    );
}

/// A backup taken WITH the node's key writes a SIGNED self-marker that round-trips through
/// disk and resolves to THIS node on restore — the tamper-evident path (issue #53). Exercises
/// the full wire: build the attestation from the live `local_node` + key, frame it into the
/// CAIRNB2 container, re-read from disk, and resolve self through the signature/bind check.
#[tokio::test]
async fn backup_with_key_writes_a_signed_marker_that_resolves_self() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let a = db::connect_and_load_schema(&base).await.unwrap();
    db::reset_node_federation_tables(&a).await.ok();
    let (sk, kid) = keystore::generate_plaintext(&dir.path().join("node.key")).unwrap();
    identity::provision(&a, &sk, &kid, "Clinic-A", "127.0.0.1:7960")
        .await
        .unwrap();
    let self_id = identity::load_local(&a).await.unwrap().node_id_hex;

    let medium = dir.path().join("cairn.medium");
    let health_path = backup::health_path_for(&dir.path().join("node.key"));
    let report = backup::backup_to(&a, &medium, &health_path, 1_000, Some((&sk, &kid)))
        .await
        .unwrap();
    assert_eq!(
        report.marker,
        backup::WrittenMarker::Signed,
        "key present → signed marker"
    );

    // Re-read the on-disk medium and resolve self through the signed marker.
    let bytes = std::fs::read(&medium).unwrap();
    let container = cairn_node::medium::parse_container(&bytes).unwrap();
    assert!(matches!(
        container.self_marker,
        Some(cairn_node::medium::SelfMarker::Signed(_))
    ));
    let dead = cairn_node::restore::resolve_dead_node(&container, None).unwrap();
    assert_eq!(
        dead.node_id_hex, self_id,
        "signed marker resolves to this node"
    );
    assert_eq!(dead.provenance, cairn_node::restore::Provenance::Signed);
}

/// A bit-rotted / tampered medium is caught by the SAME signature invariant that catches
/// a hostile peer — no separate "is the backup intact?" mechanism (ADR-0026 point 2).
#[tokio::test]
async fn a_bitrotted_medium_fails_self_verification() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let a = provisioned_node(&base, dir.path()).await;

    let medium = dir.path().join("cairn.medium");
    let health_path = backup::health_path_for(&dir.path().join("node.key"));
    backup::backup_to(&a, &medium, &health_path, 1_000, None)
        .await
        .unwrap();

    // Corrupt a byte inside the FIRST event's body (parse -> flip -> re-serialize), so the
    // container still parses structurally but that event's signature no longer checks.
    // (Flipping a raw file offset could land on a length prefix and fail parsing instead —
    // we want to prove the cryptographic check, not the structural one.)
    let mut parsed = backup::parse_medium(&std::fs::read(&medium).unwrap()).unwrap();
    let mid = parsed[0].len() / 2;
    parsed[0][mid] ^= 0xff;
    let corrupted = cairn_node::medium::serialize_container(None, &parsed);
    let report = backup::verify_medium_bytes(&corrupted).unwrap();
    assert!(
        !report.all_intact(),
        "a corrupted medium must fail verification"
    );
    assert_eq!(
        report.first_bad,
        Some(0),
        "verification must point at the corrupt event"
    );
}
