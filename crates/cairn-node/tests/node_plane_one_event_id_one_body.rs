//! #619 — one `node_event_id`, one body, through the two LIVE node-plane doors (db/007).
//!
//! The sibling of `restore_one_node_event_id_one_body.rs` (db/009, #615). A SUBSTITUTION is a
//! second, different event filed under an `event_id` the log already holds. Every arm but the local
//! genesis inserts `ON CONFLICT (node_event_id) DO NOTHING` so a repeat of the SAME event stays a
//! silent no-op — set-union, principle 1 — and a substitution looks exactly like that no-op from
//! the INSERT's side.
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

use cairn_event::{generate_key, SigningKey};
use cairn_node::db;
use node_plane_kit::{
    address_of, call, cs, fresh_node, genesis_event, held_address, node_event, node_id_hex,
    peer_event, supersede_event, trust, FreshNode, SENTENCE,
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
    assert_eq!(
        held_address(&a.db, contested).await,
        Some(address_of(&held))
    );
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

// ---------------------------------------------------------------------------
// apply_remote_node_event — the federation admission gate.
// ---------------------------------------------------------------------------

/// Node A, plus a peer B that A trusts and whose genesis A has admitted through the remote door.
/// That is the minimum for B's later events to REACH the guard: without it they are refused
/// earlier, by the deny-all trust checks, and a rival test would pass for the wrong reason.
/// Returns B's key, B's genesis id (the enroll-arm case reuses that id) and B's genesis bytes.
async fn a_with_trusted_b(base: &str) -> (FreshNode, SigningKey, Uuid, Vec<u8>) {
    let a = fresh_node(base).await;
    let (b_sk, _) = generate_key().unwrap();
    let b_genesis_id = Uuid::now_v7();
    let b_genesis = genesis_event(&b_sk, b_genesis_id, "B");
    trust(&a, &b_genesis, &b_sk).await;
    call(&a.db, "apply_remote_node_event", &b_genesis)
        .await
        .expect("A admits the genesis of a peer it trusts");
    (a, b_sk, b_genesis_id, b_genesis)
}

/// A trusted peer serves a rival `peer.revoked` under the id of its own `peer.added`.
#[tokio::test]
async fn the_admission_gate_refuses_a_rival_peer_event() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let (a, b_sk, _, _) = a_with_trusted_b(&base).await;

    let contested = Uuid::now_v7();
    let subject = node_id_hex(6);
    let held = peer_event(&b_sk, "peer.added", contested, &subject);
    let rival = peer_event(&b_sk, "peer.revoked", contested, &subject);

    call(&a.db, "apply_remote_node_event", &held)
        .await
        .expect("B's first event under a fresh id is admitted");
    let msg = call(&a.db, "apply_remote_node_event", &rival)
        .await
        .expect_err(
        "a peer's SECOND, different event under a held id must be refused. Admitting it silently \
         is #619: A and B then hold different bytes under one id, forever, and nothing says so",
    );
    assert!(
        msg.contains("apply_remote_node_event") && msg.contains(SENTENCE),
        "got: {msg}"
    );
    assert_eq!(
        held_address(&a.db, contested).await,
        Some(address_of(&held))
    );
}

/// The supersede arm, through the remote door.
#[tokio::test]
async fn the_admission_gate_refuses_a_rival_supersede() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let (a, b_sk, _, _) = a_with_trusted_b(&base).await;

    let contested = Uuid::now_v7();
    let held = supersede_event(&b_sk, contested, &node_id_hex(7));
    let rival = supersede_event(&b_sk, contested, &node_id_hex(8));

    call(&a.db, "apply_remote_node_event", &held)
        .await
        .expect("B's first supersede under a fresh id is admitted");
    let msg = call(&a.db, "apply_remote_node_event", &rival)
        .await
        .expect_err("a rival supersede under a held id must be refused");
    assert!(
        msg.contains("apply_remote_node_event") && msg.contains(SENTENCE),
        "got: {msg}"
    );
    assert_eq!(
        held_address(&a.db, contested).await,
        Some(address_of(&held))
    );
}

/// THE SHARPEST REMOTE CASE: a rival GENESIS. C is trusted too, and its genesis reuses B's genesis
/// id. Dropped silently, C's genesis would never be stored, `node_current` would never resolve C's
/// key, and every event C authors would be refused as "author key maps to no known node" —
/// logged as recoverable, which it never would be.
#[tokio::test]
async fn the_admission_gate_refuses_a_rival_genesis() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let (a, _b_sk, b_genesis_id, b_genesis) = a_with_trusted_b(&base).await;

    let (c_sk, _) = generate_key().unwrap();
    let c_genesis = genesis_event(&c_sk, b_genesis_id, "C");
    trust(&a, &c_genesis, &c_sk).await;

    let msg = call(&a.db, "apply_remote_node_event", &c_genesis)
        .await
        .expect_err("a trusted node's genesis under an id already held must be refused");
    assert!(
        msg.contains("apply_remote_node_event") && msg.contains(SENTENCE),
        "got: {msg}"
    );
    assert_eq!(
        held_address(&a.db, b_genesis_id).await,
        Some(address_of(&b_genesis)),
        "B's genesis, first under the id, is untouched"
    );
}

/// A REPEAT through the remote door, in every arm, is still admitted — set-union survives the
/// guard. (Green before the guard exists; it catches a guard moved above the branch.)
#[tokio::test]
async fn the_admission_gate_still_admits_the_same_event_twice() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let a = fresh_node(&base).await;
    let (b_sk, _) = generate_key().unwrap();
    let b_genesis = genesis_event(&b_sk, Uuid::now_v7(), "B");
    trust(&a, &b_genesis, &b_sk).await;
    let peer = peer_event(&b_sk, "peer.added", Uuid::now_v7(), &node_id_hex(9));
    let supersede = supersede_event(&b_sk, Uuid::now_v7(), &node_id_hex(10));

    for pass in 1..=2 {
        for ev in [&b_genesis, &peer, &supersede] {
            call(&a.db, "apply_remote_node_event", ev)
                .await
                .unwrap_or_else(|e| {
                    panic!("pass {pass}: the SAME event twice must stay a no-op, never raise: {e}")
                });
        }
    }
}

/// Every arm of the admission gate still merges this node's clock forward past an admitted event
/// (the HLC A3 invariant). #619 replaced the three per-arm `cairn_node_hlc_merge` calls with ONE in
/// the shared tail, after the substitution guard. `hlc_merge_helper.rs` pins that db/007 has
/// exactly one call — but a count cannot see WHERE it sits: moved inside the enroll branch, it
/// would still count one, and the peer and supersede arms would stop merging in silence. So each
/// arm is driven here with a wall ahead of the clock (inside the drift ceiling), and the clock
/// must reach it.
#[tokio::test]
async fn every_arm_of_the_admission_gate_merges_the_clock() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let (a, b_sk, _, _) = a_with_trusted_b(&base).await;
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let hour = 3_600_000;

    // A trusted C whose genesis carries a future wall, so the enroll arm has something to merge.
    let (c_sk, _) = generate_key().unwrap();
    let c_genesis = node_event(
        &c_sk,
        "node.enrolled",
        Uuid::now_v7(),
        now_ms + hour,
        serde_json::json!({ "display_name": "C", "address": "127.0.0.1:7997" }),
    );
    trust(&a, &c_genesis, &c_sk).await;
    let peer = node_event(
        &b_sk,
        "peer.added",
        Uuid::now_v7(),
        now_ms + 2 * hour,
        serde_json::json!({ "peer_node_id_hex": node_id_hex(11), "role": "peer" }),
    );
    let supersede = node_event(
        &b_sk,
        "node.superseded",
        Uuid::now_v7(),
        now_ms + 3 * hour,
        serde_json::json!({ "superseded_node_id_hex": node_id_hex(12) }),
    );

    // Walls rise arm by arm, so each arm must move the clock itself — a merge performed by an
    // earlier arm cannot account for a later one's wall.
    for (arm, event, wall) in [
        ("enroll", &c_genesis, now_ms + hour),
        ("peer", &peer, now_ms + 2 * hour),
        ("supersede", &supersede, now_ms + 3 * hour),
    ] {
        call(&a.db, "apply_remote_node_event", event)
            .await
            .unwrap_or_else(|e| panic!("the {arm} arm admits a trusted event: {e}"));
        let clock: i64 =
            a.db.query_one("SELECT hlc_wall FROM hlc_state WHERE id", &[])
                .await
                .unwrap()
                .get(0);
        assert_eq!(
            clock, wall,
            "the {arm} arm must merge this node's clock forward to the admitted wall"
        );
    }
}
