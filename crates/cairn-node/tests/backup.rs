//! ADR-0026 slice B — backup-as-cold-peer (export + self-verify) end-to-end against a
//! real node. DB-gated: needs CAIRN_TEST_PG (a database with `cairn_pgx` + the node
//! schema). Proves the round trip a solo clinic's durability story rests on:
//!   provision -> author -> back up the real federation plane -> the medium self-verifies
//!   -> a bit-rotted medium is caught -> backup health is recorded honestly.
//!
//! The APPLY/restore-into-a-DB half (and the new-identity `supersede` ceremony) is slice
//! C; this exercises only the export + verification + health surface.
//!
//! # Rewritten for CAIRNB3 (#500 slice 2c Task 9), and what did NOT change
//!
//! `backup_to` now writes an append-only, two-plane **CAIRNB3** medium instead of a
//! whole-set CAIRNB2 container, so the three tests below no longer read their result through
//! `parse_medium`/`parse_container` (which serve CAIRNB1/CAIRNB2 and refuse CAIRNB3 by
//! design — see `container::parse_container`). They read it through `parse_any` and
//! `backup::node_plane_events`, the revision-agnostic pair `restore` and `verify-backup`
//! themselves use, so this suite exercises the same reader an operator's recovery does.
//!
//! Every PROPERTY these tests were written to hold is unchanged and still asserted here:
//! the medium carries the node's federation event set in order, it self-verifies, a
//! bit-rotted medium is caught by the same signature invariant that catches a hostile peer,
//! and health is recorded honestly. What moved is the container the properties are read out
//! of. The CLINICAL plane this node now also captures is pinned next door, in
//! `backup_carries_both_planes.rs`; this file deliberately stays the federation-plane suite
//! it has always been, because that is the half `restore` still applies.
//!
//! One CONSEQUENCE worth stating rather than leaving to be rediscovered: CAIRNB3 has no head
//! self-marker. `backup_with_key_writes_a_signed_marker_that_resolves_self` therefore
//! resolves identity from the per-segment ATTESTATION via `backup::self_marker_for` — the
//! same call `main.rs`'s restore arm makes — rather than from `Container::self_marker`.

use cairn_medium::{parse_any, MediumImage};
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

    // Back up to a medium beside a health sidecar. No marker_key → an UNSIGNED capture: the
    // segments carry no attestation, but every record still verifies and the whole event set
    // still reads back exactly. The signed path is exercised below + in restore.
    let medium = dir.path().join("cairn.medium");
    let health_path = backup::health_path_for(&dir.path().join("node.key"));
    let report = backup::backup_to(&a, &medium, &health_path, 1_000, None)
        .await
        .unwrap();
    assert_eq!(report.node_events, 2);
    assert_eq!(
        report.marker,
        backup::WrittenMarker::Unsigned,
        "no key → an unsigned capture: the medium names this node in plaintext only"
    );
    assert_eq!(
        report.origin,
        backup::MediumOrigin::FirstEver,
        "nothing was at the path, so this is a first backup"
    );

    // The medium on disk parses and every event verifies (self-verifying by construction).
    // Read through `node_plane_events`, the revision-agnostic reader `restore` itself uses,
    // so this assertion is about what a recovery would actually get back.
    let bytes = std::fs::read(&medium).unwrap();
    let image = parse_any(&bytes).unwrap();
    assert!(
        matches!(image, MediumImage::V3(_)),
        "a capture writes CAIRNB3 (#500 slice 2c) — the legacy revisions stay readable but \
         are no longer written"
    );
    let parsed = backup::node_plane_events(&image).unwrap();
    assert_eq!(
        parsed, events,
        "medium holds exactly the node's federation event set, in order"
    );
    assert!(
        backup::verify_events(&parsed).all_intact(),
        "the freshly written medium must fully self-verify"
    );

    // Health was recorded (proves backup_to's read-after-write assessment passed) and is
    // honest. #500 slice 2c Task 10: v2 reports per-plane SCOPE, and Task 9 made the
    // clinical half non-vacuous.
    //
    // The clinical figures are DERIVED FROM THE DATABASE, never hardcoded, and that is not
    // style: this suite provisions a node without truncating `event_log` (it is the
    // federation-plane suite and always has been), and the DB-gated suites share one
    // serialized database, so whatever a previous suite left in `event_log` is legitimately
    // captured onto this medium. A hardcoded `0` would therefore be a statement about the
    // suite that happened to run before this one. Asserting the EQUALITY instead is also the
    // stronger property: the medium's clinical scope is exactly what the log holds.
    let health = backup::read_health(&health_path).expect("health sidecar must exist after backup");
    assert_eq!(health.node_events, 2);

    let log_rows: i64 = a
        .query_one("SELECT count(*) FROM event_log", &[])
        .await
        .unwrap()
        .get(0);
    let log_max_seq: Option<i64> = a
        .query_one("SELECT max(seq) FROM event_log", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        health.clinical_events as i64, log_rows,
        "the medium's clinical scope must equal what `event_log` holds — the whole plane is \
         swept on a first capture"
    );
    assert_eq!(
        health.clinical_watermark, log_max_seq,
        "the clinical watermark is the medium's newest captured seq, which after a full \
         first sweep is the log's own max. `None` on an empty log is the honest absence — \
         `Some(0)` would be a claim"
    );
    assert_eq!(health.last_backup_unix, 1_000);
    assert!(
        backup::describe_health(1_000, &Some(health)).starts_with("just now"),
        "a backup at now reads as fresh"
    );
}

/// A backup taken WITH the node's key writes a SIGNED attestation that round-trips through
/// disk and resolves to THIS node on restore — the tamper-evident path (issue #53). Exercises
/// the full wire: sign each captured segment from the live `local_node` + key, frame it into
/// the CAIRNB3 medium, re-read from disk, and resolve self through the signature/bind check.
///
/// **The bind is stronger on CAIRNB3 than the CAIRNB2 head marker it replaces**, and that is
/// why this test now goes through `backup::self_marker_for`: `chain::self_id_from_chain`
/// returns the ATTESTED node id, checked against a genesis present on THIS medium and signed
/// by the SAME key — never the untrusted plaintext `Segment::self_node_id_hex`.
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
        "key present → the capture's segments carry attestations, so the medium can identify \
         its own node"
    );

    // Re-read the on-disk medium and resolve self exactly the way `main.rs`'s restore arm
    // does — through the REAL `self_marker_for`, never a replica of its logic beside it,
    // because only calling the real function can catch a regression of the call site.
    let bytes = std::fs::read(&medium).unwrap();
    let image = parse_any(&bytes).unwrap();
    let self_marker = backup::self_marker_for(&image);
    assert_eq!(
        self_marker,
        Some(cairn_node::medium::SelfMarker::Unsigned(self_id.clone())),
        "the attested id derived from the chain must be THIS node's. `SelfMarker::Unsigned` \
         is the wrapper, not a weakness: the id came from a verified segment attestation \
         bound to a genesis on this medium (see `self_marker_for`'s doc for why a V3 \
         attestation cannot be wrapped as `Signed`, whose verifier expects a CAIRNB2 \
         whole-set blob)"
    );
    let container = cairn_node::medium::Container {
        self_marker,
        events: backup::node_plane_events(&image).unwrap(),
    };
    let dead = cairn_node::restore::resolve_dead_node(&container, None).unwrap();
    assert_eq!(
        dead.node_id_hex, self_id,
        "the attested marker resolves to this node"
    );
    assert_eq!(dead.provenance, cairn_node::restore::Provenance::Unsigned);

    // AND the marker is actually CONSULTED rather than merely present: an explicit
    // `--superseded-node` naming a node that is not on this medium must fail closed. (The
    // sharper half of the #53 cross-check — a marker rejecting a PEER that IS on the medium,
    // `RestoreError::NotSelf` — needs a two-enroll medium and is pinned in
    // `tests/restore.rs::v3_medium_self_marker_still_rejects_a_named_peer`; it is not
    // re-created here, where the node is sole-enroll by construction.)
    let stranger = "00".repeat(32);
    let err = cairn_node::restore::resolve_dead_node(&container, Some(&stranger)).unwrap_err();
    assert!(
        matches!(err, cairn_node::restore::RestoreError::UnknownNodeId { .. }),
        "an explicit node-id absent from the medium must be refused, got: {err:?}"
    );
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

    // Corrupt a byte inside the FIRST event's body (read the medium -> flip -> re-check), so
    // the medium still parses structurally but that event's signature no longer checks.
    // (Flipping a raw file offset could land on a length prefix and fail parsing instead —
    // we want to prove the cryptographic check, not the structural one.)
    //
    // The events come back through `node_plane_events`, and the corrupted set is checked with
    // `verify_events` directly rather than being re-framed into a container: on CAIRNB3 there
    // is no whole-set re-serialization to round-trip through, and the property under test was
    // never about framing. It is that a bit-rotted event fails the SAME `verify_self_described`
    // check that catches a hostile peer's event — one invariant, no separate "is the backup
    // intact?" mechanism.
    let image = parse_any(&std::fs::read(&medium).unwrap()).unwrap();
    let mut parsed = backup::node_plane_events(&image).unwrap();
    assert!(
        !parsed.is_empty(),
        "anti-vacuity: there must be an event to corrupt"
    );
    let mid = parsed[0].len() / 2;
    parsed[0][mid] ^= 0xff;
    let report = backup::verify_events(&parsed);
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
