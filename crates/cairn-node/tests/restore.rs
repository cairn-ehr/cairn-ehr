//! ADR-0026 slice C — restore (apply) + new-identity supersede.
//! DB-gated: needs CAIRN_TEST_PG (local PG with cairn_pgx installed).

use cairn_event::{sign, EventBody, Hlc, SigningKey};
use cairn_node::{db, identity, keystore};

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// A live node can author a node.superseded event naming a dead node-id; it lands as an
/// op='supersede' row and node_lineage resolves the edge (new <- old).
#[tokio::test]
async fn author_supersede_records_a_lineage_edge() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let a = db::connect_and_load_schema(&base).await.unwrap();
    db::reset_node_federation_tables(&a).await.ok();

    let tmp = tempfile::tempdir().unwrap();
    let (sk, kid) = keystore::generate_plaintext(&tmp.path().join("a.key")).unwrap();
    identity::provision(&a, &sk, &kid, "A", "127.0.0.1:7920")
        .await
        .unwrap();
    let id = identity::load_local(&a).await.unwrap();

    // Author supersede naming a fabricated "old" node-id (any hex node-id works here;
    // the full restore flow supplies the real one in Task 5).
    let old_hex = "1220".to_string() + &"ab".repeat(32);
    identity::author_supersede(&a, &sk, &kid, &id.node_id_hex, &old_hex)
        .await
        .unwrap();

    let row = a
        .query_one(
            "SELECT encode(superseded_node_id,'hex') AS old, encode(new_node_id,'hex') AS new
         FROM node_lineage",
            &[],
        )
        .await
        .unwrap();
    let old: String = row.get("old");
    let new: String = row.get("new");
    assert_eq!(old, old_hex, "lineage subject == the dead node-id");
    assert_eq!(
        new, id.node_id_hex,
        "lineage author == the live (new) node-id"
    );
}

/// Mint a real signed peer.added event for an arbitrary key (no DB). Used to exercise
/// the restore door's non-enroll branch (author resolution via node_current).
fn synth_peer(sk: &SigningKey, name: &str, peer_node_id_hex: &str, peer_pubkey: &str) -> Vec<u8> {
    let kid = hex::encode(sk.verifying_key().to_bytes());
    let body = EventBody {
        event_id: uuid::Uuid::now_v7().to_string(),
        patient_id: identity::NIL_PATIENT.into(),
        event_type: "peer.added".into(),
        schema_version: "node/1".into(),
        hlc: Hlc {
            wall: 2,
            counter: 0,
            node_origin: name.into(),
        },
        t_effective: None,
        signer_key_id: kid,
        contributors: serde_json::json!([]),
        payload: serde_json::json!({
            "peer_node_id_hex": peer_node_id_hex, "peer_pubkey": peer_pubkey,
            "fingerprint": "fp", "role": "peer"
        }),
        attachments: vec![],
        plaintext_twin: None,
    };
    sign(&body, sk).unwrap().signed_bytes
}

/// Mint a real signed node.enrolled event for an arbitrary key (no DB). Mirrors how
/// identity::provision builds genesis, so its content-address == the node-id.
fn synth_enroll(sk: &SigningKey, name: &str) -> Vec<u8> {
    let kid = hex::encode(sk.verifying_key().to_bytes());
    let body = EventBody {
        event_id: uuid::Uuid::now_v7().to_string(),
        patient_id: identity::NIL_PATIENT.into(),
        event_type: "node.enrolled".into(),
        schema_version: "node/1".into(),
        hlc: Hlc {
            wall: 1,
            counter: 0,
            node_origin: name.into(),
        },
        t_effective: None,
        signer_key_id: kid,
        contributors: serde_json::json!([]),
        payload: serde_json::json!({ "display_name": name, "address": "127.0.0.1:7999" }),
        attachments: vec![],
        plaintext_twin: None,
    };
    sign(&body, sk).unwrap().signed_bytes
}

/// The restore door must be a no-op on a LIVE node: with a genesis present, it raises a
/// legible "already enrolled" error. This is the structural fence that prevents the
/// self-trusting door from ever bypassing peer-admission on a running node.
#[tokio::test]
async fn restore_door_rejects_on_a_live_node() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let a = db::connect_and_load_schema(&base).await.unwrap();
    db::reset_node_federation_tables(&a).await.ok();

    let tmp = tempfile::tempdir().unwrap();
    let (sk, kid) = keystore::generate_plaintext(&tmp.path().join("a.key")).unwrap();
    identity::provision(&a, &sk, &kid, "Live", "127.0.0.1:7921")
        .await
        .unwrap();

    let other = cairn_event::generate_key().unwrap().0;
    let ev = synth_enroll(&other, "Intruder");
    let err = a
        .execute("SELECT restore_node_event($1)", &[&ev])
        .await
        .unwrap_err();
    let db_msg = err
        .as_db_error()
        .map(|e| e.message())
        .unwrap_or("<no db message>");
    assert!(
        db_msg.contains("already enrolled"),
        "fence must reject on a live node, got: {db_msg}"
    );
}

/// Into a FRESH (un-enrolled) DB, the door applies a validly-signed enroll without any
/// peer-trust — exactly what a node rehydrating its own history needs.
#[tokio::test]
async fn restore_door_accepts_into_an_empty_db() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let a = db::connect_and_load_schema(&base).await.unwrap();
    db::reset_node_federation_tables(&a).await.ok();

    let other = cairn_event::generate_key().unwrap().0;
    let ev = synth_enroll(&other, "Restored");
    a.execute("SELECT restore_node_event($1)", &[&ev])
        .await
        .expect("door applies into an empty DB without peer-trust");
    let n: i64 = a
        .query_one("SELECT count(*) FROM node_event WHERE op='enroll'", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 1, "the restored enroll is present");
    // The door must NOT set local_node (only a real new genesis does).
    let ln: i64 = a
        .query_one("SELECT count(*) FROM local_node", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(ln, 0, "restore_node_event must never write local_node");
}

/// A tampered/bit-rotted medium event fails the door's signature check — the same
/// invariant slice B proves catches a hostile peer.
#[tokio::test]
async fn restore_door_rejects_a_tampered_event() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let a = db::connect_and_load_schema(&base).await.unwrap();
    db::reset_node_federation_tables(&a).await.ok();

    let other = cairn_event::generate_key().unwrap().0;
    let mut ev = synth_enroll(&other, "Tampered");
    let mid = ev.len() / 2;
    ev[mid] ^= 0x01; // break the signature
    let err = a
        .execute("SELECT restore_node_event($1)", &[&ev])
        .await
        .unwrap_err();
    let db_msg = err
        .as_db_error()
        .map(|e| e.message())
        .unwrap_or("<no db message>");
    assert!(
        db_msg.contains("verification failed"),
        "tampered event must fail the signature check, got: {db_msg}"
    );
}

/// Full round-trip: back up node A's event set, restore it into a FRESH db under a NEW
/// identity, and assert the ADR-0026 point-1/4 guarantees hold.
#[tokio::test]
async fn restore_round_trip_rehydrates_under_a_new_identity() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let a = db::connect_and_load_schema(&base).await.unwrap();
    db::reset_node_federation_tables(&a).await.ok();

    // --- original node A ---
    let tmp = tempfile::tempdir().unwrap();
    let (sk_a, kid_a) = keystore::generate_plaintext(&tmp.path().join("a.key")).unwrap();
    identity::provision(&a, &sk_a, &kid_a, "Clinic-A", "127.0.0.1:7930")
        .await
        .unwrap();
    let old = identity::load_local(&a).await.unwrap();

    // Read A's event set the way `backup` does (signed_bytes in seq order).
    let medium: Vec<Vec<u8>> = cairn_node::backup::read_event_set(&a).await.unwrap();
    assert_eq!(medium.len(), 1, "solo node A has just its genesis");

    // --- simulate disk death: a fresh, un-enrolled DB ---
    db::reset_node_federation_tables(&a).await.ok();

    // Wrap the medium in a SIGNED-self-marker container, as `backup` does on a live node.
    let att = cairn_node::medium::build_self_attestation(&sk_a, &kid_a, &old.node_id_hex, &medium);
    let container = cairn_node::medium::Container {
        self_marker: Some(cairn_node::medium::SelfMarker::Signed(att)),
        events: medium,
    };

    // --- restore under a NEW key ---
    let (sk_new, kid_new) = keystore::generate_plaintext(&tmp.path().join("new.key")).unwrap();
    let dead = cairn_node::restore::resolve_dead_node(&container, None).unwrap();
    assert_eq!(
        dead.node_id_hex, old.node_id_hex,
        "signed marker resolves dead node == A"
    );
    assert_eq!(dead.provenance, cairn_node::restore::Provenance::Signed);
    let (name, addr) =
        cairn_node::restore::old_genesis_meta(&container.events, &dead.node_id_hex).unwrap();

    let applied = cairn_node::restore::apply_medium(&a, &container.events)
        .await
        .unwrap();
    assert_eq!(applied, 1);
    let outcome = cairn_node::restore::finalize_identity(
        &a,
        &sk_new,
        &kid_new,
        &name,
        &addr,
        &dead.node_id_hex,
    )
    .await
    .unwrap();

    // (a) old events present:
    let n_enroll: i64 = a
        .query_one("SELECT count(*) FROM node_event WHERE op='enroll'", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(n_enroll, 2, "old genesis (restored) + new genesis");
    // (b) local_node == the new id:
    let local = identity::load_local(&a).await.unwrap();
    assert_eq!(local.node_id_hex, outcome.new_node_id_hex);
    assert_ne!(
        local.node_id_hex, old.node_id_hex,
        "new physical trust boundary"
    );
    // (c) supersede recorded with subject == old id:
    let edge = a
        .query_one(
            "SELECT encode(superseded_node_id,'hex') AS old, encode(new_node_id,'hex') AS new
         FROM node_lineage",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(edge.get::<_, String>("old"), old.node_id_hex);
    assert_eq!(edge.get::<_, String>("new"), local.node_id_hex);
    // (d) trust_peer empty -> must re-peer:
    let peers: i64 = a
        .query_one("SELECT count(*) FROM trust_peer", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        peers, 0,
        "a restored node re-peers from empty (ADR-0026 point 4)"
    );
    // (e) the restore door is now fenced closed:
    let other = cairn_event::generate_key().unwrap().0;
    let ev = synth_enroll(&other, "Late");
    let err = a
        .execute("SELECT restore_node_event($1)", &[&ev])
        .await
        .unwrap_err();
    // Use as_db_error().message() — err.to_string() for a DB error returns "db error",
    // not the RAISE EXCEPTION text (tokio-postgres Display for Kind::Db is a fixed string).
    let db_msg = err
        .as_db_error()
        .map(|e| e.message())
        .unwrap_or("<no db message>");
    assert!(
        db_msg.contains("already enrolled"),
        "door closes after genesis, got: {db_msg}"
    );
}

/// The restore door's non-enroll branch: after the genesis is applied, a subsequent
/// peer.added event resolves its author via node_current and lands successfully.
#[tokio::test]
async fn restore_door_applies_a_non_enroll_event_after_genesis() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let a = db::connect_and_load_schema(&base).await.unwrap();
    db::reset_node_federation_tables(&a).await.ok();

    let a_sk = cairn_event::generate_key().unwrap().0;
    let enroll = synth_enroll(&a_sk, "A");
    let a_id = hex::encode(cairn_event::event_address(&enroll));

    // Apply the genesis through the restore door (no local_node written).
    a.execute("SELECT restore_node_event($1)", &[&enroll])
        .await
        .expect("enroll applies into empty DB");

    // Build a peer.added event authored by the same key as the enroll.
    let peer = synth_peer(
        &a_sk,
        "A",
        &("1220".to_string() + &"ee".repeat(32)),
        &"ff".repeat(32),
    );
    // Must succeed: author resolves via node_current to the restored enroll.
    a.execute("SELECT restore_node_event($1)", &[&peer])
        .await
        .expect("peer.added applies after its author's genesis");

    // Assert the stored peer row's author resolved to A.
    let row = a
        .query_one(
            "SELECT encode(author_node_id,'hex') AS author FROM node_event WHERE op='peer'",
            &[],
        )
        .await
        .unwrap();
    let author: String = row.get("author");
    assert_eq!(
        author, a_id,
        "peer event's author resolved to node A's genesis content-address"
    );
}

/// Issue #53: a REAL federated medium (built through the live doors) carries the node's own
/// genesis AND a peer's, indistinguishable from the events alone (set-union convergence). The
/// medium's SIGNED self-marker resolves self unambiguously, and naming the peer's node-id is
/// rejected fail-closed before any DB write.
#[tokio::test]
async fn federated_medium_resolves_self_and_rejects_a_peer() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let a = db::connect_and_load_schema(&base).await.unwrap();
    db::reset_node_federation_tables(&a).await.ok();

    // Node A is "self".
    let tmp = tempfile::tempdir().unwrap();
    let (sk_a, kid_a) = keystore::generate_plaintext(&tmp.path().join("a.key")).unwrap();
    identity::provision(&a, &sk_a, &kid_a, "Self-A", "127.0.0.1:7950")
        .await
        .unwrap();
    let self_id = identity::load_local(&a).await.unwrap().node_id_hex;

    // Peer B: author B's real genesis, pair A->B (records the peer.added), then admit B's
    // genesis into A's log through the real admission gate.
    let (sk_b, kid_b) = cairn_event::generate_key().unwrap();
    let body_b = EventBody {
        event_id: uuid::Uuid::now_v7().to_string(),
        patient_id: identity::NIL_PATIENT.into(),
        event_type: "node.enrolled".into(),
        schema_version: "node/1".into(),
        hlc: Hlc {
            wall: 1,
            counter: 0,
            node_origin: "Peer-B".into(),
        },
        t_effective: None,
        signer_key_id: kid_b.clone(),
        contributors: serde_json::json!([{"actor_id": kid_b, "role": "device"}]),
        payload: serde_json::json!({"display_name": "Peer-B", "address": "127.0.0.1:7951"}),
        attachments: vec![],
        plaintext_twin: None,
    };
    let signed_b = sign(&body_b, &sk_b).unwrap();
    let peer_id = hex::encode(cairn_event::event_address(&signed_b.signed_bytes));
    let bundle = cairn_event::PairingBundle {
        node_id_hex: peer_id.clone(),
        pubkey_hex: kid_b.clone(),
        address: "127.0.0.1:7951".into(),
        fingerprint: cairn_event::short_fingerprint(&kid_b).unwrap(),
        nonce: "n".into(),
        hlc: Hlc {
            wall: 1,
            counter: 0,
            node_origin: peer_id.clone(),
        },
    };
    identity::author_peer(&a, &sk_a, &kid_a, "Self-A", &bundle, Some("peer"))
        .await
        .unwrap();
    let bytes_b = signed_b.signed_bytes.clone();
    a.execute("SELECT apply_remote_node_event($1)", &[&bytes_b])
        .await
        .unwrap();

    // The medium as `backup` reads it: A.genesis + peer.added(B) + B.genesis (seq order),
    // i.e. a genuine federated medium carrying TWO enrolls. Wrapped with the SIGNED self-marker
    // `backup` writes on a live node (self = A, signed by A's key).
    let events = cairn_node::backup::read_event_set(&a).await.unwrap();
    assert_eq!(
        events.len(),
        3,
        "federated medium: A genesis + peer.added(B) + B genesis"
    );
    let att = cairn_node::medium::build_self_attestation(&sk_a, &kid_a, &self_id, &events);
    let container = cairn_node::medium::Container {
        self_marker: Some(cairn_node::medium::SelfMarker::Signed(att)),
        events,
    };

    // Omitting --superseded-node resolves to SELF via the signed marker (the events alone
    // cannot — A and B are symmetric on the medium). A multi-enroll/federated medium is reported
    // as SignedFederated (resolves self, but confirm-on-restore for the residual splice risk).
    let dead = cairn_node::restore::resolve_dead_node(&container, None).unwrap();
    assert_eq!(
        dead.node_id_hex, self_id,
        "signed marker resolves this node's own genesis"
    );
    assert_eq!(
        dead.provenance,
        cairn_node::restore::Provenance::SignedFederated
    );

    // Naming the PEER's real node-id is rejected fail-closed (the issue #53 footgun).
    let err = cairn_node::restore::resolve_dead_node(&container, Some(&peer_id)).unwrap_err();
    assert!(
        matches!(err, cairn_node::restore::RestoreError::NotSelf { .. }),
        "naming a peer's node-id must fail closed as NotSelf, got: {err:?}"
    );
}

/// After a restore, `status` reports the supersede lineage (this node supersedes the dead one).
#[tokio::test]
async fn status_reports_supersede_lineage() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let a = db::connect_and_load_schema(&base).await.unwrap();
    db::reset_node_federation_tables(&a).await.ok();

    let tmp = tempfile::tempdir().unwrap();
    let (sk, kid) = keystore::generate_plaintext(&tmp.path().join("a.key")).unwrap();
    identity::provision(&a, &sk, &kid, "A", "127.0.0.1:7940")
        .await
        .unwrap();
    let id = identity::load_local(&a).await.unwrap();
    let old_hex = "1220".to_string() + &"cd".repeat(32);
    identity::author_supersede(&a, &sk, &kid, &id.node_id_hex, &old_hex)
        .await
        .unwrap();

    let st = identity::status(&a, &tmp.path().join("a.key"))
        .await
        .unwrap();
    assert_eq!(
        st.supersedes.as_deref(),
        Some(old_hex.as_str()),
        "status must surface the supersede lineage"
    );
}

/// The restore door's fail-closed behaviour: a non-enroll event applied before its
/// author's genesis must be rejected with a legible error.
#[tokio::test]
async fn restore_door_rejects_non_enroll_before_its_genesis() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let a = db::connect_and_load_schema(&base).await.unwrap();
    db::reset_node_federation_tables(&a).await.ok();

    let a_sk = cairn_event::generate_key().unwrap().0;

    // Build a peer.added event from key A but DO NOT apply A's enroll first.
    let peer = synth_peer(
        &a_sk,
        "A",
        &("1220".to_string() + &"ee".repeat(32)),
        &"ff".repeat(32),
    );
    let err = a
        .execute("SELECT restore_node_event($1)", &[&peer])
        .await
        .unwrap_err();
    let db_msg = err
        .as_db_error()
        .map(|e| e.message())
        .unwrap_or("<no db message>");
    assert!(
        db_msg.contains("maps to no restored enroll"),
        "non-enroll before genesis must be rejected, got: {db_msg}"
    );
}
