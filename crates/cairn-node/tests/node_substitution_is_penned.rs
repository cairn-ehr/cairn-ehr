//! #619 / ADR-0073 — a substituted node event arriving over the network is PENNED, not skipped.
//!
//! Since #619 (ADR-0073) the admission gate (`apply_remote_node_event`) refuses a rival under a
//! held id, with a P0001 like every other refusal. The node puller's P0001 arm
//! skips-and-advances, because on the node plane a P0001 is
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
use cairn_node::{db, identity, sync};
use node_plane_kit::{
    address_of, call, cs, held_address, key_hex, node_id_hex, peer_event, peer_event_spelled,
    pen_count, self_node, serve_raw, spelled_oddly, SelfNode,
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

/// A submits one more, unrelated `peer.added` through its own door — so it is served AFTER
/// whatever was served before it. Returns its serving `seq`.
async fn hold_a_clean_event_after(n: &SelfNode) -> i64 {
    let id = Uuid::now_v7();
    call(
        &n.a,
        "submit_node_event",
        &peer_event(&n.sk, "peer.added", id, &node_id_hex(2)),
    )
    .await
    .expect("A holds a clean event");
    n.a.query_one(
        "SELECT seq FROM node_event WHERE node_event_id = $1::text::uuid",
        &[&id.to_string()],
    )
    .await
    .unwrap()
    .get(0)
}

/// Where this node's committed cursor for the self-pull peer stands, if anywhere.
async fn cursor(n: &SelfNode) -> Option<i64> {
    n.a.query_opt(
        "SELECT last_seq FROM sync_cursor WHERE peer_addr = $1",
        &[&n.addr.to_string()],
    )
    .await
    .unwrap()
    .map(|r| r.get(0))
}

/// Whether the rival's pen row has been acked.
async fn is_acked(n: &SelfNode, rival: &[u8]) -> bool {
    n.a.query_one(
        "SELECT acked FROM node_event_quarantine WHERE content_digest = $1",
        &[&address_of(rival)],
    )
    .await
    .expect("the rival has a pen row")
    .get(0)
}

/// The rival's pen-row history: when it was first seen (as text, to compare exactly) and how
/// many times it has been offered.
async fn row_history(n: &SelfNode, rival: &[u8]) -> (String, i32) {
    let r =
        n.a.query_one(
            "SELECT first_seen::text, seen_count FROM node_event_quarantine
              WHERE content_digest = $1",
            &[&address_of(rival)],
        )
        .await
        .expect("the rival has a pen row");
    (r.get(0), r.get(1))
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
    // Served AFTER the rival: the proof that a pen lets the rest of the stream through, which is
    // ADR-0073's whole case for penning rather than freezing. With the rival last, a pen that
    // also froze the cursor would pass every other assertion here.
    let clean_seq = hold_a_clean_event_after(&n).await;

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
    assert_eq!(s.pending, 1, "an unacked substitution makes the pull LOUD");
    assert_eq!(
        s.frozen, None,
        "a penned rival is HELD, so the cursor does not freeze at it: {s:?}"
    );
    assert_eq!(
        cursor(&n).await,
        Some(clean_seq),
        "the event served after the rival was handled and the cursor moved past both"
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
    // Without this, every assertion below would also pass if the second sweep never re-offered
    // the rival at all — "the re-offer must not un-ack" would be true of a re-offer that did not
    // happen.
    assert_eq!(
        second.quarantined, 1,
        "the rival WAS re-offered and deduped onto the acked row"
    );
    assert_eq!(
        second.pending, 0,
        "an acked substitution no longer makes the pull loud"
    );
    assert_eq!(
        pen_count(&n.a).await,
        1,
        "no second row: the re-offer deduped onto the first"
    );
    assert!(
        is_acked(&n, &rival).await,
        "the re-offer must not un-ack a human decision"
    );
    n.serve.abort();
}

/// An UNACKED substitution is never released by a later sweep. The genuine event under the
/// contested id re-applies cleanly on every full sweep, and the auto-release on a clean apply
/// deletes by the APPLIED bytes' address — so it can never reach the rival's row. A release keyed
/// on the event id or on the peer would; this pins that nothing but an ack quiets the row.
///
/// Counting rows is not enough to see that. The rival is served AFTER the genuine event, so a
/// release that wrongly deleted its row would be followed, in the same sweep, by the rival being
/// re-offered and penned afresh: one unacked row again, the pull still loud — and the original
/// row's forensics (`first_seen`, `refused_seq`, how often it was seen) gone. Mutation M15 did
/// exactly that and survived the row count. So the assertion is that it is the SAME row: its
/// `first_seen` is unchanged and its `seen_count` was bumped by the re-offer, not reset.
#[tokio::test]
async fn an_unacked_substitution_survives_the_next_sweep() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let n = self_node(&base, "127.0.0.1:7956").await;
    let signer = n.sk.clone();
    let (_held, rival) = hold_then_serve_a_rival(&n, Uuid::now_v7(), &signer).await;

    let first = full_pull(&base, &n).await;
    assert_eq!(first.quarantined, 1, "penned on the first sweep");
    let (first_seen, seen_once) = row_history(&n, &rival).await;
    assert_eq!(seen_once, 1, "a fresh pen row has been seen once");

    let second = full_pull(&base, &n).await;

    assert!(
        second.admitted >= 1,
        "the genuine event re-applied on the second sweep — the path a mis-keyed auto-release \
         would take: {second:?}"
    );
    assert_eq!(
        pen_count(&n.a).await,
        1,
        "the rival's row is still in the pen"
    );
    assert!(!is_acked(&n, &rival).await, "and still unacked");
    assert_eq!(second.pending, 1, "so the pull is still LOUD");
    assert_eq!(
        row_history(&n, &rival).await,
        (first_seen, 2),
        "and it is the SAME row — first seen when it was penned, and bumped by the re-offer — not \
         a row deleted by the genuine event's re-apply and penned afresh"
    );
    n.serve.abort();
}

/// PR #623 review, finding 1. A rival whose `event_id` is the held id SPELLED differently — a
/// hyphen after every four hex digits, which Postgres's `::uuid` reads as the same UUID — is still
/// a rival under a held id, and still penned. Before the fix the loop's lookup used the narrower
/// `uuid` crate grammar, found "nothing held", and skipped it: whoever minted a rival could switch
/// the pen off by choosing a spelling. `node_event_id_spellings.rs` pins the parsers' agreement.
#[tokio::test]
async fn a_rival_under_an_oddly_spelled_held_id_is_still_penned() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let n = self_node(&base, "127.0.0.1:7957").await;
    let contested = Uuid::now_v7();
    let subject = node_id_hex(1);
    let held = peer_event(&n.sk, "peer.added", contested, &subject);
    call(&n.a, "submit_node_event", &held)
        .await
        .expect("A holds the genuine event");
    let spelling = spelled_oddly(contested);
    assert!(
        Uuid::parse_str(&spelling).is_err(),
        "the fixture must use a spelling the `uuid` crate rejects, or this test proves nothing"
    );
    let rival = peer_event_spelled(&n.sk, "peer.revoked", &spelling, &subject);
    serve_raw(&n.a, &rival).await;

    let s = full_pull(&base, &n).await;

    assert_eq!(
        s.quarantined, 1,
        "the oddly spelled rival is PENNED, not skipped: {s:?}"
    );
    assert_eq!(s.rejected, 0, "and not filed under self-healing: {s:?}");
    assert!(pen_reason(&n, &rival).await.starts_with("substitution:"));
    assert_eq!(
        held_address(&n.a, contested).await,
        Some(address_of(&held)),
        "the genuine event is untouched"
    );
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

/// The FALSE-POSITIVE direction. A refused event this node already holds with the SAME bytes is
/// not a substitution — it is the same event again — so it is SKIPPED, never penned.
///
/// This is the routine case, not an edge. A node does not peer with itself, so its OWN events,
/// echoed back by every peer that pulled them, are refused by the trust checks on each full sweep
/// — and so is every event of a peer it has revoked, re-served by a peer it still trusts. It
/// already holds all of them, with the same bytes. Penning them would hold the INTEGRITY line on
/// for good and fill the pen quota: the flood of steady-state refusals #268 warns against.
///
/// The self-pull reproduces it: node A revokes ITS OWN self-peer, so each event A holds (the
/// genesis, the self-`peer.added`, the revocation itself) comes back, is refused by a trust check
/// (the genesis by the enroll arm's, the other two by the author check), and is found held under
/// the same content address.
///
/// The TLS config is built BEFORE the revocation: the trust store is a snapshot, and one taken
/// after it would no longer pin A's own key, so the handshake — not the classification under
/// test — would fail.
#[tokio::test]
async fn a_refusal_of_an_event_held_with_the_same_bytes_is_skipped_not_penned() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let n = self_node(&base, "127.0.0.1:7955").await;
    let cfg = sync::client_config(&base, &n.sk, sync::trust_store_from_db(&n.a).await.unwrap())
        .await
        .unwrap();
    let me = identity::load_local(&n.a).await.unwrap().node_id_hex;
    identity::author_unpeer(&n.a, &n.sk, &key_hex(&n.sk), &me, &me)
        .await
        .expect("A revokes its own self-peer through the real submit door");

    let s = sync::pull_once(n.addr, cfg, true).await.unwrap();

    // The claim first, so a regression fails at the assertion that names it. The anti-vacuity
    // check after it still fails on its own if the fixture ever stops producing refusals.
    assert_eq!(
        s.quarantined, 0,
        "held with the SAME bytes is not a substitution: nothing is penned: {s:?}"
    );
    assert_eq!(pen_count(&n.a).await, 0, "and the pen stays empty");
    assert!(
        s.rejected >= 1,
        "the fixture must actually produce refusals, or the assertions above are vacuous: {s:?}"
    );
    assert_eq!(
        s.rejected, s.received,
        "every event A holds was refused (its author is no longer an active peer) and \
         skipped: {s:?}"
    );
    assert_eq!(s.pending, 0, "so the pull is not loud");
    assert_eq!(s.frozen, None, "and the cursor did not freeze");
    n.serve.abort();
}
