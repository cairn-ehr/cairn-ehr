//! #621 — every deterministic malformed input is refused with **P0001**, naming field and door.
//!
//! ## The defect
//!
//! The node puller (`sync.rs`, `pull_into`) reads a refusal's SQLSTATE to decide what it means:
//! `P0001` is a door VERDICT (deliberate, deterministic — skip past it and let a later sweep
//! re-offer it), anything else is *"the door never got to decide"* (a deadlock, a timeout, a
//! dropped connection) and FREEZES the cursor below that seq until the next cycle.
//!
//! db/007 and db/009 raised non-`P0001` **deterministically** in four places, on input they will
//! refuse identically forever:
//!
//! | where | SQLSTATE |
//! | --- | --- |
//! | `(b ->> 'event_id')::uuid` — before any trust check | `22P02` |
//! | `NULLIF(payload ->> 'target_event_id','')::uuid` | `22P02` |
//! | the `node_event_hlc_nonneg` CHECK (a negative wall or counter) | `23514` |
//! | the `node_event_role_check` CHECK (a role outside the vocabulary) | `23514` |
//!
//! A frozen cursor never advances, so **every later event on that link is held behind the poison
//! one** — including that peer's own `peer.revoked` — and nothing is penned, so there is no `ack`
//! remedy either. This is #228's class exactly (a bare `decode()` in the 22 class froze a peer's
//! cursor permanently), closed then for hex and not for casts or CHECKs.
//!
//! ## Who can actually trigger it
//!
//! `serve` streams only rows already in the serving peer's own `node_event`, and those passed the
//! same casts and CHECKs — so an honest peer on the same schema cannot produce one. What remains
//! is a misbehaving peer crafting frames (it wedges only its own link, but silently and with no
//! remedy) and, the one that matters under principle 11, **cross-version CHECK-vocabulary skew**:
//! db/009 already widened the `op` CHECK in place, and the day `role` is widened the same way,
//! every older node pulling from a newer peer freezes that link permanently.
//!
//! ## What this suite pins
//!
//! For each of the three signed-bytes doors and each malformed field: the refusal carries the
//! **skip-and-advance code**, and its message names the **door** and the **field**. Both halves
//! matter and neither implies the other — a perfectly worded refusal under `22P02` is a wedged
//! sync link, and a `P0001` that says nothing tells an operator which peer to go and fix.
//!
//! The anti-vacuity half is in the same file on purpose: a well-formed event still applies, and a
//! valid-but-ODD UUID spelling (`spelled_oddly`) is still ACCEPTED. The helper replaces a `::uuid`
//! cast, and a validator NARROWER than the cast it replaces would refuse events the log already
//! holds — the mirror-image defect PR #623's review found in Rust (finding 1).
//!
//! The source-level guards that keep the call sites wired live in
//! `node_door_input_guards.rs`; DB-backed cases here need `$CAIRN_TEST_PG` and take
//! `db::test_serial_guard`.

#[path = "common/node_plane_kit.rs"]
mod node_plane_kit;

use cairn_event::{generate_key, SigningKey};
use cairn_node::db;
use node_plane_kit::{
    call, cs, fresh_node, genesis_event, node_event_spelled, node_event_with_hlc, node_id_hex,
    peer_event, peer_event_spelled, refusal, spelled_oddly, trust,
};
use tokio_postgres::Client;
use uuid::Uuid;

/// The SQLSTATE of a bare `RAISE EXCEPTION` — the contract between every door and the node
/// puller (db/001, above `cairn_decode_hex_or_raise`, #228; db/048 states the clinical half).
const SKIP_AND_ADVANCE: &str = "P0001";

/// The three doors that write `node_event` from signed bytes, and the only three this suite has
/// to cover: `submit_node_event` and `apply_remote_node_event` (db/007) and `restore_node_event`
/// (db/009). The pinned INVENTORY of writers is `substitution_guard_covers_every_writer.rs`'s
/// catalogue rule — this list is the subset that takes caller-supplied bytes.
const DOORS: [&str; 3] = [
    "submit_node_event",
    "apply_remote_node_event",
    "restore_node_event",
];

/// A node prepared so that an event signed by `sk` REACHES the field checks at `door` rather than
/// being refused earlier for a reason that has nothing to do with the field under test.
///
/// The three doors need three different preparations, which is exactly why this is a fixture and
/// not a constant:
///
/// * `submit_node_event` authors locally, so the signer must be THIS node's own key;
/// * `apply_remote_node_event` is the deny-all admission gate, so the signer must be a peer this
///   node trusts AND whose genesis it has already admitted (otherwise the trust checks refuse
///   first and every assertion below passes for the wrong reason);
/// * `restore_node_event` is self-trusting but refuses a node that is already enrolled — it
///   applies only into a FRESH database — so its fixture provisions nothing and instead restores
///   the medium's own genesis first, which is what makes the signer's later events resolve an
///   author through `node_current`.
struct DoorFixture {
    db: Client,
    sk: SigningKey,
    door: &'static str,
}

async fn fixture(base: &str, door: &'static str) -> DoorFixture {
    if door == "restore_node_event" {
        let db = db::connect_and_load_schema(base).await.unwrap();
        db::reset_node_federation_tables(&db).await.expect(
            "the fixture reset must succeed — a leftover local_node fences the restore door \
             closed and every assertion below then fails for the wrong reason (#296)",
        );
        let (sk, _) = generate_key().unwrap();
        call(&db, door, &genesis_event(&sk, Uuid::now_v7(), "Restored"))
            .await
            .expect("the medium's own genesis restores first, so its key resolves");
        return DoorFixture { db, sk, door };
    }
    let a = fresh_node(base).await;
    if door != "apply_remote_node_event" {
        return DoorFixture {
            db: a.db,
            sk: a.sk,
            door,
        };
    }
    let (b_sk, _) = generate_key().unwrap();
    let b_genesis = genesis_event(&b_sk, Uuid::now_v7(), "B");
    trust(&a, &b_genesis, &b_sk).await;
    call(&a.db, "apply_remote_node_event", &b_genesis)
        .await
        .expect("A admits the genesis of a peer it trusts");
    DoorFixture {
        db: a.db,
        sk: b_sk,
        door,
    }
}

/// Assert one door's refusal of one malformed event: the code a program reads, then the words a
/// human reads. `field` is the payload/body field name the message must name.
async fn refuses_legibly(f: &DoorFixture, signed: &[u8], field: &str) {
    let r = refusal(&f.db, f.door, signed).await;
    assert_eq!(
        r.sqlstate, SKIP_AND_ADVANCE,
        "{}: a refusal this door will repeat forever must carry {SKIP_AND_ADVANCE}, the code the \
         node puller reads as a VERDICT. Under any other code the puller cannot tell it from a \
         deadlock, so it freezes that peer's cursor below this seq — permanently, holding back \
         every later event on the link. Message was: {}",
        f.door, r.message
    );
    assert!(
        r.message.contains(f.door) && r.message.contains(field),
        "{}: the refusal must name its door and the field ({field}), so an operator knows which \
         peer to go and fix; got: {}",
        f.door,
        r.message
    );
}

#[tokio::test]
async fn a_non_uuid_event_id_is_refused_with_the_skip_and_advance_code() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    for door in DOORS {
        let f = fixture(&base, door).await;
        // `event_id` is an unvalidated String in the signed body, and the cast that reads it runs
        // before any trust check — so this is the one case reachable by a signer the node does
        // not trust at all, on bytes any peer may serve.
        let ev = peer_event_spelled(&f.sk, "peer.added", "not-a-uuid", &node_id_hex(1));
        refuses_legibly(&f, &ev, "event_id").await;
    }
}

#[tokio::test]
async fn a_non_uuid_target_event_id_is_refused_with_the_skip_and_advance_code() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    for door in DOORS {
        let f = fixture(&base, door).await;
        let ev = node_event_spelled(
            &f.sk,
            "peer.revoked",
            &Uuid::now_v7().to_string(),
            2,
            serde_json::json!({
                "peer_node_id_hex": node_id_hex(2),
                "role": "peer",
                "target_event_id": "nope",
            }),
        );
        refuses_legibly(&f, &ev, "target_event_id").await;
    }
}

#[tokio::test]
async fn a_negative_hlc_wall_is_refused_with_the_skip_and_advance_code() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    for door in DOORS {
        let f = fixture(&base, door).await;
        // The doors' drift ceiling bounds the wall from ABOVE only, so a negative wall sails past
        // it and lands on the table's CHECK — `23514`, with no door and no field in the message.
        let ev = node_event_with_hlc(
            &f.sk,
            "peer.added",
            &Uuid::now_v7().to_string(),
            -1,
            0,
            serde_json::json!({ "peer_node_id_hex": node_id_hex(3), "role": "peer" }),
        );
        refuses_legibly(&f, &ev, "hlc").await;
    }
}

#[tokio::test]
async fn a_negative_hlc_counter_is_refused_with_the_skip_and_advance_code() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    for door in DOORS {
        let f = fixture(&base, door).await;
        // The CHECK covers both halves of the clock, so the guard must too: a wall-only check
        // would leave this exact event raising 23514 and freezing the link.
        let ev = node_event_with_hlc(
            &f.sk,
            "peer.added",
            &Uuid::now_v7().to_string(),
            2,
            -1,
            serde_json::json!({ "peer_node_id_hex": node_id_hex(4), "role": "peer" }),
        );
        refuses_legibly(&f, &ev, "hlc").await;
    }
}

#[tokio::test]
async fn an_unknown_peer_role_is_refused_with_the_skip_and_advance_code() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    for door in DOORS {
        let f = fixture(&base, door).await;
        // Today's reachable case is a buggy trusted author. The case that outlives it is version
        // skew: the day the role vocabulary is widened (as db/009 widened `op`), an older node
        // meets the new value here — and under 23514 it would freeze that link rather than skip
        // the event and admit it after its own upgrade.
        let ev = node_event_spelled(
            &f.sk,
            "peer.added",
            &Uuid::now_v7().to_string(),
            2,
            serde_json::json!({ "peer_node_id_hex": node_id_hex(5), "role": "overlord" }),
        );
        refuses_legibly(&f, &ev, "role").await;
    }
}

/// The anti-vacuity half. Every assertion above is about a REFUSAL, and a validator that refused
/// everything would satisfy all of them — including, and this is the one that would actually get
/// written, a UUID validator narrower than the `::uuid` cast it replaces. Postgres accepts
/// spellings the `uuid` crate does not (PR #623 finding 1), and the log may already HOLD events
/// under them.
#[tokio::test]
async fn a_well_formed_event_still_applies_however_its_id_is_spelled() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    for door in DOORS {
        let f = fixture(&base, door).await;
        let canonical = peer_event(&f.sk, "peer.added", Uuid::now_v7(), &node_id_hex(6));
        call(&f.db, f.door, &canonical)
            .await
            .unwrap_or_else(|e| panic!("{door}: a well-formed event must still apply: {e}"));

        let odd = peer_event_spelled(
            &f.sk,
            "peer.added",
            &spelled_oddly(Uuid::now_v7()),
            &node_id_hex(7),
        );
        call(&f.db, f.door, &odd).await.unwrap_or_else(|e| {
            panic!(
                "{door}: Postgres's ::uuid reads this spelling, so the door must too — a narrower \
                 validator would refuse events the log can already hold: {e}"
            )
        });
    }
}
