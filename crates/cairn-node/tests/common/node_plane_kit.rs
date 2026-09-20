//! Node-plane fixtures shared by the #619 suites and `node_quarantine.rs`.
//!
//! Two kinds of thing live here:
//!
//! * **Signed node events under a CALLER-CHOSEN `event_id`.** Production mints a fresh UUIDv7 for
//!   every node event (`identity::node_event_body`), so the only way to build a SUBSTITUTION — a
//!   second, different event under an id the log already holds — is to choose the id. Everything
//!   else about two rivals differs (type or payload), which is what makes their content addresses
//!   differ and the pair a substitution rather than a repeat.
//! * **The single-DB self-pull** (#111). Node A serves its own `node_event` log to itself over pinned
//!   mTLS and pulls it back, so a row raw-inserted into A's log is streamed, received and re-applied
//!   through the REAL admission gate — `pull_into`'s classification exercised end-to-end without a
//!   second database. It lived in `node_quarantine.rs` until #619's pull suite needed it too.
//!
//! Include with `#[path = "common/node_plane_kit.rs"] mod node_plane_kit;`.
#![allow(dead_code)] // each including suite uses a different subset

use cairn_event::{
    event_address, generate_key, sign, ClockGrade, EventBody, Hlc, PairingBundle, SigningKey,
};
use cairn_node::{db, identity, keystore, sync};
use std::net::SocketAddr;
use tokio_postgres::Client;
use uuid::Uuid;

/// The tail of the refusal every write door raises through `cairn_refuse_substitution` (db/053).
/// The door's own name is interpolated IN FRONT of it, so this tail is what a test can match.
pub const SENTENCE: &str = "already exists with different content (substitution refused)";

/// The test database, or `None` when `$CAIRN_TEST_PG` is unset — the repo-wide self-skip,
/// policed by `tests/db_gate_actually_ran.rs`.
pub fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// The hex public key — what every node event carries as `signer_key_id`, and what
/// `generate_key` / `keystore::generate_plaintext` return as the key id.
pub fn key_hex(sk: &SigningKey) -> String {
    hex::encode(sk.verifying_key().to_bytes())
}

/// The content address of signed bytes — byte-identical to db/007's
/// `'\x1220' || digest(p_signed, 'sha256')`.
pub fn address_of(signed: &[u8]) -> Vec<u8> {
    event_address(signed)
}

/// The node id a genesis DEFINES: the hex of its own content address (db/007 stores
/// `node_id = v_ca`), so a pairing bundle can name a node before its genesis is ever admitted.
pub fn node_id_of(genesis: &[u8]) -> String {
    hex::encode(event_address(genesis))
}

/// A 32-byte node id as hex, for an event's SUBJECT (a peer to add, a node superseded).
///
/// Derived at runtime (house rule 6a) and discriminated by `lineage`, never `seed`/`salt`/`nonce`
/// (6b): nothing here is cryptographic — the doors only decode it.
pub fn node_id_hex(lineage: u8) -> String {
    hex::encode(
        (0..32u8)
            .map(|i| i.wrapping_mul(11).wrapping_add(lineage))
            .collect::<Vec<u8>>(),
    )
}

/// Sign a node event under a CALLER-CHOSEN `event_id`. The single builder the three below share.
///
/// `wall` stays tiny (1–3 ms since the epoch): far below the remote door's clock-drift ceiling
/// (#102), so no test here is refused for its clock rather than for what it tests.
pub fn node_event(
    sk: &SigningKey,
    event_type: &str,
    event_id: Uuid,
    wall: i64,
    payload: serde_json::Value,
) -> Vec<u8> {
    node_event_spelled(sk, event_type, &event_id.to_string(), wall, payload)
}

/// [`node_event`] with the `event_id` given as TEXT, spelled however the caller likes. The signed
/// body carries the id as a free string and the doors read it with Postgres's `::uuid`, which
/// accepts more spellings than the canonical one — see [`spelled_oddly`].
pub fn node_event_spelled(
    sk: &SigningKey,
    event_type: &str,
    event_id: &str,
    wall: i64,
    payload: serde_json::Value,
) -> Vec<u8> {
    node_event_with_hlc(sk, event_type, event_id, wall, 0, payload)
}

/// [`node_event_spelled`] with the HLC's COUNTER chosen too.
///
/// Every other builder here leaves the counter at 0, because only the wall matters to the doors'
/// drift ceiling. The counter becomes interesting exactly once: `node_event_hlc_nonneg` is a CHECK
/// over BOTH fields (#621), so a suite proving the door refuses a negative clock legibly has to be
/// able to make each half negative on its own.
pub fn node_event_with_hlc(
    sk: &SigningKey,
    event_type: &str,
    event_id: &str,
    wall: i64,
    counter: i32,
    payload: serde_json::Value,
) -> Vec<u8> {
    let kid = key_hex(sk);
    let body = EventBody {
        event_id: event_id.to_string(),
        patient_id: identity::NIL_PATIENT.into(),
        event_type: event_type.into(),
        schema_version: "node/1".into(),
        hlc: Hlc {
            wall,
            counter,
            node_origin: kid.clone(),
        },
        t_effective: None,
        signer_key_id: kid.clone(),
        contributors: serde_json::json!([{ "actor_id": kid, "role": "recorded" }]),
        payload,
        attachments: vec![],
        plaintext_twin: None,
        clock_grade: ClockGrade::SelfAsserted,
        safety: None,
    };
    sign(&body, sk).unwrap().signed_bytes
}

/// A `node.enrolled` genesis under a chosen id. Its content address IS the node's id.
pub fn genesis_event(sk: &SigningKey, event_id: Uuid, name: &str) -> Vec<u8> {
    node_event(
        sk,
        "node.enrolled",
        event_id,
        1,
        serde_json::json!({ "display_name": name, "address": "127.0.0.1:7999" }),
    )
}

/// A `peer.added` / `peer.revoked` about `subject_hex`, under a chosen id.
pub fn peer_event(sk: &SigningKey, event_type: &str, event_id: Uuid, subject_hex: &str) -> Vec<u8> {
    node_event(
        sk,
        event_type,
        event_id,
        2,
        serde_json::json!({ "peer_node_id_hex": subject_hex, "role": "peer" }),
    )
}

/// [`peer_event`] under an id given as TEXT — for a rival whose id is spelled [`spelled_oddly`].
pub fn peer_event_spelled(
    sk: &SigningKey,
    event_type: &str,
    event_id: &str,
    subject_hex: &str,
) -> Vec<u8> {
    node_event_spelled(
        sk,
        event_type,
        event_id,
        2,
        serde_json::json!({ "peer_node_id_hex": subject_hex, "role": "peer" }),
    )
}

/// `id` spelled with a hyphen after EVERY group of four hex digits
/// (`a0ee-bc99-9c0b-4ef8-bb6d-6bb9-bd38-0a11`). Postgres's `::uuid` reads it as `id`; the `uuid`
/// crate's `parse_str` rejects it — the gap PR #623's review found (finding 1).
pub fn spelled_oddly(id: Uuid) -> String {
    let simple = id.simple().to_string();
    simple
        .as_bytes()
        .chunks(4)
        .map(|c| std::str::from_utf8(c).unwrap())
        .collect::<Vec<_>>()
        .join("-")
}

/// A `node.superseded` naming `superseded_hex`, under a chosen id. Separate from [`peer_event`]
/// because the supersede arm reads `superseded_node_id_hex`, not `peer_node_id_hex`.
pub fn supersede_event(sk: &SigningKey, event_id: Uuid, superseded_hex: &str) -> Vec<u8> {
    node_event(
        sk,
        "node.superseded",
        event_id,
        3,
        serde_json::json!({ "superseded_node_id_hex": superseded_hex }),
    )
}

/// Call a node-plane door (`submit_node_event`, `apply_remote_node_event`, `restore_node_event`)
/// and return its refusal message, if it refused. `door` is always a constant in these suites.
pub async fn call(c: &Client, door: &str, signed: &[u8]) -> Result<(), String> {
    c.execute(&format!("SELECT {door}($1)"), &[&signed])
        .await
        .map(|_| ())
        .map_err(|e| {
            e.as_db_error()
                .map(|d| d.message().to_string())
                .unwrap_or_else(|| e.to_string())
        })
}

/// A door's refusal, with the part a PROGRAM reads beside the part a human reads.
///
/// [`call`] returns the message alone, which is all a substitution test needs. #621 needs the
/// SQLSTATE as well: the node puller routes on it (P0001 = a deliberate verdict, skip-and-advance;
/// anything else = not a verdict), so a refusal carrying the wrong code is a wedged sync link even
/// though its sentence reads perfectly.
pub struct DoorRefusal {
    pub sqlstate: String,
    pub message: String,
}

/// Call a door expecting it to REFUSE, and return both halves of the refusal. Panics if the door
/// accepted — a test that meant to see a refusal and saw an admission has learned nothing.
pub async fn refusal(c: &Client, door: &str, signed: &[u8]) -> DoorRefusal {
    let e = c
        .execute(&format!("SELECT {door}($1)"), &[&signed])
        .await
        .expect_err("the door must REFUSE this event");
    let db = e
        .as_db_error()
        .unwrap_or_else(|| panic!("{door} failed without a database error at all: {e}"));
    DoorRefusal {
        sqlstate: db.code().code().to_string(),
        message: db.message().to_string(),
    }
}

/// What `node_event` holds under `event_id` — its content address — or `None`.
pub async fn held_address(c: &Client, event_id: Uuid) -> Option<Vec<u8>> {
    c.query_opt(
        "SELECT content_address FROM node_event WHERE node_event_id = $1::text::uuid",
        &[&event_id.to_string()],
    )
    .await
    .unwrap()
    .map(|r| r.get(0))
}

/// A freshly provisioned node, reset first, with no serve listener.
pub struct FreshNode {
    pub db: Client,
    pub sk: SigningKey,
    /// Hex of this node's own genesis content address.
    pub node_id: String,
}

/// Provision node A through the real `submit_node_event` genesis arm.
pub async fn fresh_node(base: &str) -> FreshNode {
    let db = db::connect_and_load_schema(base).await.unwrap();
    db::reset_node_federation_tables(&db).await.expect(
        "the fixture reset must succeed — swallowing it with .ok() is the shape behind the #296 \
         pollution lessons: a leftover local_node would fence the doors closed and every \
         assertion would then fail for the wrong reason",
    );
    let (sk, kid) = generate_key().unwrap();
    let node_id = identity::provision(&db, &sk, &kid, "A", "127.0.0.1:7999")
        .await
        .unwrap();
    FreshNode { db, sk, node_id }
}

/// Make `a` trust the node whose genesis is `peer_genesis` (signed by `peer_sk`): `a` authors a
/// `peer.added` through the real submit door, which is what `apply_remote_node_event`'s trust
/// checks read (`trust_peer` shows only events THIS node authored).
pub async fn trust(a: &FreshNode, peer_genesis: &[u8], peer_sk: &SigningKey) {
    let pubkey_hex = key_hex(peer_sk);
    let bundle = PairingBundle {
        node_id_hex: node_id_of(peer_genesis),
        fingerprint: cairn_event::short_fingerprint(&pubkey_hex).unwrap(),
        pubkey_hex,
        address: "127.0.0.1:7998".into(),
        // Runtime-derived (house rule 6a): `nonce` is a CodeQL sink NAME, and this field is inert
        // (#530) — nothing reads it back — so any per-run value will do.
        nonce: format!("fixture-{}", Uuid::now_v7()),
        hlc: Hlc {
            wall: 0,
            counter: 0,
            node_origin: a.node_id.clone(),
        },
    };
    identity::author_peer(
        &a.db,
        &a.sk,
        &key_hex(&a.sk),
        &a.node_id,
        &bundle,
        Some("peer"),
    )
    .await
    .unwrap();
}

// ---------------------------------------------------------------------------
// The single-DB self-pull (moved from node_quarantine.rs).
// ---------------------------------------------------------------------------

/// Provision node A, self-peer it (so A trusts A for the mutual-mTLS self-pull),
/// bind a serve listener, and return everything a self-pull needs.
pub struct SelfNode {
    pub a: Client,
    pub addr: SocketAddr,
    pub serve: tokio::task::JoinHandle<anyhow::Result<()>>,
    pub sk: SigningKey,
    _tmp: tempfile::TempDir,
}

pub async fn self_node(base: &str, listen_addr: &str) -> SelfNode {
    let a = db::connect_and_load_schema(base).await.unwrap();
    db::reset_node_federation_tables(&a).await.expect(
        "the fixture reset must succeed — swallowing it with .ok() is the shape behind the #296 \
         pollution lessons: a leftover local_node would fence the doors closed and every \
         assertion would then fail for the wrong reason",
    );
    let tmp = tempfile::tempdir().unwrap();
    let (sk, kid) = keystore::generate_plaintext(&tmp.path().join("a.key")).unwrap();
    identity::provision(&a, &sk, &kid, "A", listen_addr)
        .await
        .unwrap();
    let id = identity::load_local(&a).await.unwrap();
    // Self-peer so the mutual-mTLS handshake pins A's own key as trusted.
    let self_bundle = PairingBundle {
        node_id_hex: id.node_id_hex.clone(),
        pubkey_hex: id.pubkey_hex.clone(),
        address: listen_addr.into(),
        fingerprint: cairn_event::short_fingerprint(&id.pubkey_hex).unwrap(),
        // Runtime-derived (house rule 6a) — see `trust` above.
        nonce: format!("fixture-{}", Uuid::now_v7()),
        hlc: Hlc {
            wall: 0,
            counter: 0,
            node_origin: id.node_id_hex.clone(),
        },
    };
    identity::author_peer(&a, &sk, &kid, &id.node_id_hex, &self_bundle, Some("peer"))
        .await
        .unwrap();
    let trust = sync::trust_store_from_db(&a).await.unwrap();
    let listen: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let (addr, serve_cfg) = sync::bind_serve(listen, base, &sk, trust).await.unwrap();
    let serve = tokio::spawn(sync::serve(serve_cfg));
    SelfNode {
        a,
        addr,
        serve,
        sk,
        _tmp: tmp,
    }
}

/// How many rows the node quarantine pen holds (acked or not).
pub async fn pen_count(a: &Client) -> i64 {
    a.query_one("SELECT count(*) FROM node_event_quarantine", &[])
        .await
        .unwrap()
        .get(0)
}

/// Raw-insert `signed` as a row A will SERVE, under a fresh random `node_event_id` — the stand-in
/// for a peer serving these bytes. Owner privilege bypasses the grant floor, as in every #111 test.
///
/// The row's table id deliberately differs from the `event_id` inside its signed body: the puller
/// re-applies the BODY, so this is how a rival under an id A already holds can be served beside the
/// genuine row (the table cannot hold two rows under one `node_event_id`). Returns the row's `seq`.
pub async fn serve_raw(a: &Client, signed: &[u8]) -> i64 {
    a.query_one(
        "INSERT INTO node_event
             (node_event_id, op, author_node_id, subject_node_id, signer_key_id,
              hlc_wall, hlc_counter, node_origin, signed_bytes, content_address)
         VALUES (gen_random_uuid(), 'peer', '\\x00', '\\x00', 'k', 1, 0, 'n',
                 $1::bytea, '\\x1220'::bytea || digest($1::bytea, 'sha256'))
         RETURNING seq",
        &[&signed],
    )
    .await
    .expect("owner may seed a served row")
    .get(0)
}
