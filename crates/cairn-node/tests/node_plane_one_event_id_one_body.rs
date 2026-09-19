//! #619 — one `node_event_id`, one body, through the two LIVE node-plane doors (db/007).
//!
//! The sibling of `restore_one_node_event_id_one_body.rs` (db/009, #615). A SUBSTITUTION is a
//! second, different event filed under an `event_id` the log already holds. Every door inserts
//! `ON CONFLICT (node_event_id) DO NOTHING` so a repeat of the SAME event stays a silent no-op —
//! set-union, principle 1 — and a substitution looks exactly like that no-op from the INSERT's side.
//! Before #619, db/007 compared nothing, so the rival vanished and the door returned the id as if
//! it had succeeded.
//!
//! What that cost, per door (stated precisely — ADR-0073 corrects #619's own "A keeps trusting C",
//! which cannot happen because `trust_peer` reads only events THIS node authored):
//!
//! * `submit_node_event` (local): a revocation this node authors under an id it already holds is
//!   dropped, and the node keeps trusting a peer it revoked — #615's shape, on the door that
//!   authors peering. Reaching it needs this node's signing key.
//! * `apply_remote_node_event` (the federation admission gate): a peer's rival is dropped, the
//!   puller counts it admitted and advances past it, and the two nodes hold different bytes under
//!   one id forever. A dropped rival GENESIS is the sharpest case: that peer's key then never
//!   resolves here, and every event it authors is refused.
//!
//! Every guarded ARM gets its own rival case, because the guard sits once in a shared tail and a
//! later edit could route one arm around it. Every door also gets an IDEMPOTENCE case — the same
//! event twice must still succeed — because a guard that refused a repeat would break set-union
//! itself; those cases pass before the guard exists and are what catch a guard moved ABOVE the
//! branch (where nothing is held yet, so it would refuse every clean write).

#[path = "common/node_plane_kit.rs"]
mod node_plane_kit;

use cairn_node::db;
use node_plane_kit::{
    address_of, call, cs, fresh_node, held_address, node_id_hex, peer_event, supersede_event,
    SENTENCE,
};
use uuid::Uuid;

/// THE LOCAL CASE. A revocation this node authors under an id it already holds must be refused
/// LOUDLY. Before #619 the door returned success, the revocation vanished, and `trust_peer` went
/// on showing the peer as active with nothing telling anyone.
#[tokio::test]
async fn the_local_door_refuses_a_rival_revocation_instead_of_dropping_it() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let a = fresh_node(&base).await;

    let contested = Uuid::now_v7();
    let peer = node_id_hex(1);
    let added = peer_event(&a.sk, "peer.added", contested, &peer);
    let revoked = peer_event(&a.sk, "peer.revoked", contested, &peer);

    call(&a.db, "submit_node_event", &added)
        .await
        .expect("the first event under a fresh id is admitted");
    let msg = call(&a.db, "submit_node_event", &revoked).await.expect_err(
        "a SECOND, different node event under a held id must be REFUSED. Returning success here \
         is #619: the revocation vanishes and this node keeps trusting the peer, in silence",
    );
    assert!(
        msg.contains("submit_node_event") && msg.contains(SENTENCE),
        "the refusal must name its door and say it was a substitution, so the operator can tell \
         it from a signature or trust failure; got: {msg}"
    );
    assert_eq!(
        held_address(&a.db, contested).await,
        Some(address_of(&added)),
        "the event first written under the id is untouched"
    );
}

/// The supersede arm reaches the same tail by a different branch, so it gets its own rival.
#[tokio::test]
async fn the_local_door_refuses_a_rival_supersede() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let a = fresh_node(&base).await;

    let contested = Uuid::now_v7();
    let held = supersede_event(&a.sk, contested, &node_id_hex(2));
    let rival = supersede_event(&a.sk, contested, &node_id_hex(3));

    call(&a.db, "submit_node_event", &held)
        .await
        .expect("the first supersede under a fresh id is admitted");
    let msg = call(&a.db, "submit_node_event", &rival)
        .await
        .expect_err("a rival supersede under a held id must be refused, not dropped");
    assert!(
        msg.contains("submit_node_event") && msg.contains(SENTENCE),
        "got: {msg}"
    );
    assert_eq!(held_address(&a.db, contested).await, Some(address_of(&held)));
}

/// A REPEAT is not a substitution. Green before the guard exists and green after — its job is to
/// fail if the guard is ever placed where it would refuse a clean or repeated write.
#[tokio::test]
async fn the_local_door_still_admits_the_same_event_twice() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let a = fresh_node(&base).await;

    let peer = peer_event(&a.sk, "peer.added", Uuid::now_v7(), &node_id_hex(4));
    let supersede = supersede_event(&a.sk, Uuid::now_v7(), &node_id_hex(5));
    for pass in 1..=2 {
        for ev in [&peer, &supersede] {
            call(&a.db, "submit_node_event", ev)
                .await
                .unwrap_or_else(|e| {
                    panic!("pass {pass}: the SAME event twice must stay a no-op, never raise: {e}")
                });
        }
    }
}
