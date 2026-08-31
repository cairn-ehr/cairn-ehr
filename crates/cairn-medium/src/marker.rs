//! The self-marker: which node a medium belongs to, and the signed self-attestation that
//! makes that claim tamper-evident. See the crate docs for WHY a self-marker exists at all
//! (`node_event` converges by set-union, so nothing *in the events* can say whose backup this
//! is) and for the converged-peer splice this module's commitment cannot close.
//!
//! **CAIRNB2 only, and frozen.** This module serves media that already exist and gains
//! nothing from further work: CAIRNB3's equivalent of "which node does this belong to" is
//! `segment`, because a whole-set commitment ([`event_set_commitment`]) cannot survive an
//! append — appending one event changes the commitment of everything already committed to it.
//! Do not extend this module; a CAIRNB3 concern belongs in `segment`.

use crate::chunk::put_chunk;
use crate::container::{KIND_NONE, KIND_SIGNED, KIND_UNSIGNED};
use cairn_event::{event_address, sign, verify_self_described, EventBody, Hlc, SigningKey};

/// Event-type of the in-container self-attestation. NOT a clinical/node event — it never
/// enters `node_event` and never syncs (that is the whole point: it must NOT converge).
pub const SELF_ATTEST_TYPE: &str = "node.self_attested";

/// Which node a medium belongs to, written into the container at backup time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelfMarker {
    /// The self node-id (hex content-address), recorded without a signature — closes the
    /// operator-typo footgun but is not tamper-evident.
    Unsigned(String),
    /// A signed `node.self_attested` event (bytes) authored by the live node key. Cannot be
    /// FORGED (no off-medium private key) and is bound to its event set; the residual is a
    /// converged-peer splice on a multi-enroll medium (see module docs) — never a silent forge.
    Signed(Vec<u8>),
}

// ---------------------------------------------------------------------------
// Enroll scan (shared by restore + self-attestation verification).
// ---------------------------------------------------------------------------

/// What an enroll scan found: the verified enrolls PLUS a count of events that failed
/// signature verification. The count exists for honest degradation (principle 4): an
/// event that fails verify is invisible to every downstream decision, but "0 enrolls
/// because the medium is empty of them" and "0 enrolls because every event failed
/// verification (corrupt, or signed before the ADR-0040 signing contexts)" are very
/// different operator situations and must stay distinguishable — see
/// `cairn_node::restore::RestoreError::NoVerifiableGenesis`.
#[derive(Debug, Clone, PartialEq)]
pub struct EnrollScan {
    /// Every verified `node.enrolled` as (node_id_hex, body) pairs.
    pub enrolls: Vec<(String, EventBody)>,
    /// Events on the medium whose signature did NOT verify (any event type).
    pub unverifiable: usize,
}

/// Scan a medium's events for verified `node.enrolled` events, counting (never silently
/// dropping) events that fail signature verification. A node-id is the content-address
/// of its genesis, so we hash each VERIFIED enroll's bytes (a corrupt enroll cannot
/// name a node). Pure.
pub fn scan_enrolls(events: &[Vec<u8>]) -> EnrollScan {
    let mut found = Vec::new();
    let mut unverifiable = 0;
    for e in events {
        match verify_self_described(e) {
            Ok(body) => {
                if body.event_type == "node.enrolled" {
                    found.push((hex::encode(event_address(e)), body));
                }
            }
            Err(_) => unverifiable += 1,
        }
    }
    EnrollScan {
        enrolls: found,
        unverifiable,
    }
}

/// Every verified `node.enrolled` on the medium as (node_id_hex, body) pairs — the
/// scan without the failure count, for callers that only need the enrolls. Pure.
pub fn enrolls(events: &[Vec<u8>]) -> Vec<(String, EventBody)> {
    scan_enrolls(events).enrolls
}

// ---------------------------------------------------------------------------
// Self-attestation (the SIGNED marker payload).
// ---------------------------------------------------------------------------

/// A deterministic, order-independent commitment to a medium's event SET. Each event's
/// content-address is sorted (frame reordering — harmless under set-union — does not change it),
/// concatenated, and hashed. Pure. BINDS a self-attestation to the exact event set it was written
/// for: a genuine attestation lifted from a backup whose set DIFFERS commits to a different value,
/// and adding/removing any event changes it — both then fail closed. Caveat (see module docs): two
/// fully-converged peers hold IDENTICAL sets, so this commitment is identical on both and cannot
/// distinguish their media — it binds to set CONTENT, not to a node.
pub fn event_set_commitment(events: &[Vec<u8>]) -> String {
    let mut addresses: Vec<Vec<u8>> = events.iter().map(|e| event_address(e)).collect();
    addresses.sort();
    // Reuse event_address as a plain multihash(sha2-256) over the concatenation — no new dep.
    hex::encode(event_address(&addresses.concat()))
}

/// Build a signed self-attestation naming `self_node_id_hex`, authored by the live node key and
/// BOUND to the `events` it will be stored alongside (via [`event_set_commitment`]). No DB, but
/// NOT pure: it mints a fresh `event_id` (`Uuid::now_v7`, i.e. wall-clock + randomness), so two
/// calls differ. That is harmless — the `event_id` is neither committed nor checked on verify;
/// the attestation's authority comes entirely from its signature + the commitment + the signer
/// bind. The attestation is never ordered against anything, so it carries a fixed 0/0 HLC. It
/// lives in the backup container only — never inserted into `node_event`, never synced — so it
/// cannot converge away the local self-distinction it records, and the commitment ties it to this
/// medium's event SET so it cannot be replayed onto a backup with a DIFFERENT set (a converged
/// peer's identical-set medium is the documented exception — see module docs).
pub fn build_self_attestation(
    sk: &SigningKey,
    key_id: &str,
    self_node_id_hex: &str,
    events: &[Vec<u8>],
) -> Vec<u8> {
    let body = EventBody {
        event_id: uuid::Uuid::now_v7().to_string(),
        patient_id: cairn_event::NIL_PATIENT.into(),
        event_type: SELF_ATTEST_TYPE.into(),
        schema_version: "node/1".into(),
        hlc: Hlc {
            wall: 0,
            counter: 0,
            node_origin: self_node_id_hex.into(),
        },
        t_effective: None,
        signer_key_id: key_id.into(),
        // ADR-0051 ratified vocabulary ("device" is an actor kind, not a role).
        contributors: serde_json::json!([{"actor_id": key_id, "role": "recorded"}]),
        payload: serde_json::json!({
            "self_node_id_hex": self_node_id_hex,
            "event_set_commitment": event_set_commitment(events),
        }),
        attachments: vec![],
        plaintext_twin: None,
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    };
    // A signing failure here is a programming error (bad key), not a runtime condition.
    sign(&body, sk)
        .expect("self-attestation signing")
        .signed_bytes
}

/// Verify a signed self-attestation against the medium it sits on. Returns `Some(self_id_hex)`
/// IFF every check holds, else `None` (fail closed — a tampered, mismatched, or foreign-set
/// marker withholds the auto-detection rather than misdirecting it):
///   - the attestation's own signature verifies and it is a `node.self_attested`;
///   - it names a `self_node_id_hex`;
///   - its `event_set_commitment` matches THIS medium's event set (the MEDIUM-SET bind: a genuine
///     attestation lifted from a backup whose set DIFFERS commits to a different value and is
///     rejected). NOTE: this binds to set CONTENT, so it CANNOT reject a peer's genuine marker
///     spliced from a byte-identical converged medium — that residual is handled at restore time
///     (see `cairn_node::restore::Provenance::SignedFederated` and the module docs), not here;
///   - that id is the content-address of an enroll ON THIS medium; AND
///   - that enroll's genesis signer == the attestation's signer (the UNFORGEABLE bind: only the
///     node that signed its own genesis could have signed this attestation).
pub fn verify_self_attestation(attestation: &[u8], events: &[Vec<u8>]) -> Option<String> {
    let body = verify_self_described(attestation).ok()?;
    if body.event_type != SELF_ATTEST_TYPE {
        return None;
    }
    let self_id = body
        .payload
        .get("self_node_id_hex")?
        .as_str()?
        .to_ascii_lowercase();
    // MEDIUM bind: the attestation must commit to exactly this medium's event set.
    if body.payload.get("event_set_commitment")?.as_str()? != event_set_commitment(events) {
        return None;
    }
    let attester_key = body.signer_key_id;
    // SIGNER bind: the named id must be a genesis on the medium signed by the SAME key.
    enrolls(events)
        .into_iter()
        .find(|(id, _)| *id == self_id)
        .filter(|(_, genesis)| genesis.signer_key_id == attester_key)
        .map(|_| self_id)
}

// ---------------------------------------------------------------------------
// Self-marker serialization (pure). `container::serialize_container` calls this.
// ---------------------------------------------------------------------------

/// Serialize a self-marker into its kind-tagged block. Pure.
pub(crate) fn put_marker(out: &mut Vec<u8>, marker: Option<&SelfMarker>) {
    match marker {
        None => out.push(KIND_NONE),
        Some(SelfMarker::Unsigned(id)) => {
            out.push(KIND_UNSIGNED);
            put_chunk(out, id.as_bytes());
        }
        Some(SelfMarker::Signed(att)) => {
            out.push(KIND_SIGNED);
            put_chunk(out, att);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::{enroll, kid, node_id, sk};

    #[test]
    fn signed_attestation_verifies_against_its_own_genesis() {
        let k = sk();
        let g = enroll(&k, "Self");
        let att = build_self_attestation(&k, &kid(&k), &node_id(&g), std::slice::from_ref(&g));
        let got = verify_self_attestation(&att, std::slice::from_ref(&g));
        assert_eq!(
            got,
            Some(node_id(&g)),
            "attestation binds to its genesis on the medium"
        );
    }

    #[test]
    fn signed_attestation_rejected_when_signer_is_not_the_genesis_key() {
        // The unforgeable bind: an attestation signed by key B but naming A's node-id must NOT
        // verify, even though A's enroll is on the medium. An attacker has no private key, so
        // they can never produce a *valid* attestation for a node they do not control.
        let a = sk();
        let g_a = enroll(&a, "A");
        let attacker = sk();
        // Attacker signs an attestation naming A's node-id with the attacker's OWN key, bound
        // to A's medium (so the commitment passes and the SIGNER bind is what fails).
        let forged = build_self_attestation(
            &attacker,
            &kid(&attacker),
            &node_id(&g_a),
            std::slice::from_ref(&g_a),
        );
        assert_eq!(
            verify_self_attestation(&forged, &[g_a]),
            None,
            "an attestation whose signer != the named genesis's signer must fail closed"
        );
    }

    #[test]
    fn signed_attestation_rejected_when_named_node_absent_from_medium() {
        let k = sk();
        let g = enroll(&k, "Self");
        // Attestation names a node-id that is NOT on this medium, but is bound to this medium
        // (commitment passes) so the NAMED-ABSENT check is what fails.
        let other = sk();
        let ghost_id = node_id(&enroll(&other, "Ghost"));
        let att = build_self_attestation(&other, &kid(&other), &ghost_id, std::slice::from_ref(&g));
        assert_eq!(
            verify_self_attestation(&att, &[g]),
            None,
            "no enroll to bind to → fail closed"
        );
    }

    #[test]
    fn signed_attestation_rejected_when_spliced_onto_a_medium_with_a_different_set() {
        // Cross-medium splice onto a medium whose event SET DIFFERS: lift node B's GENUINE
        // attestation+genesis onto A's medium, which holds A's genesis + B's genesis. The
        // attestation's signature and signer-bind both pass — but it commits to B's OWN (smaller)
        // event set, not this medium's, so the MEDIUM-SET bind rejects it. This is the splice the
        // commitment DOES close. The converged-identical-set case is the documented residual the
        // commitment cannot close — see the next test.
        let b = sk();
        let g_b = enroll(&b, "B");
        let b_events = vec![g_b.clone()];
        let att_b = build_self_attestation(&b, &kid(&b), &node_id(&g_b), &b_events);
        // Target medium: A's genesis + B's genesis (a set B's attestation did NOT commit to).
        let a = sk();
        let foreign_medium = vec![enroll(&a, "A"), g_b];
        assert_eq!(
            verify_self_attestation(&att_b, &foreign_medium),
            None,
            "a marker committing to a DIFFERENT set must fail the commitment check"
        );
    }

    #[test]
    fn signed_attestation_cannot_reject_a_peer_marker_on_a_byte_identical_converged_medium() {
        // KNOWN LIMITATION (issue #53 follow-up — surfaced by code review): two fully-converged
        // mutual peers hold BYTE-IDENTICAL event sets, so `event_set_commitment` is identical on
        // both media. A peer's GENUINE signed marker therefore verifies against this medium's
        // (identical) set — the commitment binds to set CONTENT and so cannot tell the two media
        // apart. This test pins that reality honestly: the Signed path is forgery-proof and
        // splice-proof for a DIFFERENT set, but it CANNOT, on its own, reject a peer's valid
        // marker spliced between converged peers. The defence lives at restore time
        // (`Provenance::SignedFederated` → confirm name/address) + physical custody, NOT here.
        let a = sk();
        let b = sk();
        let g_a = enroll(&a, "A");
        let g_b = enroll(&b, "B");
        // The converged set both peers hold (identical bytes on each peer's own medium).
        let converged = vec![g_a, g_b.clone()];
        // B's GENUINE marker, built over the converged set as B's own backup would build it.
        let att_b = build_self_attestation(&b, &kid(&b), &node_id(&g_b), &converged);
        // Spliced onto A's medium, which holds the IDENTICAL converged set → still verifies as B.
        assert_eq!(
            verify_self_attestation(&att_b, &converged),
            Some(node_id(&g_b)),
            "on a byte-identical converged medium the commitment cannot reject a peer's genuine marker"
        );
    }

    #[test]
    fn signed_attestation_rejected_when_event_set_is_altered() {
        // Adding (or removing) any event after the attestation was built changes the medium
        // commitment, so the node's own attestation no longer validates → fail closed.
        let k = sk();
        let g = enroll(&k, "Self");
        let events = vec![g.clone()];
        let att = build_self_attestation(&k, &kid(&k), &node_id(&g), &events);
        let mut altered = events.clone();
        altered.push(enroll(&sk(), "Injected"));
        assert_eq!(
            verify_self_attestation(&att, &altered),
            None,
            "altering the event set must invalidate the bound attestation"
        );
        // The unaltered set still verifies (sanity).
        assert_eq!(verify_self_attestation(&att, &events), Some(node_id(&g)));
    }

    #[test]
    fn tampered_signed_attestation_fails_closed() {
        let k = sk();
        let g = enroll(&k, "Self");
        let mut att = build_self_attestation(&k, &kid(&k), &node_id(&g), std::slice::from_ref(&g));
        let mid = att.len() / 2;
        att[mid] ^= 0x01; // break the signature
        assert_eq!(
            verify_self_attestation(&att, &[g]),
            None,
            "a flipped byte must fail closed"
        );
    }
}
