use cairn_event::{event_address, short_fingerprint, sign, EventBody, Hlc, PairingBundle, SigningKey};
use std::path::Path;
use tokio_postgres::Client;

pub const NIL_PATIENT: &str = "00000000-0000-0000-0000-000000000000";

pub struct Identity {
    pub node_id_hex: String,
    pub pubkey_hex: String,
    pub fingerprint: String,
    pub address: String,
}

fn node_event_body(event_type: &str, signer_key_id: &str, node_origin: &str,
                   wall: i64, counter: i32, payload: serde_json::Value) -> EventBody {
    EventBody {
        event_id: uuid::Uuid::now_v7().to_string(),
        patient_id: NIL_PATIENT.into(),
        event_type: event_type.into(),
        schema_version: "node/1".into(),
        hlc: Hlc { wall, counter, node_origin: node_origin.into() },
        t_effective: None,
        signer_key_id: signer_key_id.into(),
        contributors: serde_json::json!([{"actor_id": signer_key_id, "role": "device"}]),
        payload,
        attachments: vec![],
    }
}

/// Author the genesis node.enrolled, submit it, return node_id (hex of its content-address).
pub async fn provision(db: &Client, sk: &SigningKey, key_id: &str, display_name: &str, address: &str)
    -> anyhow::Result<String> {
    let body = node_event_body("node.enrolled", key_id, display_name, 0, 0,
        serde_json::json!({"display_name": display_name, "address": address}));
    let signed = sign(&body, sk)?;
    let signed_bytes = signed.signed_bytes.clone();
    db.execute("SELECT submit_node_event($1)", &[&signed_bytes]).await?;
    Ok(hex::encode(event_address(&signed.signed_bytes)))
}

/// Load the local node identity, or `None` if this node has not been provisioned
/// yet (no `local_node` row). Use this where a missing node is a *normal* state to
/// report (e.g. `status` before `init`); use [`load_local`] where it is an error.
pub async fn load_local_opt(db: &Client) -> anyhow::Result<Option<Identity>> {
    let row = db.query_opt(
        "SELECT encode(node_id,'hex') AS node_id_hex, signer_key_id, COALESCE(address,'') AS address
         FROM local_node WHERE id", &[]).await?;
    let Some(row) = row else { return Ok(None) };
    let pubkey_hex: String = row.get("signer_key_id");
    Ok(Some(Identity {
        node_id_hex: row.get("node_id_hex"),
        fingerprint: short_fingerprint(&pubkey_hex)?,
        pubkey_hex,
        address: row.get("address"),
    }))
}

/// Load the local node identity; errors if the node has not been provisioned.
/// The pairing/identity/unpeer commands all require an existing node, so a missing
/// `local_node` row is a genuine error there (run `init` first).
pub async fn load_local(db: &Client) -> anyhow::Result<Identity> {
    load_local_opt(db)
        .await?
        .ok_or_else(|| anyhow::anyhow!("node not provisioned: run `cairn-node init` first"))
}

// ---------------------------------------------------------------------------
// Peering authorship
// ---------------------------------------------------------------------------

/// A row from the `trust_peer` view.
pub struct PeerRow {
    pub peer_node_id_hex: String,
    pub fingerprint: String,
    pub role: Option<String>,
    pub status: String,
}

/// Author a `peer.added` event and submit it.
///
/// The payload includes `peer_node_id_hex` (required by the submit door),
/// `peer_pubkey`, `fingerprint`, and the optional `role`.
pub async fn author_peer(
    db: &Client,
    sk: &SigningKey,
    key_id: &str,
    node_origin: &str,
    peer: &PairingBundle,
    role: Option<&str>,
) -> anyhow::Result<String> {
    let body = node_event_body("peer.added", key_id, node_origin, 0, 0, serde_json::json!({
        "peer_node_id_hex": peer.node_id_hex,
        "peer_pubkey":      peer.pubkey_hex,
        "fingerprint":      peer.fingerprint,
        "role":             role,
    }));
    let signed = sign(&body, sk)?;
    let bytes = signed.signed_bytes.clone();
    db.execute("SELECT submit_node_event($1)", &[&bytes]).await?;
    Ok(hex::encode(event_address(&signed.signed_bytes)))
}

/// Author a `peer.revoked` event and submit it.
///
/// The payload includes `peer_node_id_hex` (required by the submit door).
pub async fn author_unpeer(
    db: &Client,
    sk: &SigningKey,
    key_id: &str,
    node_origin: &str,
    peer_node_id_hex: &str,
) -> anyhow::Result<String> {
    let body = node_event_body("peer.revoked", key_id, node_origin, 0, 0, serde_json::json!({
        "peer_node_id_hex": peer_node_id_hex,
    }));
    let signed = sign(&body, sk)?;
    let bytes = signed.signed_bytes.clone();
    db.execute("SELECT submit_node_event($1)", &[&bytes]).await?;
    Ok(hex::encode(event_address(&signed.signed_bytes)))
}

// ---------------------------------------------------------------------------
// Node status
// ---------------------------------------------------------------------------

/// A snapshot of this node's assembly state, suitable for one-per-line display.
#[derive(Debug)]
pub struct Status {
    pub node_id_hex: String,
    /// `false` iff the node has no genesis enrollment yet (no `local_node` row) —
    /// i.e. `status` was run before `init`. The rest of the struct still populates
    /// (peers are 0, the floor self-check is independent of provisioning), so an
    /// operator gets an honest "uninitialized" reading instead of a crash.
    pub initialized: bool,
    pub peers_active: i64,
    pub peers_revoked: i64,
    /// `true` iff the key file exists and loads as a valid 32-byte Ed25519 seed.
    /// Missing or unreadable key: `false` — honest degradation, not an error.
    pub keystore_ok: bool,
    /// Key-at-rest posture (ADR-0026): v1 is plaintext-0600 — the passphrase the
    /// keystore API accepts is NOT yet honoured (no KDF/seal). Surfaced so an
    /// operator never assumes the seed is encrypted (PR #28 review, finding 3).
    pub key_at_rest: String,
    /// The DB role this status was queried over (`current_user`).
    pub runtime_role: String,
    /// `true` iff the connected role CANNOT raw-INSERT into `node_event` — i.e. the
    /// in-DB submit/admission floor actually binds THIS connection. A superuser or
    /// the table owner yields `false`: the floor exists but is BYPASSABLE by this
    /// connection (run the runtime as the unprivileged `cairn_node` role — e.g. a
    /// login role granted `cairn_node` — to enforce it). PR #28 review, finding 2.
    pub db_floor_enforced: bool,
    /// Hard-coded stub (ADR-0026): no recovery escrow in v1.
    pub dr_escrow: String,
}

/// Assemble the node's current status without erroring on a missing keystore.
///
/// `key_path` is the path to the node's signing-key file.  If the file is
/// absent or corrupt, `keystore_ok` is set to `false` and the rest of the
/// struct is still populated (honest degradation).
pub async fn status(db: &Client, key_path: &Path) -> anyhow::Result<Status> {
    // Load the local node identity for the node_id. A missing row means the node
    // has not been provisioned yet (`status` run before `init`) — report that
    // honestly rather than erroring (HANDOVER: "status crashes if run before init").
    let id = load_local_opt(db).await?;

    // Count peers by status from trust_peer.
    let rows = db.query(
        "SELECT status, count(*) AS cnt FROM trust_peer GROUP BY status",
        &[],
    ).await?;
    let mut peers_active: i64 = 0;
    let mut peers_revoked: i64 = 0;
    for row in &rows {
        let s: String = row.get("status");
        let cnt: i64 = row.get("cnt");
        match s.as_str() {
            "active"  => peers_active  = cnt,
            "revoked" => peers_revoked = cnt,
            _         => {}
        }
    }

    // Keystore health: try to load the key; a missing/invalid file is not an error.
    let keystore_ok = crate::keystore::load(key_path, None).is_ok();

    // In-DB floor self-check: is the submit/admission gate actually unbypassable for
    // THIS connection? `has_table_privilege` returns true for a superuser/owner (who
    // can raw-INSERT around the gate) and false for the `cairn_node` role (INSERT
    // revoked). Surfaced so the "enforced in Postgres" claim is honest at runtime.
    let floor = db
        .query_one(
            "SELECT current_user::text AS role,
                    has_table_privilege(current_user, 'node_event', 'INSERT') AS can_insert",
            &[],
        )
        .await?;
    let runtime_role: String = floor.get("role");
    let can_insert: bool = floor.get("can_insert");

    Ok(Status {
        // When un-provisioned, surface a legible sentinel rather than a blank
        // node_id, and flag `initialized=false` so callers can prompt for `init`.
        node_id_hex: id.as_ref().map(|i| i.node_id_hex.clone())
            .unwrap_or_else(|| "(uninitialized — run `cairn-node init`)".into()),
        initialized: id.is_some(),
        peers_active,
        peers_revoked,
        keystore_ok,
        key_at_rest: "PLAINTEXT (0600; ADR-0026 KDF/seal + escrow pending)".into(),
        runtime_role,
        db_floor_enforced: !can_insert,
        dr_escrow: "STUBBED (ADR-0026): no recovery escrow; key loss = node loss".into(),
    })
}

/// Query the `trust_peer` view and return the current peer set.
pub async fn list_peers(db: &Client) -> anyhow::Result<Vec<PeerRow>> {
    let rows = db.query(
        "SELECT encode(peer_node_id,'hex') AS pid, COALESCE(fingerprint,'') AS fp, role, status
         FROM trust_peer ORDER BY pid",
        &[],
    ).await?;
    Ok(rows.iter().map(|r| PeerRow {
        peer_node_id_hex: r.get("pid"),
        fingerprint:      r.get("fp"),
        role:             r.get("role"),
        status:           r.get("status"),
    }).collect())
}
