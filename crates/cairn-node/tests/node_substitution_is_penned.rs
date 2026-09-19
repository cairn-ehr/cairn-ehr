//! #619 / ADR-0073 — a substituted node event arriving over the network is PENNED, not skipped.
//!
//! Since Task 3 the admission gate refuses a rival under a held id, with a P0001 like every other
//! refusal. The node puller's P0001 arm skips-and-advances, because on the node plane a P0001 is
//! almost always SCOPING (an event from a node this one does not peer with) and heals on a later
//! sweep. A substitution never heals — the id is taken — so skipping it would log "recoverable,
//! non-fatal" for something that is neither, and keep no trace of a peer that served two different
//! signed events under one id. The puller asks the table (by STATE — `sync/substitution.rs`) and
//! pens it: durable, loud every cycle until a human acks.
//!
//! These use the single-DB self-pull (`common/node_plane_kit.rs`): node A holds the genuine event,
//! and a raw-inserted served row carries the rival's bytes, so the real `pull_into` streams it,
//! re-applies it through the real gate, and classifies the refusal. The ordinary scoping refusal is
//! pinned as still SKIPPED by `node_quarantine.rs::a_verifiable_but_refused_event_is_skipped_not_penned`.

#[path = "common/node_plane_kit.rs"]
mod node_plane_kit;

use cairn_event::generate_key;
use cairn_node::{db, sync};
use node_plane_kit::{
    address_of, call, cs, held_address, node_id_hex, peer_event, pen_count, self_node, serve_raw,
    SelfNode,
};
use uuid::Uuid;

/// A holds `peer.added` under `contested` (through its own door); `rival` is served under the same
/// id. Returns (held bytes, rival bytes).
async fn hold_then_serve_a_rival(
    n: &SelfNode,
    contested: Uuid,
    rival_signer: &cairn_event::SigningKey,
) -> (Vec<u8>, Vec<u8>) {
    let subject = node_id_hex(1);
    let held = peer_event(&n.sk, "peer.added", contested, &subject);
    let rival = peer_event(rival_signer, "peer.revoked", contested, &subject);
    call(&n.a, "submit_node_event", &held)
        .await
        .expect("A holds the genuine event");
    serve_raw(&n.a, &rival).await;
    (held, rival)
}

async fn full_pull(base: &str, n: &SelfNode) -> sync::PullStats {
    let cfg = sync::client_config(base, &n.sk, sync::trust_store_from_db(&n.a).await.unwrap())
        .await
        .unwrap();
    sync::pull_once(n.addr, cfg, true).await.unwrap()
}

async fn pen_reason(n: &SelfNode, rival: &[u8]) -> String {
    n.a.query_one(
        "SELECT reason FROM node_event_quarantine WHERE content_digest = $1",
        &[&address_of(rival)],
    )
    .await
    .expect("the rival has a pen row")
    .get(0)
}

#[tokio::test]
async fn a_rival_under_a_held_id_is_penned_not_skipped() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let n = self_node(&base, "127.0.0.1:7951").await;
    let contested = Uuid::now_v7();
    let signer = n.sk.clone();
    let (held, rival) = hold_then_serve_a_rival(&n, contested, &signer).await;

    let s = full_pull(&base, &n).await;

    assert_eq!(
        s.quarantined, 1,
        "the rival is PENNED — skipping it would file a substitution under 'self-healing', which \
         it can never be"
    );
    assert_eq!(
        s.rejected, 0,
        "nothing else in the stream is refused, and the rival is not counted as a skip"
    );
    assert!(
        s.pending >= 1,
        "an unacked substitution makes the pull LOUD"
    );
    let reason = pen_reason(&n, &rival).await;
    assert!(
        reason.starts_with("substitution:") && reason.contains(&contested.to_string()),
        "the pen row says what it is and which id: {reason}"
    );
    assert_eq!(
        held_address(&n.a, contested).await,
        Some(address_of(&held)),
        "the genuine event is untouched"
    );
    n.serve.abort();
}

/// An acked substitution stays quiet: the row keeps its ack through every re-offer (the pen
/// dedupes onto it), so a human's decision is not undone by the next full sweep.
#[tokio::test]
async fn an_acked_substitution_stays_quiet_on_reoffer() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let n = self_node(&base, "127.0.0.1:7952").await;
    let signer = n.sk.clone();
    let (_held, rival) = hold_then_serve_a_rival(&n, Uuid::now_v7(), &signer).await;

    let first = full_pull(&base, &n).await;
    assert_eq!(first.quarantined, 1, "penned on the first sweep");
    let acked = sync::ack_node_quarantine(&n.a, &hex::encode(address_of(&rival)))
        .await
        .unwrap();
    assert_eq!(acked, 1, "the ack found the substitution's row");

    let second = full_pull(&base, &n).await;
    assert_eq!(
        second.pending, 0,
        "an acked substitution no longer makes the pull loud"
    );
    assert_eq!(
        pen_count(&n.a).await,
        1,
        "no second row: the re-offer deduped onto the first"
    );
    let still_acked: bool =
        n.a.query_one(
            "SELECT acked FROM node_event_quarantine WHERE content_digest = $1",
            &[&address_of(&rival)],
        )
        .await
        .unwrap()
        .get(0);
    assert!(still_acked, "the re-offer must not un-ack a human decision");
    n.serve.abort();
}

/// "Whichever check refused it." A rival signed by a key A does not trust is refused by the
/// author check BEFORE the door reaches its substitution guard — and it is still a rival under a
/// held id, so it is still penned, never filed under self-healing.
#[tokio::test]
async fn a_rival_refused_by_an_earlier_check_is_still_penned() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let n = self_node(&base, "127.0.0.1:7953").await;
    let (stranger, _) = generate_key().unwrap();
    let contested = Uuid::now_v7();
    let (_held, rival) = hold_then_serve_a_rival(&n, contested, &stranger).await;

    let s = full_pull(&base, &n).await;

    assert_eq!(
        s.quarantined, 1,
        "a rival under a held id is penned whatever refused it"
    );
    assert_eq!(s.rejected, 0, "and is not filed under self-healing");
    assert!(pen_reason(&n, &rival).await.starts_with("substitution:"));
    n.serve.abort();
}
