//! Signature verification of an event set, reusing the existing self-described-signature
//! invariant (no DB, no external key needed — every signed event names its own verifying key).
//! This is the read side of the verify-before-write discipline: `serialize_and_verify_container`
//! is called BEFORE an image is written over a live medium, so a serialization/signing
//! regression can never overwrite a good medium with an unrestorable one.

use crate::container::{parse_medium, serialize_container, MediumV3};
use crate::error::BackupError;
use crate::marker::SelfMarker;
use crate::segment::Plane;
use cairn_event::verify_self_described;

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

// ---------------------------------------------------------------------------
// CAIRNB3 — the chain pass, the watermark, and self-identification (issue #500 slice 2a).
// ---------------------------------------------------------------------------

/// One thing wrong with one segment. Every variant carries the segment's PLANE and INDEX:
/// "clinical segment 7 breaks the chain" sends an operator somewhere, "chain invalid"
/// does not — and the standing rule in this codebase is NAME, NEVER COUNT.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SegmentFault {
    /// The attestation is present but does not verify against this segment (tampered, or
    /// the segment was moved in the chain).
    AttestationInvalid { plane: Plane, index: u32 },
    /// The records do not hash to the attested commitment.
    CommitmentMismatch { plane: Plane, index: u32 },
    /// `prev_commitment` does not match the preceding segment's commitment.
    ChainBroken {
        plane: Plane,
        index: u32,
        expected: String,
        found: String,
    },
    /// A signed segment names a node with no matching genesis on this medium.
    SelfIdUnbound {
        plane: Plane,
        index: u32,
        self_node_id_hex: String,
    },
}

/// What a chain pass found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainReport {
    pub segments: usize,
    pub signed: usize,
    /// Segments written without a signing key. NOT a fault: an unavailable key must never
    /// block a backup, so an unsigned segment travels flagged — it simply is not
    /// tamper-evident, and no caller may treat it as if it were.
    pub unsigned: usize,
    pub faults: Vec<SegmentFault>,
    /// Position (into `MediumV3::segments`) of the last segment verified with every
    /// preceding one. `None` when the first segment already fails. Everything at or before
    /// this is trustworthy; everything after it is not, which is what bounds the loss from
    /// a torn or damaged tail to exactly the increments after this point.
    pub verified_through: Option<usize>,
}

impl ChainReport {
    /// No faults. Unsigned segments do not make a medium un-intact.
    pub fn intact(&self) -> bool {
        self.faults.is_empty()
    }
}

/// Walk the chain once, in file order.
///
/// The walk STOPS advancing `verified_through` at the first fault: a chain is a chain, and
/// a segment after a break has no verified predecessor to hang from. Faults after that
/// point are still collected and reported, because "one break" and "the whole tail is
/// rubble" are different operator situations.
pub fn chain_report(m: &MediumV3) -> ChainReport {
    let mut faults = Vec::new();
    let mut signed = 0;
    let mut unsigned = 0;
    let mut verified_through = None;
    let mut expected_prev = String::new();
    let mut still_good = true;

    for (i, seg) in m.segments.iter().enumerate() {
        let mut ok = true;
        if seg.prev_commitment != expected_prev {
            faults.push(SegmentFault::ChainBroken {
                plane: seg.plane,
                index: seg.index,
                expected: expected_prev.clone(),
                found: seg.prev_commitment.clone(),
            });
            ok = false;
        }
        match &seg.attestation {
            None => unsigned += 1,
            Some(_) => {
                signed += 1;
                if crate::attest::verify_segment_attestation(seg).is_none() {
                    faults.push(SegmentFault::AttestationInvalid {
                        plane: seg.plane,
                        index: seg.index,
                    });
                    ok = false;
                }
            }
        }
        expected_prev = crate::attest::segment_commitment(&seg.records);
        if ok && still_good {
            verified_through = Some(i);
        } else {
            still_good = false;
        }
    }

    ChainReport {
        segments: m.segments.len(),
        signed,
        unsigned,
        faults,
        verified_through,
    }
}

/// The highest `source_seq` this medium can be TRUSTED to hold for `plane`.
///
/// Derived from `verified_through`, never from the file's tail. That is the property that
/// makes a torn append cost exactly one increment: an unverifiable trailing segment does
/// not advance the cursor, so the next capture re-writes its records rather than skipping
/// past them.
///
/// `None` — never `Some(0)` — when no verified segment of that plane exists. Zero is a
/// CLAIM ("everything up to seq 0 is here"); the honest answer to "what do you hold?" when
/// nothing verified is "I do not know".
pub fn watermark(m: &MediumV3, report: &ChainReport, plane: Plane) -> Option<i64> {
    let through = report.verified_through?;
    m.segments[..=through]
        .iter()
        .filter(|s| s.plane == plane)
        .flat_map(|s| s.records.iter().map(|r| r.source_seq))
        .max()
}

/// Verify every record's SIGNATURE, across every segment, in file order.
///
/// Separate from [`chain_report`] because it answers a different question and one does not
/// imply the other. The chain pass checks attestations and COMMITMENTS, and a commitment is
/// taken over content addresses — which a tampered blob still has, just a different one. In
/// a signed segment that is enough (the commitment fails), but an UNSIGNED segment has no
/// attestation, so without this pass nothing checks its bytes at all.
///
/// Reuses [`verify_event`] unchanged, so a record on a medium faces exactly the check a
/// replicated event faces at the apply door: no second definition of "valid".
pub fn verify_records(m: &MediumV3) -> VerifyReport {
    let events: Vec<Vec<u8>> = m
        .segments
        .iter()
        .flat_map(|s| s.records.iter().map(|r| r.signed_bytes.clone()))
        .collect();
    verify_events(&events)
}

/// Which node this medium belongs to, from the LAST verified signed segment.
///
/// Two binds, mirroring the CAIRNB2 marker's: the attestation must verify against its own
/// segment (done in `chain_report`), and the node it names must have a genesis
/// (`node.enrolled`) present on THIS medium, signed by the SAME key that signed the
/// attestation. The second bind is what makes a foreign attestation unusable: only the
/// node that signed its own genesis could have signed this.
///
/// `None` on any doubt. Fail closed — a withheld identification falls back to an operator
/// choice, whereas a wrong one records an immutable supersede against the wrong node.
pub fn self_id_from_chain(m: &MediumV3, report: &ChainReport) -> Option<String> {
    let through = report.verified_through?;
    // Every node-plane record on the medium, as candidate genesis events.
    let node_events: Vec<Vec<u8>> = m
        .segments
        .iter()
        .filter(|s| s.plane == Plane::Node)
        .flat_map(|s| s.records.iter().map(|r| r.signed_bytes.clone()))
        .collect();
    let found = crate::marker::enrolls(&node_events);

    for seg in m.segments[..=through].iter().rev() {
        // Every arm is `continue`, never `?`: an unsigned or unverifiable segment means
        // "keep looking further back", not "give up on the whole medium". A `?` here would
        // let one unsigned tail segment hide a perfectly good identification beneath it.
        let Some(att) = seg.attestation.as_deref() else {
            continue;
        };
        let Some(id) = crate::attest::verify_segment_attestation(seg) else {
            continue;
        };
        let Ok(body) = cairn_event::verify_self_described(att) else {
            continue;
        };
        let attester = body.signer_key_id;
        if found
            .iter()
            .any(|(gid, genesis)| *gid == id && genesis.signer_key_id == attester)
        {
            return Some(id);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attest::{segment_commitment, tests_support};
    use crate::container::MEDIUM_MAGIC_V2;
    use crate::record::MediumRecord;
    use crate::segment::Segment;
    use crate::testkit::{enroll, sk};

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

    #[test]
    fn a_well_formed_chain_verifies_through_its_last_segment() {
        let (m, _) = crate::testkit::chain_of(3, 1);
        let r = chain_report(&m);
        assert!(r.intact(), "faults: {:?}", r.faults);
        assert_eq!(r.verified_through, Some(2));
        assert_eq!((r.segments, r.signed, r.unsigned), (3, 3, 0));
    }

    /// A break is located by plane AND index. "chain invalid" sends an operator nowhere.
    #[test]
    fn a_chain_break_is_located_not_merely_counted() {
        let (mut m, _) = crate::testkit::chain_of(4, 1);
        m.segments[2].prev_commitment = "deadbeef".into();
        let r = chain_report(&m);
        assert!(!r.intact());
        assert_eq!(
            r.verified_through,
            Some(1),
            "verified through the last GOOD segment"
        );
        assert!(
            r.faults.iter().any(|f| matches!(
                f,
                SegmentFault::AttestationInvalid { index: 2, .. }
                    | SegmentFault::ChainBroken { index: 2, .. }
            )),
            "the fault must name segment 2: {:?}",
            r.faults
        );
    }

    /// A genuine segment spliced from ANOTHER medium fails on its predecessor, even though
    /// its own signature and commitment are perfectly valid.
    #[test]
    fn a_spliced_segment_fails_on_its_predecessor() {
        let (mut mine, _) = crate::testkit::chain_of(2, 1);
        let (theirs, _) = crate::testkit::chain_of(2, 2);
        mine.segments[1] = theirs.segments[1].clone();
        let r = chain_report(&mine);
        assert!(!r.intact(), "a foreign segment must not validate here");
    }

    /// The watermark comes from the last VERIFIED segment, so a torn or broken tail costs
    /// exactly one increment: its records are re-captured, never lost.
    #[test]
    fn the_watermark_ignores_everything_after_the_last_verified_segment() {
        let (mut m, seqs) = crate::testkit::chain_of(3, 1);
        let good = watermark(&m, &chain_report(&m), Plane::Clinical);
        assert_eq!(good, Some(seqs.last().copied().unwrap()));
        m.segments[2].records[0].signed_bytes[0] ^= 0xff; // breaks segment 2
        let after = watermark(&m, &chain_report(&m), Plane::Clinical);
        assert!(
            after < good,
            "a broken tail must not advance the cursor: {after:?} vs {good:?}"
        );
    }

    /// A plane with no verified segment has NO watermark — `None`, never `Some(0)`. Zero is
    /// a claim ("I hold everything up to seq 0"); the honest answer is "I do not know".
    #[test]
    fn a_plane_with_no_verified_segment_has_no_watermark() {
        let (m, _) = crate::testkit::chain_of(1, 1); // clinical only
        assert_eq!(watermark(&m, &chain_report(&m), Plane::Node), None);
    }

    /// Self-identification takes the LAST verified attestation and binds the named id to a
    /// genesis present on this medium, signed by the same key.
    #[test]
    fn self_id_binds_the_named_node_to_a_genesis_on_this_medium() {
        let (m, _) = crate::testkit::chain_with_genesis();
        let r = chain_report(&m);
        assert!(self_id_from_chain(&m, &r).is_some());
    }

    /// A named node with no genesis on the medium yields NOTHING. Fail closed: the marker
    /// can be withheld, never turned into a wrong-but-valid identity.
    #[test]
    fn an_unbound_self_id_is_withheld_not_guessed() {
        let (mut m, _) = crate::testkit::chain_with_genesis();
        m.segments[0].records.clear(); // remove the genesis; the attestation now mismatches
        let r = chain_report(&m);
        assert_eq!(self_id_from_chain(&m, &r), None);
    }

    /// Mutation testing (task 8 step 5) found that the test above never reaches the SIGNER
    /// bind at all: clearing the genesis segment's own records also invalidates ITS OWN
    /// attestation, so `verified_through` stays `None` and `self_id_from_chain` returns via
    /// `report.verified_through?` before the loop body — with or without the
    /// `genesis.signer_key_id == attester` conjunct — ever runs. Per house rule 5, a
    /// conjunct no test kills gets a test added, not quietly waved through.
    ///
    /// This test reaches the loop with a fully INTACT, correctly-chained medium instead, so
    /// the signer bind is the ONLY thing standing between a forged self-id and a false
    /// positive: an attacker holding no key for the real node can still sign a *valid*
    /// segment attestation that simply CLAIMS the real node's genuine node-id
    /// (`self_node_id_hex` is attacker-supplied, not derived) — the id half of the match
    /// succeeds on its own. The genesis segment is deliberately left UNSIGNED, so the only
    /// signed attestation anywhere on the medium naming `self_id` is the forged one; that
    /// isolates the signer-key comparison from the genuine-owner segment that would
    /// otherwise legitimately self-identify further back in the walk.
    #[test]
    fn a_forged_self_id_naming_a_real_genesis_is_withheld_not_misdirected() {
        let owner = sk();
        let genesis = enroll(&owner, "real-node");
        let self_id = hex::encode(cairn_event::event_address(&genesis));
        let node_records = vec![MediumRecord {
            signed_bytes: genesis,
            attestation: None,
            attester_key: None,
            dek_wrapped: None,
            source_seq: 1,
        }];
        let s0 = Segment {
            plane: Plane::Node,
            index: 0,
            prev_commitment: String::new(),
            self_node_id_hex: self_id.clone(),
            attestation: None, // unsigned: no genuine attestation for `self_id` exists here
            records: node_records,
        };
        let prev = segment_commitment(&s0.records);
        // An attacker, holding no key for `owner`, signs a genuinely-valid attestation that
        // simply claims the real node's self_id.
        let attacker = sk();
        let s1 = tests_support::signed(
            &attacker,
            &self_id,
            Plane::Clinical,
            1,
            &prev,
            vec![tests_support::salted_record(9, 0)],
        );
        let m = MediumV3 {
            segments: vec![s0, s1],
            unknown: vec![],
            truncated_tail: false,
        };
        let r = chain_report(&m);
        assert!(
            r.intact(),
            "both segments are internally consistent and correctly chained: {:?}",
            r.faults
        );
        assert_eq!(
            self_id_from_chain(&m, &r),
            None,
            "the claimed id matches a real genesis, but the signer does not — must withhold"
        );
    }

    /// Every record's SIGNATURE is checked, not merely its commitment.
    ///
    /// This is a distinct property and it is easy to lose: the chain pass verifies
    /// attestations and commitments, and a commitment is over content ADDRESSES — which a
    /// tampered blob still has. So in a SIGNED segment tampering is caught twice (the
    /// address changes, so the commitment fails), but in an UNSIGNED segment there is no
    /// attestation at all, and without this pass nothing would check the bytes.
    #[test]
    fn a_tampered_record_is_caught_even_in_an_unsigned_segment() {
        let mut m = crate::testkit::unsigned_chain_of(2);
        assert!(chain_report(&m).intact(), "unsigned but well-formed");
        let clean = verify_records(&m);
        assert_eq!(
            clean.first_bad, None,
            "the fixture's records must verify to begin with"
        );

        m.segments[1].records[0].signed_bytes[0] ^= 0xff;
        let report = verify_records(&m);
        assert_eq!(report.total, 2);
        assert_eq!(report.intact, 1);
        assert_eq!(
            report.first_bad,
            Some(1),
            "and it must NAME which record failed"
        );
    }

    /// An all-unsigned medium identifies nobody, and says so without inventing a fault.
    #[test]
    fn an_unsigned_medium_identifies_nobody_and_is_not_a_fault() {
        let m = crate::testkit::unsigned_chain_of(2);
        let r = chain_report(&m);
        assert_eq!((r.signed, r.unsigned), (0, 2));
        assert!(
            r.intact(),
            "unsigned is not a FAULT — it is a declared limitation"
        );
        assert_eq!(self_id_from_chain(&m, &r), None);
    }
}
