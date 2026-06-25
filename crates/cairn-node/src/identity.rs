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

/// Advance this node's local HLC in the DB and return the new (wall, counter). The
/// clock lives in Postgres (fat-Postgres, ADR-0001) so a single authority orders all
/// authored events; the Rust side just reads the next stamp before it signs (the HLC
/// is inside the signed body, so it MUST be obtained before `sign`).
///
/// INVARIANT — authoring is single-threaded on a node. The tick → `sign` → `submit`
/// here are three separate DB round-trips, NOT one transaction: the `FOR UPDATE` in
/// `node_hlc_tick` serializes the tick itself, but it does not bind the resulting
/// stamp to the later `submit` (and hence to the `node_event.seq` assigned at INSERT).
/// So if two authors ran concurrently on one node, HLC order and seq order could
/// diverge. This is harmless for set-union convergence today (seq is the sync cursor;
/// HLC is the displayed clock), and node authoring IS effectively single-threaded —
/// but do NOT parallelize authoring without making tick+submit one transaction first,
/// or the HLC↔seq correspondence silently breaks.
async fn next_hlc(db: &Client) -> anyhow::Result<(i64, i32)> {
    let row = db.query_one("SELECT wall, counter FROM node_hlc_tick()", &[]).await?;
    Ok((row.get("wall"), row.get("counter")))
}

/// Author the genesis node.enrolled, submit it, return node_id (hex of its content-address).
pub async fn provision(db: &Client, sk: &SigningKey, key_id: &str, display_name: &str, address: &str)
    -> anyhow::Result<String> {
    let (wall, counter) = next_hlc(db).await?;
    let body = node_event_body("node.enrolled", key_id, display_name, wall, counter,
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
    let (wall, counter) = next_hlc(db).await?;
    let body = node_event_body("peer.added", key_id, node_origin, wall, counter, serde_json::json!({
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
    let (wall, counter) = next_hlc(db).await?;
    let body = node_event_body("peer.revoked", key_id, node_origin, wall, counter, serde_json::json!({
        "peer_node_id_hex": peer_node_id_hex,
    }));
    let signed = sign(&body, sk)?;
    let bytes = signed.signed_bytes.clone();
    db.execute("SELECT submit_node_event($1)", &[&bytes]).await?;
    Ok(hex::encode(event_address(&signed.signed_bytes)))
}

/// Author a `node.superseded` event and submit it (ADR-0026 slice C).
///
/// A restored node mints a fresh keypair (the old signing key is never backed up) and
/// records that its new identity SUCCEEDS the dead node. This is the actor-algebra
/// `supersede` applied to nodes: the dead node's past events stay signature-verifiable
/// forever, but the new node cannot sign as the old one — a destroyed node is a new
/// physical trust boundary. Authored by THIS (new) node's key, like peer/revoke.
pub async fn author_supersede(
    db: &Client,
    sk: &SigningKey,
    key_id: &str,
    node_origin: &str,
    old_node_id_hex: &str,
) -> anyhow::Result<String> {
    let (wall, counter) = next_hlc(db).await?;
    let body = node_event_body("node.superseded", key_id, node_origin, wall, counter,
        serde_json::json!({ "superseded_node_id_hex": old_node_id_hex }));
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
    /// At-rest key escrow status (ADR-0026). "recovery code set …" when a sealed
    /// dual-recipient key is present; "STUBBED …" otherwise.
    pub dr_escrow: String,
    /// `true` iff the at-rest key carries an off-node recovery wrap (ADR-0026 escrow).
    /// `false` for plaintext keys and any key sealed without a dual-recipient wrap.
    pub recovery_escrow: bool,
    /// Backup freshness (ADR-0026 point 7): the last successful backup's age, size and
    /// location, or "never — running without a net" when the node has no backup-health
    /// sidecar. Node-local operational state read from a `backup-status.json` sibling of
    /// the key file — never an event, never signed, never replicated. Degrades to the
    /// "never" warning when absent/unreadable (the fail-safe reading: it can only
    /// under-claim freshness, never assert a backup the node does not hold).
    pub last_backup: String,
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

    // At-rest posture, inspected WITHOUT the secret (a sealed key cannot be loaded
    // here — we have no passphrase in `status` — so we classify the file instead).
    let kstate = crate::keystore::key_at_rest_state(key_path);
    use crate::keystore::KeyAtRest;
    let keystore_ok = matches!(kstate, KeyAtRest::Sealed { .. } | KeyAtRest::Plaintext);
    // One classification site derives ALL THREE escrow-related fields together, so the
    // human strings and the `recovery_escrow` bool can never drift out of agreement
    // (the redundancy a split if/else would invite). `recovery_escrow` is true ONLY for
    // a sealed bundle with a structurally-intact recovery wrap.
    const STUB: &str = "STUBBED (ADR-0026): no recovery escrow; key loss = node loss";
    let (key_at_rest, dr_escrow, recovery_escrow) = match kstate {
        KeyAtRest::Sealed { dual_recipient } => (
            format!("SEALED (argon2id + xchacha20poly1305{})",
                    if dual_recipient { "; dual-recipient" } else { "" }),
            if dual_recipient {
                "recovery code set (off-node escrow; ADR-0026 slice A)".to_string()
            } else {
                STUB.to_string()
            },
            dual_recipient,
        ),
        KeyAtRest::Plaintext =>
            ("PLAINTEXT (0600; run `cairn-node seal-key`)".to_string(), STUB.to_string(), false),
        KeyAtRest::Missing => ("MISSING".to_string(), STUB.to_string(), false),
        KeyAtRest::Corrupt => ("CORRUPT (unparseable key file)".to_string(), STUB.to_string(), false),
    };

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

    // Backup health (ADR-0026 point 7): read the node-local sidecar beside the key and
    // render an honest freshness line. `SystemTime` is the operational wall-clock (a
    // display value, NOT the clinical HLC). A clock before the epoch (cannot happen on a
    // sane host) falls back to 0, which only makes the age display larger — never unsafe.
    let now_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let health = crate::backup::read_health(&crate::backup::health_path_for(key_path));
    let last_backup = crate::backup::describe_health(now_unix, &health);

    Ok(Status {
        // When un-provisioned, surface a legible sentinel rather than a blank
        // node_id, and flag `initialized=false` so callers can prompt for `init`.
        node_id_hex: id.as_ref().map(|i| i.node_id_hex.clone())
            .unwrap_or_else(|| "(uninitialized — run `cairn-node init`)".into()),
        initialized: id.is_some(),
        peers_active,
        peers_revoked,
        keystore_ok,
        key_at_rest,
        runtime_role,
        db_floor_enforced: !can_insert,
        dr_escrow,
        recovery_escrow,
        last_backup,
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
