//! ADR-0026 slice C — restore orchestration (apply a backup medium under a new identity),
//! plus the self-identification of a cold medium (issue #53).
//!
//! WHY: the live apply_remote_node_event gate is the PEER-admission path and rejects a node
//! rehydrating its OWN history (no trust set in a fresh DB). Restore therefore uses the
//! self-trusting restore_node_event door (db/009), then mints a fresh key and records a
//! node-level supersede (the old signing key is never backed up). This module holds the PURE
//! helpers (which enroll on the medium is "self", old-genesis metadata) and the thin DB
//! orchestration; main.rs owns key-minting + recovery-code printing (as `init` does).
//!
//! Identifying self is NOT derivable from the events: by set-union sync a node's `node_event`
//! set CONVERGES with its peers', so the events alone cannot say which enroll is local. Self
//! is read from the medium's container-level self-marker, written at backup time (see
//! [`crate::medium`]). A SIGNED marker on a SOLE-enroll medium is authoritative and tamper-evident.
//! A SIGNED marker on a MULTI-enroll (federated/converged) medium still resolves self correctly
//! absent an adversary, but cannot — by construction — rule out a peer-medium splice (converged
//! peers hold byte-identical event sets; see [`crate::medium`]), so it is surfaced as
//! [`Provenance::SignedFederated`] for operator confirmation. An UNSIGNED marker pins self
//! (operator-error-safe) but is flagged; a marker-less legacy medium falls back to an explicit
//! `--superseded-node` (or a sole-enroll medium).

use crate::medium::{enrolls, scan_enrolls, verify_self_attestation, Container, SelfMarker};

#[derive(thiserror::Error, Debug)]
pub enum RestoreError {
    #[error("medium has no genesis (node.enrolled) event")]
    NoGenesis,
    /// The medium DOES carry events, but none verified and so no genesis could be read.
    /// Distinct from [`RestoreError::NoGenesis`] for honest degradation at the
    /// disaster-recovery seam: "the genesis is absent" and "the genesis may well be
    /// present but the signatures do not verify (corrupt medium, or a backup written
    /// before the ADR-0040 signing contexts — such media need re-signing/re-init)"
    /// send the operator down entirely different paths.
    #[error(
        "medium has no VERIFIABLE genesis: {unverifiable} of {total} event(s) failed signature \
         verification (corrupt bytes, or signatures predating the ADR-0040 signing contexts — \
         pre-ADR-0040 backups fail closed and need re-initialization)"
    )]
    NoVerifiableGenesis { unverifiable: usize, total: usize },
    /// A marker-less (legacy) medium with more than one enroll: self cannot be identified from
    /// the events (set-union convergence), so the operator must name the dead node explicitly.
    #[error("marker-less medium carries {0} enrolls; pass --superseded-node <hex> to pick the dead node")]
    Ambiguous(usize),
    #[error(
        "--superseded-node {wanted} names no enroll on this medium (available: {available:?})"
    )]
    UnknownNodeId {
        wanted: String,
        available: Vec<String>,
    },
    /// The explicit `--superseded-node` contradicts the medium's self-marker — it names a peer
    /// or some other enroll, not this node. Fail closed before any DB write rather than record a
    /// wrong, immutable supersede edge / adopt a peer's name (issue #53).
    #[error(
        "--superseded-node {wanted} is not this node (the medium's self-marker says {self_id})"
    )]
    NotSelf { wanted: String, self_id: String },
    /// The medium's self-marker did not verify: a SIGNED marker failed its signature/bind check
    /// (tampered, corrupt, signed by a key that is not the named genesis's, or committing to a
    /// different event set), or an UNSIGNED marker names an enroll absent from the medium. A
    /// failed marker WITHHOLDS self-detection (fail closed → name the node explicitly); it cannot
    /// be turned into a wrong-but-valid identity (that would need a forged signature).
    #[error("medium self-marker did not verify (tampered or mismatched); pass --superseded-node to override")]
    InvalidSelfMarker,
}

/// How "self" was determined for a restore — surfaced so the CLI can warn appropriately.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    /// A valid SIGNED self-attestation on a SOLE-enroll medium — fully tamper-evident: no peer
    /// genesis is present, so a foreign/spliced marker would name an absent enroll and fail closed.
    Signed,
    /// A valid SIGNED self-attestation on a MULTI-enroll (federated/converged) medium. It resolves
    /// self correctly absent an adversary, but converged peers hold byte-identical event sets, so
    /// an attacker holding a PEER's genuine cold medium could splice that peer's valid marker here
    /// and the commitment cannot distinguish them (see [`crate::medium`]). Not a silent misdirect
    /// — surfaced so the CLI asks the operator to confirm the echoed name/address.
    SignedFederated,
    /// An UNSIGNED self-marker — operator-error-safe, not tamper-evident. Warn the operator to
    /// confirm the restored identity's name/address.
    Unsigned,
    /// No marker on the medium (legacy CAIRNB1 / pre-enrollment backup). Self came from an
    /// explicit `--superseded-node` or a sole-enroll medium. Warn the operator to confirm.
    NoMarker,
}

/// The dead node a restore will supersede, plus how it was identified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadNode {
    pub node_id_hex: String,
    pub provenance: Provenance,
}

/// Cross-check an explicit `--superseded-node` against the resolved self id. An explicit value
/// is a confirmation/override: it must equal self, else fail closed (a peer / wrong enroll →
/// `NotSelf`; an off-medium / malformed value → `UnknownNodeId`). `None` means "trust the
/// marker", which is the common case.
fn confirm_explicit(
    explicit: Option<&str>,
    self_id: &str,
    all: &[(String, cairn_event::EventBody)],
) -> Result<(), RestoreError> {
    let Some(want) = explicit.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(());
    };
    let want = want.to_ascii_lowercase();
    if want == self_id {
        return Ok(());
    }
    if all.iter().any(|(id, _)| *id == want) {
        return Err(RestoreError::NotSelf {
            wanted: want,
            self_id: self_id.to_string(),
        });
    }
    Err(RestoreError::UnknownNodeId {
        wanted: want,
        available: all.iter().map(|(id, _)| id.clone()).collect(),
    })
}

/// Resolve which enroll on the cold medium is THIS node (the dead node a restore supersedes),
/// using the container's self-marker as the source of truth.
///
/// - SIGNED marker: verified against an enroll on the medium signed by the same key. A
///   tampered/forged marker fails closed (`InvalidSelfMarker`). Fully authoritative on a
///   sole-enroll medium (`Provenance::Signed`); on a multi-enroll medium it resolves self but
///   carries a residual peer-medium splice risk, so it is reported as `Provenance::SignedFederated`
///   for operator confirmation (see [`Provenance`] / [`crate::medium`]).
/// - UNSIGNED marker: pins self (closes the operator-typo footgun) but is not tamper-evident;
///   the named id must be an enroll on the medium, else `InvalidSelfMarker`.
/// - No marker (legacy/pre-enrollment): self is not derivable from the events, so require an
///   explicit `--superseded-node` (validated to be on the medium), or auto-resolve a sole
///   enroll. More than one enroll without a marker is `Ambiguous`.
///
/// In every branch an explicit `--superseded-node` is validated by [`confirm_explicit`], so
/// naming a peer or an off-medium id is rejected BEFORE any key-minting or DB write.
pub fn resolve_dead_node(
    container: &Container,
    explicit: Option<&str>,
) -> Result<DeadNode, RestoreError> {
    let scan = scan_enrolls(&container.events);
    let all = scan.enrolls;
    if all.is_empty() {
        // Distinguish "no genesis on the medium" from "events present but none
        // verified" — the latter means corruption or pre-ADR-0040 signatures, and
        // misreporting it as a missing genesis misdirects disaster recovery.
        if scan.unverifiable > 0 {
            return Err(RestoreError::NoVerifiableGenesis {
                unverifiable: scan.unverifiable,
                total: container.events.len(),
            });
        }
        return Err(RestoreError::NoGenesis);
    }

    match &container.self_marker {
        Some(SelfMarker::Signed(att)) => {
            let self_id = verify_self_attestation(att, &container.events)
                .ok_or(RestoreError::InvalidSelfMarker)?;
            confirm_explicit(explicit, &self_id, &all)?;
            // A signed marker is fully tamper-evident only on a SOLE-enroll medium: with no peer
            // genesis present, a spliced foreign marker would name an absent enroll and fail
            // closed. On a multi-enroll (federated/converged) medium a peer's genuine marker could
            // be spliced (the commitment cannot distinguish byte-identical converged sets), so we
            // downgrade to SignedFederated to prompt operator confirmation. Conservative: it flags
            // every multi-enroll medium, including ones not actually converged with the peer.
            let provenance = if all.len() == 1 {
                Provenance::Signed
            } else {
                Provenance::SignedFederated
            };
            Ok(DeadNode {
                node_id_hex: self_id,
                provenance,
            })
        }
        Some(SelfMarker::Unsigned(id)) => {
            let self_id = id.to_ascii_lowercase();
            if !all.iter().any(|(e, _)| *e == self_id) {
                return Err(RestoreError::InvalidSelfMarker);
            }
            confirm_explicit(explicit, &self_id, &all)?;
            Ok(DeadNode {
                node_id_hex: self_id,
                provenance: Provenance::Unsigned,
            })
        }
        None => resolve_without_marker(explicit, all),
    }
}

/// The marker-less (legacy / pre-enrollment) fallback: self is not derivable from the events,
/// so honour an explicit `--superseded-node` (validated to name an enroll on the medium), or
/// auto-resolve a sole-enroll medium; more than one enroll is `Ambiguous`.
fn resolve_without_marker(
    explicit: Option<&str>,
    all: Vec<(String, cairn_event::EventBody)>,
) -> Result<DeadNode, RestoreError> {
    if let Some(want) = explicit.map(str::trim).filter(|s| !s.is_empty()) {
        let want = want.to_ascii_lowercase();
        if all.iter().any(|(id, _)| *id == want) {
            return Ok(DeadNode {
                node_id_hex: want,
                provenance: Provenance::NoMarker,
            });
        }
        return Err(RestoreError::UnknownNodeId {
            wanted: want,
            available: all.into_iter().map(|(id, _)| id).collect(),
        });
    }
    match all.len() {
        1 => Ok(DeadNode {
            node_id_hex: all.into_iter().next().unwrap().0,
            provenance: Provenance::NoMarker,
        }),
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
        db.execute("SELECT restore_node_event($1)", &[e])
            .await
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
    use crate::medium::build_self_attestation;
    use cairn_event::{event_address, sign, EventBody, Hlc, SigningKey};

    fn sk() -> SigningKey {
        cairn_event::generate_key().unwrap().0
    }
    fn kid(sk: &SigningKey) -> String {
        hex::encode(sk.verifying_key().to_bytes())
    }
    fn node_id(ev: &[u8]) -> String {
        hex::encode(event_address(ev))
    }

    fn enroll(sk: &SigningKey, name: &str) -> Vec<u8> {
        let body = EventBody {
            event_id: uuid::Uuid::now_v7().to_string(),
            patient_id: crate::identity::NIL_PATIENT.into(),
            event_type: "node.enrolled".into(),
            schema_version: "node/1".into(),
            hlc: Hlc {
                wall: 1,
                counter: 0,
                node_origin: name.into(),
            },
            t_effective: None,
            signer_key_id: kid(sk),
            contributors: serde_json::json!([]),
            payload: serde_json::json!({ "display_name": name, "address": "10.0.0.1:7843" }),
            attachments: vec![],
            plaintext_twin: None,
        };
        sign(&body, sk).unwrap().signed_bytes
    }

    /// Build a Container with a SIGNED self-marker attesting `self_k`'s genesis.
    fn signed_container(self_k: &SigningKey, events: Vec<Vec<u8>>, self_id: &str) -> Container {
        let att = build_self_attestation(self_k, &kid(self_k), self_id, &events);
        Container {
            self_marker: Some(SelfMarker::Signed(att)),
            events,
        }
    }

    #[test]
    fn signed_marker_resolves_self_on_a_federated_medium() {
        // A converged federated medium: self + a peer, indistinguishable from the events alone.
        // The SIGNED marker resolves self unambiguously, but a multi-enroll medium carries a
        // residual peer-medium splice risk, so provenance is SignedFederated (confirm on restore).
        let self_k = sk();
        let self_ev = enroll(&self_k, "Self");
        let peer_ev = enroll(&sk(), "Peer");
        let self_id = node_id(&self_ev);
        let c = signed_container(&self_k, vec![self_ev, peer_ev], &self_id);

        let got = resolve_dead_node(&c, None).unwrap();
        assert_eq!(got.node_id_hex, self_id);
        assert_eq!(
            got.provenance,
            Provenance::SignedFederated,
            "a multi-enroll medium is flagged for confirmation, not claimed fully tamper-evident"
        );
    }

    #[test]
    fn signed_marker_on_a_sole_enroll_medium_is_fully_signed() {
        // The genuinely-solo clinic (ADR-0026's primary case): one enroll on the medium, so a
        // spliced foreign marker would name an absent enroll and fail closed. The signed marker is
        // therefore fully tamper-evident → Provenance::Signed.
        let self_k = sk();
        let self_ev = enroll(&self_k, "Solo");
        let self_id = node_id(&self_ev);
        let c = signed_container(&self_k, vec![self_ev], &self_id);

        let got = resolve_dead_node(&c, None).unwrap();
        assert_eq!(got.node_id_hex, self_id);
        assert_eq!(got.provenance, Provenance::Signed);
    }

    #[test]
    fn signed_marker_rejects_explicit_naming_a_peer() {
        // The issue #53 footgun: naming a PEER's real node-id (it IS on the medium) must fail
        // closed, never record a supersede against a node that is not self.
        let self_k = sk();
        let self_ev = enroll(&self_k, "Self");
        let peer_ev = enroll(&sk(), "Peer");
        let self_id = node_id(&self_ev);
        let c = signed_container(&self_k, vec![self_ev, peer_ev.clone()], &self_id);

        let err = resolve_dead_node(&c, Some(&node_id(&peer_ev))).unwrap_err();
        assert!(
            matches!(err, RestoreError::NotSelf { .. }),
            "naming a peer must fail closed: {err:?}"
        );
    }

    #[test]
    fn signed_marker_accepts_matching_explicit() {
        let self_k = sk();
        let self_ev = enroll(&self_k, "Self");
        let self_id = node_id(&self_ev);
        let c = signed_container(&self_k, vec![self_ev], &self_id);
        let got = resolve_dead_node(&c, Some(&self_id)).unwrap();
        assert_eq!(got.node_id_hex, self_id);
    }

    #[test]
    fn forged_signed_marker_fails_closed() {
        // A marker signed by an attacker's key naming self's id must NOT verify — the attacker
        // holds no node key, so they cannot forge a valid self-attestation.
        let self_k = sk();
        let self_ev = enroll(&self_k, "Self");
        let self_id = node_id(&self_ev);
        let attacker = sk();
        let forged =
            build_self_attestation(&attacker, &kid(&attacker), &self_id, std::slice::from_ref(&self_ev));
        let c = Container {
            self_marker: Some(SelfMarker::Signed(forged)),
            events: vec![self_ev],
        };
        assert!(matches!(
            resolve_dead_node(&c, None),
            Err(RestoreError::InvalidSelfMarker)
        ));
    }

    #[test]
    fn unsigned_marker_resolves_self_and_flags_provenance() {
        let self_k = sk();
        let self_ev = enroll(&self_k, "Self");
        let self_id = node_id(&self_ev);
        let c = Container {
            self_marker: Some(SelfMarker::Unsigned(self_id.clone())),
            events: vec![self_ev, enroll(&sk(), "Peer")],
        };
        let got = resolve_dead_node(&c, None).unwrap();
        assert_eq!(got.node_id_hex, self_id);
        assert_eq!(
            got.provenance,
            Provenance::Unsigned,
            "unsigned marker is flagged for caution"
        );
    }

    #[test]
    fn unsigned_marker_naming_an_absent_enroll_fails_closed() {
        let self_k = sk();
        let self_ev = enroll(&self_k, "Self");
        let ghost = node_id(&enroll(&sk(), "Ghost")); // not on the medium
        let c = Container {
            self_marker: Some(SelfMarker::Unsigned(ghost)),
            events: vec![self_ev],
        };
        assert!(matches!(
            resolve_dead_node(&c, None),
            Err(RestoreError::InvalidSelfMarker)
        ));
    }

    #[test]
    fn marker_less_sole_enroll_auto_detects() {
        let ev = enroll(&sk(), "Solo");
        let c = Container {
            self_marker: None,
            events: vec![ev.clone()],
        };
        let got = resolve_dead_node(&c, None).unwrap();
        assert_eq!(got.node_id_hex, node_id(&ev));
        assert_eq!(got.provenance, Provenance::NoMarker);
    }

    #[test]
    fn marker_less_multiple_enrolls_require_explicit() {
        let c = Container {
            self_marker: None,
            events: vec![enroll(&sk(), "A"), enroll(&sk(), "B")],
        };
        assert!(matches!(
            resolve_dead_node(&c, None),
            Err(RestoreError::Ambiguous(2))
        ));
    }

    #[test]
    fn marker_less_explicit_must_be_on_the_medium() {
        let a = enroll(&sk(), "A");
        let c = Container {
            self_marker: None,
            events: vec![a, enroll(&sk(), "B")],
        };
        let bogus = "1220".to_string() + &"99".repeat(32);
        assert!(matches!(
            resolve_dead_node(&c, Some(&bogus)),
            Err(RestoreError::UnknownNodeId { .. })
        ));
    }

    #[test]
    fn no_enroll_is_an_error() {
        let c = Container {
            self_marker: None,
            events: vec![],
        };
        assert!(matches!(
            resolve_dead_node(&c, None),
            Err(RestoreError::NoGenesis)
        ));
    }

    #[test]
    fn medium_whose_events_fail_verification_reports_that_not_a_missing_genesis() {
        // Honest degradation at the disaster-recovery seam: a medium whose genesis IS
        // present but fails signature verification (corrupt bytes, or signatures
        // predating the ADR-0040 signing contexts) must NOT be misdiagnosed as
        // "medium has no genesis" — that sends the operator hunting for a missing
        // event instead of a verification problem.
        let k = sk();
        let mut g = enroll(&k, "Self");
        let mid = g.len() / 2;
        g[mid] ^= 0x01; // the genesis is on the medium; it merely no longer verifies
        let c = Container {
            self_marker: None,
            events: vec![g],
        };
        match resolve_dead_node(&c, None) {
            Err(RestoreError::NoVerifiableGenesis {
                unverifiable: 1,
                total: 1,
            }) => {}
            other => panic!("expected NoVerifiableGenesis {{1,1}}, got {other:?}"),
        }
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
