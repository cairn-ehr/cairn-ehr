//! The backup-medium container format and its self-marker (ADR-0026 slice B + issue #53).
//!
//! WHY A SELF-MARKER: a backup medium is a node's `node_event` set. By set-union sync that
//! set CONVERGES with every peer's — two fully-synced mutual peers hold byte-identical event
//! sets. So nothing *in the events* can say which node a given backup belongs to; on restore
//! we could not tell "self" from a peer, and would record a wrong, immutable supersede edge
//! and adopt a peer's name (issue #53). The fix is a marker written into the CONTAINER (not
//! the synced event stream) at backup time, when `local_node` still names self authoritatively.
//!
//! The marker is SIGNED when the node's key is available at backup, UNSIGNED otherwise — an
//! unsigned marker never blocks a backup, it just travels flagged for caution. The safety
//! asymmetry we want (mirrors "uncertainty can only withhold an auto-link"): tampering with a
//! SIGNED marker can only WITHHOLD (delete/corrupt → restore fails closed to a manual choice),
//! and an attacker holds no private key (the signing key is never backed up) so a *wrong*
//! self-attestation cannot be FORGED.
//!
//! KNOWN LIMITATION — the converged-peer splice (issue #53 follow-up). The "never misdirect"
//! property is NOT absolute. The medium bind ([`event_set_commitment`]) ties a marker to the
//! exact event SET it sits beside, which rejects a marker lifted from a backup with a *different*
//! set. But two fully-converged mutual peers hold BYTE-IDENTICAL event sets (that is the very
//! premise of this marker), so their commitments are identical too. An attacker who physically
//! holds a PEER's genuine cold medium can therefore splice that peer's valid signed marker onto
//! this one and `verify_self_attestation` cannot tell them apart — there is no signal in the
//! shared bytes that distinguishes the two media. The splice is IMPOSSIBLE on a sole-enroll
//! medium (a foreign marker would name an absent enroll → fail closed), so the residual risk is
//! exactly the multi-enroll / federated case. Its defences are not in this module: restore-time
//! provenance ([`crate::restore::Provenance::SignedFederated`] → confirm the echoed name/address)
//! plus physical custody of the medium. So: forgery-proof always; misdirect-proof for sole-enroll
//! media and for splices from a *different* set; a peer-medium splice between converged peers is a
//! confirm-on-restore residual, not a silent misdirect.
//!
//! This module does no DB and no I/O (serialization, parsing, and signature checks only), so it
//! is trivially unit-testable and reusable by both the backup and restore paths. It is *mostly*
//! pure — [`build_self_attestation`] is the one exception (it mints a fresh UUID, see its docs).

use cairn_event::{event_address, sign, verify_self_described, EventBody, Hlc, SigningKey};

/// Magic header for the original marker-less medium (ADR-0026 slice B). Kept for backward
/// compatibility: such a medium parses to events with `self_marker == None`.
pub const MEDIUM_MAGIC_V1: &[u8] = b"CAIRNB1\n";
/// Magic header for the self-marked medium (issue #53). Carries a marker block before the
/// event frames. Distinct from the keystore's `CAIRNK1` so the artifacts can never be confused.
pub const MEDIUM_MAGIC_V2: &[u8] = b"CAIRNB2\n";

/// Event-type of the in-container self-attestation. NOT a clinical/node event — it never
/// enters `node_event` and never syncs (that is the whole point: it must NOT converge).
pub const SELF_ATTEST_TYPE: &str = "node.self_attested";

/// Upper bound on a single length-prefixed chunk on the medium (event frame OR marker). A
/// signed node-event is a few hundred bytes; 8 MiB caps a corrupt length prefix so a bit-flip
/// can never force a multi-GiB allocation during parse. Mirrors the wire frame cap in `sync.rs`.
const MAX_CHUNK_BYTES: usize = 8 * 1024 * 1024;

/// Marker kind discriminant bytes (first byte of the CAIRNB2 marker block).
const KIND_NONE: u8 = 0;
const KIND_UNSIGNED: u8 = 1;
const KIND_SIGNED: u8 = 2;

#[derive(thiserror::Error, Debug)]
pub enum BackupError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// The medium bytes are not a valid backup container (bad magic / truncated frame).
    #[error("decode: {0}")]
    Decode(String),
}

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

/// A parsed backup medium: the (optional) self-marker plus the signed event set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Container {
    /// `None` for a legacy CAIRNB1 medium (no marker) or a backup taken before enrollment.
    pub self_marker: Option<SelfMarker>,
    pub events: Vec<Vec<u8>>,
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
/// [`crate::restore::RestoreError::NoVerifiableGenesis`].
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
        patient_id: crate::identity::NIL_PATIENT.into(),
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
///     (see [`crate::restore::Provenance::SignedFederated`] and the module docs), not here;
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
// Length-prefixed chunk helpers (pure).
// ---------------------------------------------------------------------------

/// Append a `[u32 big-endian length][bytes]` chunk. The cap is asserted (debug) so a future
/// change that lifts the upstream size bound can never silently truncate a length prefix.
fn put_chunk(out: &mut Vec<u8>, bytes: &[u8]) {
    debug_assert!(
        bytes.len() <= MAX_CHUNK_BYTES,
        "chunk exceeds the medium frame cap"
    );
    out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(bytes);
}

/// Read one `[u32 length][bytes]` chunk, returning (chunk, remainder). Errors (never panics)
/// on a truncated or over-cap frame — a partial/corrupt medium is reported, not accepted.
fn take_chunk(rest: &[u8]) -> Result<(&[u8], &[u8]), BackupError> {
    if rest.len() < 4 {
        return Err(BackupError::Decode(format!(
            "truncated medium: {} byte(s) without a complete length prefix",
            rest.len()
        )));
    }
    let len = u32::from_be_bytes([rest[0], rest[1], rest[2], rest[3]]) as usize;
    if len > MAX_CHUNK_BYTES {
        return Err(BackupError::Decode(format!(
            "medium frame length {len} exceeds {MAX_CHUNK_BYTES}-byte cap (corrupt)"
        )));
    }
    let end = 4 + len;
    if rest.len() < end {
        return Err(BackupError::Decode(format!(
            "truncated medium: frame claims {len} bytes, only {} remain",
            rest.len() - 4
        )));
    }
    Ok((&rest[4..end], &rest[end..]))
}

// ---------------------------------------------------------------------------
// Container serialize / parse (pure).
// ---------------------------------------------------------------------------

/// Serialize a self-marker into its kind-tagged block. Pure.
fn put_marker(out: &mut Vec<u8>, marker: Option<&SelfMarker>) {
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

/// Serialize a full CAIRNB2 container: magic ++ marker block ++ event frames. Pure. The event
/// order is preserved for legibility but is set-union-independent on restore (convergence is
/// by content-address).
pub fn serialize_container(marker: Option<&SelfMarker>, events: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::with_capacity(MEDIUM_MAGIC_V2.len() + 1 + 32 * events.len());
    out.extend_from_slice(MEDIUM_MAGIC_V2);
    put_marker(&mut out, marker);
    for e in events {
        put_chunk(&mut out, e);
    }
    out
}

/// Parse a medium image into its marker + event set. Handles BOTH formats: a CAIRNB2 medium
/// yields its marker; a legacy CAIRNB1 medium yields `self_marker: None`. Errors (never
/// panics) on bad magic, an unknown marker kind, or a truncated frame.
pub fn parse_container(bytes: &[u8]) -> Result<Container, BackupError> {
    if let Some(rest) = bytes.strip_prefix(MEDIUM_MAGIC_V2) {
        let (&kind, mut rest) = rest
            .split_first()
            .ok_or_else(|| BackupError::Decode("CAIRNB2 medium missing marker kind".into()))?;
        let self_marker = match kind {
            KIND_NONE => None,
            KIND_UNSIGNED => {
                let (id, r) = take_chunk(rest)?;
                rest = r;
                let id = std::str::from_utf8(id)
                    .map_err(|_| BackupError::Decode("unsigned marker is not UTF-8".into()))?;
                Some(SelfMarker::Unsigned(id.to_string()))
            }
            KIND_SIGNED => {
                let (att, r) = take_chunk(rest)?;
                rest = r;
                Some(SelfMarker::Signed(att.to_vec()))
            }
            other => {
                return Err(BackupError::Decode(format!("unknown marker kind {other}")));
            }
        };
        let events = take_frames(rest)?;
        Ok(Container {
            self_marker,
            events,
        })
    } else if let Some(rest) = bytes.strip_prefix(MEDIUM_MAGIC_V1) {
        Ok(Container {
            self_marker: None,
            events: take_frames(rest)?,
        })
    } else {
        Err(BackupError::Decode(
            "missing CAIRNB1/CAIRNB2 magic header".into(),
        ))
    }
}

/// Read the trailing repeated event frames until a clean end-of-buffer (peer-stream EOF style).
fn take_frames(mut rest: &[u8]) -> Result<Vec<Vec<u8>>, BackupError> {
    let mut events = Vec::new();
    while !rest.is_empty() {
        let (frame, r) = take_chunk(rest)?;
        events.push(frame.to_vec());
        rest = r;
    }
    Ok(events)
}

/// Parse just the event set from a medium image (either format). For callers that only verify
/// signatures and do not care about the marker (e.g. `verify-backup`).
pub fn parse_medium(bytes: &[u8]) -> Result<Vec<Vec<u8>>, BackupError> {
    Ok(parse_container(bytes)?.events)
}

// ---------------------------------------------------------------------------
// Signature verification of the event set (reuses the existing invariant — no DB).
// ---------------------------------------------------------------------------

/// What a verification pass found. `intact` events verified their signature; `first_bad` is the
/// index of the first that did NOT. A medium is sound iff every event is intact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyReport {
    pub total: usize,
    pub intact: usize,
    pub first_bad: Option<usize>,
}

impl VerifyReport {
    /// Every event verified — the backup is restorable as-is.
    pub fn all_intact(&self) -> bool {
        self.first_bad.is_none() && self.intact == self.total
    }
}

/// Verify ONE signed event the way restore will: its self-described Ed25519 key must sign the
/// COSE body, and the body's claimed signer must match that key. A flipped byte → `false`.
pub fn verify_event(signed: &[u8]) -> bool {
    verify_self_described(signed).is_ok()
}

/// Verify every event in a set. Deterministic; no DB, no external key.
pub fn verify_events(events: &[Vec<u8>]) -> VerifyReport {
    let mut intact = 0;
    let mut first_bad = None;
    for (i, e) in events.iter().enumerate() {
        if verify_event(e) {
            intact += 1;
        } else if first_bad.is_none() {
            first_bad = Some(i);
        }
    }
    VerifyReport {
        total: events.len(),
        intact,
        first_bad,
    }
}

/// Parse a medium image and verify every event in one step. A `Decode` error means the
/// container is structurally broken; an `Ok(report)` with `!all_intact()` means it parsed but
/// carries a tampered/corrupt event.
pub fn verify_medium_bytes(bytes: &[u8]) -> Result<VerifyReport, BackupError> {
    Ok(verify_events(&parse_medium(bytes)?))
}

/// Serialize a container and self-verify its event set in one step, returning the verified
/// bytes — or an error if the set fails its own signature check. Runs BEFORE the image is
/// written over the live medium (verify-before-write), so a serialization/signing regression
/// can never overwrite a good medium with an unrestorable one.
pub fn serialize_and_verify_container(
    marker: Option<&SelfMarker>,
    events: &[Vec<u8>],
) -> Result<Vec<u8>, BackupError> {
    let report = verify_events(events);
    if !report.all_intact() {
        return Err(BackupError::Decode(format!(
            "refusing to write a medium that fails its own self-verification \
             ({} of {} events intact, first bad at index {:?})",
            report.intact, report.total, report.first_bad
        )));
    }
    Ok(serialize_container(marker, events))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_event::{EventBody, Hlc, SigningKey};

    fn sk() -> SigningKey {
        cairn_event::generate_key().unwrap().0
    }
    fn kid(sk: &SigningKey) -> String {
        hex::encode(sk.verifying_key().to_bytes())
    }
    fn node_id(ev: &[u8]) -> String {
        hex::encode(event_address(ev))
    }

    /// A real, validly-signed enroll for `sk` — its content-address IS the node-id.
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
            clock_grade: cairn_event::ClockGrade::SelfAsserted,
        };
        sign(&body, sk).unwrap().signed_bytes
    }

    #[test]
    fn container_roundtrips_unsigned_marker_and_events() {
        let k = sk();
        let g = enroll(&k, "Self");
        let events = vec![g.clone()];
        let marker = SelfMarker::Unsigned(node_id(&g));
        let image = serialize_container(Some(&marker), &events);
        assert!(
            image.starts_with(MEDIUM_MAGIC_V2),
            "self-marked medium carries CAIRNB2 magic"
        );
        let got = parse_container(&image).unwrap();
        assert_eq!(got.self_marker, Some(marker));
        assert_eq!(got.events, events, "parse recovers the exact event set");
    }

    #[test]
    fn container_roundtrips_a_signed_marker() {
        let k = sk();
        let g = enroll(&k, "Self");
        let att = build_self_attestation(&k, &kid(&k), &node_id(&g), std::slice::from_ref(&g));
        let image = serialize_container(Some(&SelfMarker::Signed(att.clone())), &[g]);
        let got = parse_container(&image).unwrap();
        assert_eq!(got.self_marker, Some(SelfMarker::Signed(att)));
    }

    #[test]
    fn container_roundtrips_no_marker() {
        let k = sk();
        let events = vec![enroll(&k, "Self")];
        let image = serialize_container(None, &events);
        let got = parse_container(&image).unwrap();
        assert_eq!(got.self_marker, None);
        assert_eq!(got.events, events);
    }

    #[test]
    fn legacy_cairnb1_medium_parses_with_no_marker() {
        // A CAIRNB1 image (magic ++ frames) must still parse, yielding self_marker == None.
        let k = sk();
        let g = enroll(&k, "Self");
        let mut image = MEDIUM_MAGIC_V1.to_vec();
        put_chunk(&mut image, &g);
        let got = parse_container(&image).unwrap();
        assert_eq!(got.self_marker, None, "legacy medium has no marker");
        assert_eq!(got.events, vec![g]);
    }

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

    #[test]
    fn parse_rejects_missing_magic_and_unknown_kind() {
        assert!(matches!(
            parse_container(b"not a medium"),
            Err(BackupError::Decode(_))
        ));
        // CAIRNB2 with an out-of-range marker kind.
        let mut bad = MEDIUM_MAGIC_V2.to_vec();
        bad.push(99);
        assert!(matches!(parse_container(&bad), Err(BackupError::Decode(_))));
    }

    #[test]
    fn parse_rejects_a_truncated_frame() {
        let k = sk();
        let mut image = serialize_container(None, &[enroll(&k, "Self")]);
        image.pop(); // last frame now claims more bytes than remain
        assert!(matches!(
            parse_container(&image),
            Err(BackupError::Decode(_))
        ));
    }

    #[test]
    fn verify_pinpoints_a_tampered_event_through_the_container() {
        let k = sk();
        let events: Vec<Vec<u8>> = (0..3).map(|_| enroll(&k, "n")).collect();
        let mut image = serialize_container(None, &events);
        assert!(verify_medium_bytes(&image).unwrap().all_intact());
        // Corrupt a byte well inside the body region (after magic + marker-kind byte).
        let idx = MEDIUM_MAGIC_V2.len() + 24;
        image[idx] ^= 0xff;
        assert!(
            !verify_medium_bytes(&image).unwrap().all_intact(),
            "a bit-flip must fail verify"
        );
    }

    #[test]
    fn serialize_and_verify_refuses_a_tampered_set() {
        let k = sk();
        let mut events: Vec<Vec<u8>> = (0..3).map(|_| enroll(&k, "n")).collect();
        let mid = events[1].len() / 2;
        events[1][mid] ^= 0xff;
        assert!(matches!(
            serialize_and_verify_container(None, &events),
            Err(BackupError::Decode(_))
        ));
    }
}
