//! ADR-0026 slice C — restore orchestration (apply a backup medium under a new identity).
//!
//! WHY: the live apply_remote_node_event gate is the PEER-admission path and rejects a
//! node rehydrating its OWN history (no trust set in a fresh DB). Restore therefore uses
//! the self-trusting restore_node_event door (db/009), then mints a fresh key and records
//! a node-level supersede (the old signing key is never backed up). This module holds the
//! PURE helpers (dead-node-id resolution, old-genesis metadata) and the thin DB
//! orchestration; main.rs owns key-minting + recovery-code printing (as `init` does).

use cairn_event::{event_address, verify_self_described};

#[derive(thiserror::Error, Debug)]
pub enum RestoreError {
    #[error("medium has no genesis (node.enrolled) event")]
    NoGenesis,
    #[error("medium carries {0} enrolls; pass --superseded-node <hex> to pick the dead node")]
    Ambiguous(usize),
    #[error("--superseded-node {wanted} names no enroll on this medium (available: {available:?})")]
    UnknownNodeId {
        wanted: String,
        available: Vec<String>,
    },
}

/// Every enroll (node.enrolled) on the medium, as (node_id_hex, body) pairs. A node-id
/// is the content-address of its genesis, so we hash each verified enroll's bytes. Only
/// events that VERIFY are considered (a corrupt enroll cannot name a node).
fn enrolls(events: &[Vec<u8>]) -> Vec<(String, cairn_event::EventBody)> {
    events
        .iter()
        .filter_map(|e| {
            let body = verify_self_described(e).ok()?;
            if body.event_type == "node.enrolled" {
                Some((hex::encode(event_address(e)), body))
            } else {
                None
            }
        })
        .collect()
}

/// Resolve the dead node's id (hex) to supersede on restore.
///
/// - `explicit` (operator's --superseded-node) is normalized to lower hex and then
///   REQUIRED to name an enroll actually present on the medium. The medium IS the dead
///   node's own history, so its genesis enroll is always there; an explicit id that
///   matches nothing is a typo (or the wrong node) and we fail closed HERE, before any
///   key-minting or DB write. This is load-bearing: an unchecked id would otherwise
///   (a) abort deep in submit_node_event's `decode(...,'hex')` AFTER the new genesis was
///   already committed — a half-finalized, un-retryable-in-place node — when malformed,
///   or (b) silently supersede a node that was never restored when well-formed-but-wrong.
/// - else, if the medium has exactly ONE enroll, that is the dead node (the solo-clinic
///   case — ADR-0026's primary deployment).
/// - else it is ambiguous (a federated node whose log holds peers' genesis too) and we
///   fail closed, telling the operator to pass --superseded-node.
pub fn resolve_dead_node_id(
    events: &[Vec<u8>],
    explicit: Option<&str>,
) -> Result<String, RestoreError> {
    let found = enrolls(events);
    if let Some(e) = explicit.map(str::trim).filter(|s| !s.is_empty()) {
        let want = e.to_ascii_lowercase();
        if found.iter().any(|(id, _)| *id == want) {
            return Ok(want);
        }
        return Err(RestoreError::UnknownNodeId {
            wanted: want,
            available: found.into_iter().map(|(id, _)| id).collect(),
        });
    }
    match found.len() {
        0 => Err(RestoreError::NoGenesis),
        1 => Ok(found.into_iter().next().unwrap().0),
        n => Err(RestoreError::Ambiguous(n)),
    }
}

/// The (display_name, address) recorded in the enroll whose content-address == node_id.
/// Used so the new genesis keeps the node's name/address (paper-parity: a restored node
/// is the same clinic). Returns None if no such enroll is on the medium.
pub fn old_genesis_meta(events: &[Vec<u8>], node_id_hex: &str) -> Option<(String, String)> {
    let want = node_id_hex.to_ascii_lowercase();
    enrolls(events)
        .into_iter()
        .find(|(id, _)| *id == want)
        .map(|(_, body)| {
            let name = body
                .payload
                .get("display_name")
                .and_then(|v| v.as_str())
                .unwrap_or("restored-node")
                .to_string();
            let addr = body
                .payload
                .get("address")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            (name, addr)
        })
}

use tokio_postgres::Client;

/// What a completed restore produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreOutcome {
    pub new_node_id_hex: String,
    pub superseded_node_id_hex: String,
}

/// Apply every signed event from the medium through the self-trusting restore door, in
/// medium order (the node's own genesis is first, so non-enroll events resolve their
/// author). Idempotent: the door's ON CONFLICT DO NOTHING makes re-application a no-op.
/// MUST run while the DB is still un-enrolled — the door fails closed once a genesis
/// exists (which is exactly what finalize_identity creates next).
///
/// Returns the number of events PROCESSED (the slice length), not the number newly
/// inserted. Because the door uses ON CONFLICT DO NOTHING, re-applying the same medium
/// is a no-op at the DB level but still returns the same count — not 0.
pub async fn apply_medium(db: &Client, events: &[Vec<u8>]) -> anyhow::Result<usize> {
    use anyhow::Context;
    for (i, e) in events.iter().enumerate() {
        db.execute("SELECT restore_node_event($1)", &[e]).await
            .with_context(|| format!("applying restored event #{i}"))?;
    }
    Ok(events.len())
}

/// After the medium is applied, mint the node's NEW identity: author a fresh genesis
/// (sets local_node = NEW and permanently fences the restore door closed), then author a
/// node-level supersede(dead -> new). The signing key is the freshly-minted one (the old
/// key was never backed up). Returns the new + superseded node-ids for the operator.
pub async fn finalize_identity(
    db: &Client,
    sk: &cairn_event::SigningKey,
    key_id: &str,
    name: &str,
    address: &str,
    old_node_id_hex: &str,
) -> anyhow::Result<RestoreOutcome> {
    let new_node_id_hex = crate::identity::provision(db, sk, key_id, name, address).await?;
    crate::identity::author_supersede(db, sk, key_id, name, old_node_id_hex).await?;
    Ok(RestoreOutcome {
        new_node_id_hex,
        superseded_node_id_hex: old_node_id_hex.to_ascii_lowercase(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_event::{sign, EventBody, Hlc, SigningKey};

    fn enroll(sk: &SigningKey, name: &str) -> Vec<u8> {
        let kid = hex::encode(sk.verifying_key().to_bytes());
        let body = EventBody {
            event_id: uuid::Uuid::now_v7().to_string(),
            patient_id: crate::identity::NIL_PATIENT.into(),
            event_type: "node.enrolled".into(),
            schema_version: "node/1".into(),
            hlc: Hlc { wall: 1, counter: 0, node_origin: name.into() },
            t_effective: None,
            signer_key_id: kid,
            contributors: serde_json::json!([]),
            payload: serde_json::json!({ "display_name": name, "address": "10.0.0.1:7843" }),
            attachments: vec![],
        };
        sign(&body, sk).unwrap().signed_bytes
    }

    fn sk() -> SigningKey {
        cairn_event::generate_key().unwrap().0
    }
    fn node_id(ev: &[u8]) -> String {
        hex::encode(event_address(ev))
    }

    #[test]
    fn single_enroll_auto_detects_the_dead_node() {
        let k = sk();
        let ev = enroll(&k, "Solo");
        let got = resolve_dead_node_id(std::slice::from_ref(&ev), None).unwrap();
        assert_eq!(got, node_id(&ev), "the sole enroll is the dead node");
    }

    #[test]
    fn multiple_enrolls_require_an_explicit_arg() {
        let a = enroll(&sk(), "A");
        let b = enroll(&sk(), "B");
        let err = resolve_dead_node_id(&[a, b], None).unwrap_err();
        assert!(matches!(err, RestoreError::Ambiguous(2)));
    }

    #[test]
    fn explicit_arg_overrides_auto_detect() {
        let a = enroll(&sk(), "A");
        let b = enroll(&sk(), "B");
        let want = node_id(&b);
        let got = resolve_dead_node_id(&[a, b], Some(&want)).unwrap();
        assert_eq!(got, want);
    }

    #[test]
    fn explicit_arg_not_on_medium_is_rejected() {
        // A well-formed hex node-id that is NOT any enroll on the medium. Must fail
        // CLOSED here, before any DB write — otherwise it would silently supersede a
        // node that was never restored (and name the new node "restored-node").
        let a = enroll(&sk(), "A");
        let b = enroll(&sk(), "B");
        let bogus = "1220".to_string() + &"99".repeat(32);
        let err = resolve_dead_node_id(&[a, b], Some(&bogus)).unwrap_err();
        assert!(
            matches!(err, RestoreError::UnknownNodeId { .. }),
            "an off-medium dead node-id must fail closed, got: {err:?}"
        );
    }

    #[test]
    fn explicit_malformed_hex_is_rejected_before_any_db_write() {
        // A non-hex value can never match an enroll's content-address, so it is
        // rejected here rather than aborting deep in submit_node_event's
        // decode(...,'hex') AFTER the new genesis was already committed.
        let a = enroll(&sk(), "A");
        let err =
            resolve_dead_node_id(std::slice::from_ref(&a), Some("not-a-node-id")).unwrap_err();
        assert!(
            matches!(err, RestoreError::UnknownNodeId { .. }),
            "a malformed dead node-id must fail closed, got: {err:?}"
        );
    }

    #[test]
    fn no_enroll_is_an_error() {
        let err = resolve_dead_node_id(&[], None).unwrap_err();
        assert!(matches!(err, RestoreError::NoGenesis));
    }

    #[test]
    fn old_genesis_meta_reads_name_and_address() {
        let k = sk();
        let ev = enroll(&k, "Clinic-7");
        let (name, addr) = old_genesis_meta(std::slice::from_ref(&ev), &node_id(&ev)).unwrap();
        assert_eq!(name, "Clinic-7");
        assert_eq!(addr, "10.0.0.1:7843");
    }
}
