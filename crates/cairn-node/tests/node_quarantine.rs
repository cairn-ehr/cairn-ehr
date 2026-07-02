//! Issue #111 — durable quarantine for the node-event pull plane. The node-plane
//! sibling of cairn-sync's clinical quarantine (#108/PR #110). DB-gated: needs
//! CAIRN_TEST_PG (fresh DB + cairn_pgx).
//!
//! Semantics mirrored from PR #110, adapted to the node plane's deny-all steady
//! state: pull_into pens ONLY an UNVERIFIABLE node_event (never applies without
//! repair), keeps skip-and-sweep for a verifiable-but-refused event (untrusted
//! author / not-yet-have-code — self-heals on a later sweep), records the
//! serving `seq` as a derived re-offer floor, auto-releases a penned row the
//! moment its bytes apply, fails LOUDLY (stats.pending) while any unacked row
//! exists, and lets a human license a permanent exclusion via `acked = TRUE`.
//!
//! The tests use a single-DB self-pull: node A serves its own node_event log to
//! itself over pinned mTLS and pulls it back, so a row raw-inserted into A's
//! node_event is streamed, received, and re-applied through the real admission
//! gate — exercising pull_into's classification end-to-end without a second DB.

use cairn_event::{generate_key, sign, EventBody, Hlc};
use cairn_node::{db, identity, keystore, sync};
use std::net::SocketAddr;
use tokio_postgres::Client;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// Corrupt bytes that cannot verify as a COSE_Sign1/Ed25519 event — the
/// "unverifiable" class the node pen exists for.
const BAD: &[u8] = b"\xde\xad\xbe\xef";

/// Provision node A, self-peer it (so A trusts A for the mutual-mTLS self-pull),
/// bind a serve listener, and return everything a self-pull needs.
struct SelfNode {
    a: Client,
    addr: SocketAddr,
    serve: tokio::task::JoinHandle<anyhow::Result<()>>,
    sk: cairn_event::SigningKey,
    _tmp: tempfile::TempDir,
}

async fn self_node(base: &str, listen_addr: &str) -> SelfNode {
    let a = db::connect_and_load_schema(base).await.unwrap();
    db::reset_node_federation_tables(&a).await.ok();
    let tmp = tempfile::tempdir().unwrap();
    let (sk, kid) = keystore::generate_plaintext(&tmp.path().join("a.key")).unwrap();
    identity::provision(&a, &sk, &kid, "A", listen_addr).await.unwrap();
    let id = identity::load_local(&a).await.unwrap();
    // Self-peer so the mutual-mTLS handshake pins A's own key as trusted.
    let self_bundle = cairn_event::PairingBundle {
        node_id_hex: id.node_id_hex.clone(),
        pubkey_hex: id.pubkey_hex.clone(),
        address: listen_addr.into(),
        fingerprint: cairn_event::short_fingerprint(&id.pubkey_hex).unwrap(),
        nonce: "n".into(),
        hlc: cairn_event::Hlc { wall: 0, counter: 0, node_origin: id.node_id_hex.clone() },
    };
    identity::author_peer(&a, &sk, &kid, &id.node_id_hex, &self_bundle, Some("peer"))
        .await
        .unwrap();
    let trust = sync::trust_store_from_db(&a).await.unwrap();
    let listen: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let (addr, serve_cfg) = sync::bind_serve(listen, base, &sk, trust).await.unwrap();
    let serve = tokio::spawn(sync::serve(serve_cfg));
    SelfNode { a, addr, serve, sk, _tmp: tmp }
}

/// Raw-insert an UNVERIFIABLE node_event into A's log (owner privilege bypasses
/// the C5.4 raw-INSERT floor — this stands in for a corrupt/pre-ADR-0040 frame a
/// peer would serve). Returns the auto-assigned `seq`.
async fn insert_corrupt_node_event(a: &Client) -> i64 {
    a.query_one(
        "INSERT INTO node_event
             (node_event_id, op, author_node_id, subject_node_id, signer_key_id,
              hlc_wall, hlc_counter, node_origin, signed_bytes, content_address)
         VALUES (gen_random_uuid(), 'peer', '\\x00', '\\x00', 'k', 1, 0, 'n',
                 $1::bytea, '\\x1220'::bytea || digest($1::bytea, 'sha256'))
         RETURNING seq",
        &[&BAD.to_vec()],
    )
    .await
    .expect("owner may seed a corrupt served row")
    .get(0)
}

async fn pen_count(a: &Client) -> i64 {
    a.query_one("SELECT count(*) FROM node_event_quarantine", &[])
        .await
        .unwrap()
        .get(0)
}

#[tokio::test]
async fn unverifiable_node_event_is_penned_loudly_and_dedupes_on_reoffer() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let n = self_node(&base, "127.0.0.1:7930").await;
    let corrupt_seq = insert_corrupt_node_event(&n.a).await;

    // First full-sweep pull: the corrupt frame is served, received, refused as
    // unverifiable, and PENNED (not silently skipped past).
    let cfg = sync::client_config(&base, &n.sk, sync::trust_store_from_db(&n.a).await.unwrap())
        .await
        .unwrap();
    let s1 = sync::pull_once(n.addr, cfg, true).await.unwrap();
    assert_eq!(s1.quarantined, 1, "the unverifiable event was quarantined this cycle");
    assert!(s1.pending >= 1, "an unacked pen makes the pull report a LOUD pending count");
    assert_eq!(pen_count(&n.a).await, 1, "exactly one durable pen row");

    let row = n
        .a
        .query_one(
            "SELECT peer, refused_seq, reason, seen_count, acked
               FROM node_event_quarantine
              WHERE content_digest = '\\x1220'::bytea || digest($1::bytea, 'sha256')",
            &[&BAD.to_vec()],
        )
        .await
        .expect("the corrupt bytes are penned under their content digest");
    let refused_seq: i64 = row.get(1);
    let reason: String = row.get(2);
    let seen_count: i32 = row.get(3);
    let acked: bool = row.get(4);
    assert_eq!(refused_seq, corrupt_seq, "the serving seq is recorded (the re-offer floor)");
    assert!(!reason.trim().is_empty(), "a legible refusal reason is stored");
    assert_eq!(seen_count, 1, "first offer");
    assert!(!acked, "a fresh pen is unacked");

    // Second full-sweep pull: the same bytes are re-offered → dedupe onto the one
    // row (seen_count bumps), never a duplicate, still loud.
    let cfg2 = sync::client_config(&base, &n.sk, sync::trust_store_from_db(&n.a).await.unwrap())
        .await
        .unwrap();
    let s2 = sync::pull_once(n.addr, cfg2, true).await.unwrap();
    assert!(s2.pending >= 1, "still loud until resolved or acked");
    assert_eq!(pen_count(&n.a).await, 1, "re-offer dedupes onto the one row");
    let seen_count2: i32 = n
        .a
        .query_one(
            "SELECT seen_count FROM node_event_quarantine
              WHERE content_digest = '\\x1220'::bytea || digest($1::bytea, 'sha256')",
            &[&BAD.to_vec()],
        )
        .await
        .unwrap()
        .get(0);
    assert!(seen_count2 > 1, "seen_count bumped on re-offer, got {seen_count2}");

    n.serve.abort();
}

#[tokio::test]
async fn the_derived_floor_reoffers_a_penned_event_on_an_incremental_pull() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let n = self_node(&base, "127.0.0.1:7934").await;
    insert_corrupt_node_event(&n.a).await;

    // First pull (full sweep) pens the corrupt event and checkpoints the cursor PAST it.
    let cfg = sync::client_config(&base, &n.sk, sync::trust_store_from_db(&n.a).await.unwrap())
        .await
        .unwrap();
    let s1 = sync::pull_once(n.addr, cfg, true).await.unwrap();
    assert_eq!(s1.quarantined, 1, "penned on the first sweep");
    let seen1: i32 = n
        .a
        .query_one(
            "SELECT seen_count FROM node_event_quarantine
              WHERE content_digest = '\\x1220'::bytea || digest($1::bytea, 'sha256')",
            &[&BAD.to_vec()],
        )
        .await
        .unwrap()
        .get(0);

    // Now an INCREMENTAL pull (full_sweep = false): the cursor sits past the refused slot,
    // so ONLY the derived floor (fetching from refused_seq - 1) can re-offer it. If the
    // floor were off by one (fetching from refused_seq, which `serve` streams with a STRICT
    // seq > after_seq), the event would be skipped and seen_count would not move.
    let cfg2 = sync::client_config(&base, &n.sk, sync::trust_store_from_db(&n.a).await.unwrap())
        .await
        .unwrap();
    let s2 = sync::pull_once(n.addr, cfg2, false).await.unwrap();
    assert!(s2.received >= 1, "the incremental pull re-received the penned slot via the floor");
    assert!(s2.pending >= 1, "still loud");
    let seen2: i32 = n
        .a
        .query_one(
            "SELECT seen_count FROM node_event_quarantine
              WHERE content_digest = '\\x1220'::bytea || digest($1::bytea, 'sha256')",
            &[&BAD.to_vec()],
        )
        .await
        .unwrap()
        .get(0);
    assert!(seen2 > seen1, "incremental re-offer bumped seen_count ({seen1} -> {seen2})");

    n.serve.abort();
}

#[tokio::test]
async fn acking_a_pen_row_silences_the_loud_pull() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let n = self_node(&base, "127.0.0.1:7931").await;
    insert_corrupt_node_event(&n.a).await;

    let cfg = sync::client_config(&base, &n.sk, sync::trust_store_from_db(&n.a).await.unwrap())
        .await
        .unwrap();
    let s1 = sync::pull_once(n.addr, cfg, true).await.unwrap();
    assert!(s1.pending >= 1, "loud before ack");

    // A human licenses the permanent exclusion.
    n.a.execute(
        "UPDATE node_event_quarantine SET acked = TRUE
          WHERE content_digest = '\\x1220'::bytea || digest($1::bytea, 'sha256')",
        &[&BAD.to_vec()],
    )
    .await
    .unwrap();

    // Next pull: the row is still re-offered (the peer still serves the bytes) but
    // an acked row no longer counts as pending — the pull is no longer loud.
    let cfg2 = sync::client_config(&base, &n.sk, sync::trust_store_from_db(&n.a).await.unwrap())
        .await
        .unwrap();
    let s2 = sync::pull_once(n.addr, cfg2, true).await.unwrap();
    assert_eq!(s2.pending, 0, "an acked pen no longer makes the pull loud");
    assert_eq!(pen_count(&n.a).await, 1, "the acked row is retained as the recorded decision");

    n.serve.abort();
}

#[tokio::test]
async fn a_penned_event_that_now_applies_is_auto_released() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let n = self_node(&base, "127.0.0.1:7932").await;

    // Simulate a previously-quarantined event whose cause is now fixed: pin a pen
    // row whose content_digest is A's genesis enroll — a VALID, trusted event that
    // A already holds and will re-apply idempotently on the next self-pull. On a
    // successful apply, pull_into must DELETE the stale pen row (auto-requeue).
    let (digest, seq): (Vec<u8>, i64) = {
        let r = n
            .a
            .query_one(
                "SELECT content_address, seq FROM node_event WHERE op='enroll' ORDER BY seq LIMIT 1",
                &[],
            )
            .await
            .unwrap();
        (r.get(0), r.get(1))
    };
    n.a.execute(
        "INSERT INTO node_event_quarantine (content_digest, signed_bytes, peer, refused_seq, reason)
         SELECT content_address, signed_bytes, $1, $2, 'stale pen from a since-fixed cause'
           FROM node_event WHERE op='enroll' ORDER BY seq LIMIT 1",
        &[&n.addr.to_string(), &seq],
    )
    .await
    .unwrap();
    assert_eq!(pen_count(&n.a).await, 1, "the stale pen row is present before the pull");

    let cfg = sync::client_config(&base, &n.sk, sync::trust_store_from_db(&n.a).await.unwrap())
        .await
        .unwrap();
    let s = sync::pull_once(n.addr, cfg, true).await.unwrap();
    assert_eq!(s.pending, 0, "no unacked pen remains after the event applied");

    let still_there: i64 = n
        .a
        .query_one("SELECT count(*) FROM node_event_quarantine WHERE content_digest = $1", &[&digest])
        .await
        .unwrap()
        .get(0);
    assert_eq!(still_there, 0, "a penned event that now applies is auto-released (deleted)");

    n.serve.abort();
}

#[tokio::test]
async fn list_and_ack_quarantine_helpers_drive_the_cli_surface() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let n = self_node(&base, "127.0.0.1:7935").await;
    insert_corrupt_node_event(&n.a).await;
    let cfg = sync::client_config(&base, &n.sk, sync::trust_store_from_db(&n.a).await.unwrap())
        .await
        .unwrap();
    sync::pull_once(n.addr, cfg, true).await.unwrap(); // pens the corrupt event

    // `cairn-node quarantine` (list): one row with the expected shape.
    let rows = sync::list_node_quarantine(&n.a).await.unwrap();
    assert_eq!(rows.len(), 1, "one penned row is listed");
    let r = &rows[0];
    let digest = r["digest"].as_str().expect("digest is hex text");
    assert!(!digest.is_empty(), "digest present");
    assert!(r["refused_seq"].as_i64().unwrap() >= 1, "refused_seq recorded");
    assert!(!r["reason"].as_str().unwrap().trim().is_empty(), "legible reason");
    assert_eq!(r["acked"], serde_json::json!(false), "fresh pen is unacked");

    // An unknown-but-valid-hex digest acks nothing (0 rows) — the CLI reports "no such digest".
    assert_eq!(
        sync::ack_node_quarantine(&n.a, "00").await.unwrap(),
        0,
        "acking an absent digest updates no rows"
    );
    // A non-hex digest is a legible error, not a panic.
    assert!(
        sync::ack_node_quarantine(&n.a, "not-hex!").await.is_err(),
        "a non-hex digest is rejected"
    );

    // `cairn-node ack-quarantine <digest>`: acks exactly the one row.
    assert_eq!(
        sync::ack_node_quarantine(&n.a, digest).await.unwrap(),
        1,
        "acking the listed digest updates its row"
    );
    let after = sync::list_node_quarantine(&n.a).await.unwrap();
    assert_eq!(after[0]["acked"], serde_json::json!(true), "the row now reads acked");

    n.serve.abort();
}

#[tokio::test]
async fn a_verifiable_but_refused_event_is_skipped_not_penned() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let n = self_node(&base, "127.0.0.1:7933").await;

    // A VALID, correctly-signed event — but of a type the node-event gate refuses
    // (a clinical `note.added` is not a node event: `apply_remote_node_event`
    // fail-closed-classifies it as unknown). This stands in for the node plane's
    // normal deny-all steady state (a verifiable event the gate won't admit): it
    // must be SKIPPED and swept, NEVER penned — penning it would flood the pen with
    // ordinary, self-healing refusals.
    let (sk, kid) = generate_key().unwrap();
    let body = EventBody {
        event_id: uuid::Uuid::now_v7().to_string(),
        patient_id: uuid::Uuid::now_v7().to_string(),
        event_type: "note.added".into(),
        schema_version: "note/1".into(),
        hlc: Hlc { wall: 1, counter: 0, node_origin: "stranger".into() },
        t_effective: None,
        signer_key_id: kid.clone(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: serde_json::json!({"text": "not a node event"}),
        attachments: vec![],
        plaintext_twin: Some("a valid but non-node event".into()),
    };
    let signed = sign(&body, &sk).unwrap().signed_bytes;
    // Raw-insert as a served node_event row (op is just the table's column; the gate
    // re-derives the real type from the signed body, so it sees `note.added`).
    n.a.execute(
        "INSERT INTO node_event
             (node_event_id, op, author_node_id, subject_node_id, signer_key_id,
              hlc_wall, hlc_counter, node_origin, signed_bytes, content_address)
         VALUES (gen_random_uuid(), 'peer', '\\x00', '\\x00', 'k', 1, 0, 'n',
                 $1::bytea, '\\x1220'::bytea || digest($1::bytea, 'sha256'))",
        &[&signed],
    )
    .await
    .unwrap();

    let cfg = sync::client_config(&base, &n.sk, sync::trust_store_from_db(&n.a).await.unwrap())
        .await
        .unwrap();
    let s = sync::pull_once(n.addr, cfg, true).await.unwrap();
    assert!(s.rejected >= 1, "the unknown-type event was refused (deny-all), got {}", s.rejected);
    assert_eq!(s.quarantined, 0, "a verifiable refusal is NOT penned");
    assert_eq!(s.pending, 0, "no unacked pen ⇒ the pull is not loud");
    assert_eq!(pen_count(&n.a).await, 0, "the pen stays empty for a self-healing refusal");

    n.serve.abort();
}
