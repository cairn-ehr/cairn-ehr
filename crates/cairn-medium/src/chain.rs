//! The chain pass, the watermark, and self-identification for a CAIRNB3 medium (issue
//! #500 slice 2a).
//!
//! WHY A SEPARATE MODULE FROM `verify`: `verify.rs` answers one question — "do these
//! bytes verify" — a flat, whole-set signature check with no notion of segments, chains,
//! or planes. This module answers a different one — "does this medium's CHAIN hold" —
//! which needs attestations, commitments, predecessor links, per-plane watermarks, and
//! which node the medium belongs to.
//!
//! NEITHER MODULE ANSWERS "is this medium sound". That question is [`crate::health`]'s, and
//! it is the one a caller almost always wants: a chain pass alone reports a tampered record
//! in the last unsigned segment as perfectly intact, because an unsigned segment has no
//! attestation to fail and nothing chains off the last one. Reach for `health::assess`.
//!
//! FAULT REPORTING: [`SegmentFault`] is the vocabulary of everything that can be wrong
//! with a signed, chained medium. Every variant carries the segment's POSITION and PLANE —
//! the standing rule in this codebase is NAME, NEVER COUNT. [`chain_report`] is the one
//! function that walks every segment and constructs faults; the helpers below
//! ([`watermark`], [`self_id_from_chain`]) READ a `ChainReport`, they never redecide it.

use crate::container::MediumV3;
use crate::segment::Plane;
use crate::verify::{verify_events, VerifyReport};

/// One thing wrong with one segment.
///
/// EVERY VARIANT CARRIES `position` — the segment's index into [`MediumV3::segments`], which
/// is where the reader actually found it — as well as the segment's own self-declared
/// `index`. The two are different vocabularies and only the first is trustworthy: `index` is
/// a field read off the medium, and on an UNSIGNED segment it is entirely attacker- or
/// corruption-controlled. Locating a fault by `index` alone (as this enum once did) can send
/// an operator to "clinical segment 4000000000" for damage sitting at file position 3. The
/// standing rule is NAME, NEVER COUNT — and a name that points somewhere else is no better
/// than a count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SegmentFault {
    /// The attestation is present but does not verify against this segment (tampered, the
    /// segment was moved in the chain, or its records no longer hash to what was attested).
    ///
    /// NOTE on the absence of a `CommitmentMismatch` case: `attest::verify_segment_attestation`
    /// folds the commitment comparison into its own verdict, so a records-only mismatch
    /// already surfaces here. Distinguishing "the records changed" from "the signature itself
    /// changed" IS a strictly better operator diagnosis, but nothing consumes that distinction
    /// until the health and verify-backup surfaces land in a later slice. Deferred, not
    /// forgotten — reintroduce it together with the surface that displays it.
    AttestationInvalid {
        plane: Plane,
        position: usize,
        index: u32,
    },
    /// `prev_commitment` does not match the preceding segment's commitment.
    ChainBroken {
        plane: Plane,
        position: usize,
        index: u32,
        expected: String,
        found: String,
    },
    /// A signed segment names a node with no matching genesis on this medium — a forged or
    /// stale identity claim: an attacker holding no key for the real node can still sign a
    /// genuinely-valid segment attestation that simply CLAIMS the real node's id (or any
    /// other id), since `self_node_id_hex` is attacker-supplied, not derived. Raised ONLY
    /// when the medium carries at least one genesis anywhere (see `chain_report`'s guard):
    /// with no genesis at all, the honest verdict is "cannot determine", not "failed".
    SelfIdUnbound {
        plane: Plane,
        position: usize,
        index: u32,
        self_node_id_hex: String,
    },
    /// The segment's self-declared `index` disagrees with where it actually sits.
    ///
    /// This is what makes every OTHER fault's location trustworthy. It also turns issue #522
    /// (two crates independently deriving the next chain index, with no shared helper to keep
    /// them agreeing) from a silent divergence into a loud one: if `cairn-node` and
    /// `cairn-sync` ever disagree about numbering, the medium says so on the next read rather
    /// than quietly carrying two interleaved numbering schemes.
    IndexMismatch {
        plane: Plane,
        position: usize,
        declared: u32,
    },
    /// A plane tag this build does not recognise, written by a NEWER Cairn.
    ///
    /// **Not damage, and not the segment's fault.** The remedy is "upgrade this node", the
    /// opposite of "fetch another copy" — see [`crate::error::BackupError`] for why that
    /// distinction is load-bearing. It is a fault only in the sense that this build cannot
    /// route these records, so the medium must not be reported as fully readable HERE. The
    /// chain still traverses it (the records are readable as bytes and their commitment is
    /// computable), so a later known-plane segment is NOT collateral damage.
    UnknownPlane {
        plane_tag: u8,
        position: usize,
        index: u32,
        record_count: usize,
    },
    /// A segment carrying no records. `put_segment` refuses to write one, so finding one
    /// means the medium was written by something else — and it is dangerous: an empty
    /// segment's commitment is the multihash of the empty string, identical on every medium,
    /// so anything chaining off it can be spliced in from elsewhere undetected.
    EmptySegment {
        plane: Plane,
        position: usize,
        index: u32,
    },
}

impl SegmentFault {
    /// Where the reader actually found the faulty segment — the trustworthy coordinate.
    pub fn position(&self) -> usize {
        match self {
            SegmentFault::AttestationInvalid { position, .. }
            | SegmentFault::ChainBroken { position, .. }
            | SegmentFault::SelfIdUnbound { position, .. }
            | SegmentFault::IndexMismatch { position, .. }
            | SegmentFault::UnknownPlane { position, .. }
            | SegmentFault::EmptySegment { position, .. } => *position,
        }
    }
}

/// What a chain pass found.
///
/// This is HALF a medium's health — the structural half. See [`crate::health::MediumHealth`]
/// for the composed verdict, and read [`ChainReport::chain_intact`] before using this type to
/// decide anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainReport {
    pub segments: usize,
    /// Segments carrying an attestation that VERIFIED.
    pub signed_valid: usize,
    /// Segments carrying an attestation that did NOT verify. Counted separately because a
    /// single `signed` tally (which this type once carried) incremented on the presence of
    /// an attestation BLOB, before verification, and was never decremented — so a medium
    /// whose every attestation had been tampered with reported "3 signed, 0 unsigned"
    /// alongside three faults. Any operator surface that renders counts — which is what
    /// counts are for — would have stated the opposite of the facts.
    pub signed_invalid: usize,
    /// Segments written without a signing key. NOT a fault: an unavailable key must never
    /// block a backup, so an unsigned segment travels flagged — it simply is not
    /// tamper-evident, and no caller may treat it as if it were.
    pub unsigned: usize,
    pub faults: Vec<SegmentFault>,
    /// Position (into `MediumV3::segments`) of the last segment whose chain link (and,
    /// when SIGNED, attestation and self-id bind) held with every preceding one. `None`
    /// when the first segment already fails.
    ///
    /// WHAT THIS DOES NOT MEAN: `verified_through` is STRUCTURAL CHAIN CONTINUITY, not a
    /// tamper-evidence boundary. Two ways it is weaker than that:
    ///   1. An UNSIGNED segment sets `ok = true` on a matching `prev_commitment` ALONE, and
    ///      `prev_commitment` is `segment_commitment` over PUBLIC bytes — derivable by
    ///      anyone holding the preceding segment's records, no key required. So anyone can
    ///      append a well-formed unsigned segment carrying ARBITRARY `source_seq` and
    ///      advance this (and hence the watermark) with zero tamper-evidence. This is
    ///      operationally NECESSARY (principle 7: an unavailable signing key must never
    ///      block a backup), not an oversight — closing it needs an operator-facing signal
    ///      that does not exist yet, deferred to the slice that adds that surface.
    ///   2. `SelfIdUnbound` (a forged self-id claim) does NOT retract this, deliberately:
    ///      the records in that segment ARE validly signed and chain-linked — only the
    ///      identity CLAIM is forged. Only "which node captured this" is in doubt, which is
    ///      what [`self_id_from_chain`] (not this field) answers.
    ///   3. `UnknownPlane` does not retract it either: a newer Cairn's plane is structurally
    ///      sound, merely unroutable HERE, so the segments after it are not collateral damage.
    ///
    /// What this DOES bound honestly: a torn or structurally-broken tail costs exactly the
    /// increments after this point, never more, and never silently more than that.
    pub verified_through: Option<usize>,
}

impl ChainReport {
    /// The CHAIN holds: no structural fault, no unreadable plane, no forged identity claim.
    ///
    /// **This is not "the medium is sound".** It says nothing about whether the records'
    /// signatures verify (see [`verify_records`]), nor whether the file's tail was torn (see
    /// `MediumV3::truncated_tail`), nor whether the medium carries anything at all. A
    /// tampered record inside the LAST unsigned segment leaves this `true`: there is no
    /// attestation to fail, and nothing chains off the last segment to notice its commitment
    /// changed. Reading this as a whole-medium all-clear is precisely the composite untruth
    /// issue #500 is about — use [`crate::health::assess`] instead, which cannot be read that
    /// way because it composes every check this crate can make.
    pub fn chain_intact(&self) -> bool {
        self.faults.is_empty()
    }
}

/// Walk the chain once, in file order.
///
/// The walk STOPS advancing `verified_through` at the first STRUCTURAL fault: a chain is a
/// chain, and a segment after a break has no verified predecessor to hang from. Faults after
/// that point are still collected and reported, because "one break" and "the whole tail is
/// rubble" are different operator situations.
///
/// This is also where the SELF-ID bind is checked: for every SIGNED segment whose attestation
/// verifies, the id it claims must belong to a genesis actually present on this medium,
/// signed by the SAME key. Checking it HERE, not only in `self_id_from_chain`, is what turns
/// a forged claim into a LOCATED `SelfIdUnbound` instead of a bare `None` that tells an
/// operator nothing: a forged identity claim is the single most security-relevant failure
/// this file can report.
pub fn chain_report(m: &MediumV3) -> ChainReport {
    let mut faults = Vec::new();
    let mut signed_valid = 0;
    let mut signed_invalid = 0;
    let mut unsigned = 0;
    let mut verified_through = None;
    let mut expected_prev = String::new();
    let mut still_good = true;

    // Every verified genesis anywhere on the medium, gathered ONCE up front — a segment's
    // self-id can bind to a genesis anywhere in file order, not only to one already walked.
    //
    // DELIBERATELY NOT BOUNDED BY `verified_through` (unlike the attestation search in
    // `self_id_from_chain`), and the asymmetry is intentional: `enrolls` verifies each
    // genesis event's OWN Ed25519 signature, and a validly-signed genesis is validly signed
    // no matter which segment it sits in. Chain position is a statement about ordering, not
    // about whether a signature holds. Bounding this half would mean refusing to identify a
    // medium whose genesis happens to sit after a break — withholding an identification we
    // can actually prove.
    let node_events: Vec<Vec<u8>> = m
        .segments
        .iter()
        .filter(|s| s.plane == Plane::Node)
        .flat_map(|s| s.records.iter().map(|r| r.signed_bytes.clone()))
        .collect();
    let genesis = crate::marker::enrolls(&node_events);
    // GUARD — do not remove: with NO genesis anywhere on the medium, the honest verdict for
    // every signed segment's self-id claim is "cannot determine", not "failed". A partial
    // capture may legitimately not carry the node plane yet (this backup run has not
    // captured it), and flagging that as a SelfIdUnbound fault would red-flag a healthy,
    // still-partial medium. This guard looks redundant only until that partial-capture case
    // starts failing verification for no real reason.
    let any_genesis = !genesis.is_empty();

    for (position, seg) in m.segments.iter().enumerate() {
        let mut ok = true;

        // The self-declared index must match where we actually found it, or every fault
        // located below points somewhere that may not exist.
        if seg.index as usize != position {
            faults.push(SegmentFault::IndexMismatch {
                plane: seg.plane,
                position,
                declared: seg.index,
            });
            ok = false;
        }

        if seg.records.is_empty() {
            faults.push(SegmentFault::EmptySegment {
                plane: seg.plane,
                position,
                index: seg.index,
            });
            ok = false;
        }

        if seg.prev_commitment != expected_prev {
            faults.push(SegmentFault::ChainBroken {
                plane: seg.plane,
                position,
                index: seg.index,
                expected: expected_prev.clone(),
                found: seg.prev_commitment.clone(),
            });
            ok = false;
        }

        // An unreadable plane is reported but does NOT retract the chain: the records are
        // readable as bytes, so the commitment below is computable and the segments after it
        // still verify. Naming it is what stops the medium reporting itself fully readable.
        if let Plane::Unknown(plane_tag) = seg.plane {
            faults.push(SegmentFault::UnknownPlane {
                plane_tag,
                position,
                index: seg.index,
                record_count: seg.records.len(),
            });
        }

        match &seg.attestation {
            None => unsigned += 1,
            Some(att) => match crate::attest::verify_segment_attestation(seg) {
                None => {
                    signed_invalid += 1;
                    faults.push(SegmentFault::AttestationInvalid {
                        plane: seg.plane,
                        position,
                        index: seg.index,
                    });
                    ok = false;
                }
                Some(claimed_id) => {
                    signed_valid += 1;
                    if any_genesis {
                        // Infallible in practice: `verify_segment_attestation` above already
                        // verified this exact signature via the same deterministic call, so
                        // re-deriving the body here cannot fail. It is written as a fallible
                        // match anyway because this is a hostile-input recovery path, and an
                        // `.expect` here would hand an attacker a process crash instead of a
                        // refusal.
                        if let Ok(body) = cairn_event::verify_self_described(att) {
                            let attester = body.signer_key_id;
                            let bound = genesis.iter().any(|(gid, gbody)| {
                                *gid == claimed_id && gbody.signer_key_id == attester
                            });
                            if !bound {
                                faults.push(SegmentFault::SelfIdUnbound {
                                    plane: seg.plane,
                                    position,
                                    index: seg.index,
                                    self_node_id_hex: claimed_id,
                                });
                            }
                        }
                    }
                }
            },
        }

        expected_prev = crate::attest::segment_commitment(&seg.records);
        if ok && still_good {
            verified_through = Some(position);
        } else {
            still_good = false;
        }
    }

    ChainReport {
        segments: m.segments.len(),
        signed_valid,
        signed_invalid,
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
/// CLAIM ("I hold through seq 0"); the honest answer to "what do you hold?" when nothing
/// verified is "I do not know". A record legitimately AT seq 0 therefore yields `Some(0)`,
/// which is a different statement from `None` and must stay one.
///
/// # This is a high-water MARK, not a completeness claim
///
/// It is `max`, and `max` proves only "the largest seq I hold is N" — NOT "I hold everything
/// up to N". Nothing here checks contiguity, and a capture that wrote seqs 1,2,3,5,6 yields
/// `Some(6)`: a caller using this as a cursor would then start after 6 and **seq 4 would
/// never be captured, while the medium reported itself complete through 6.** Ask
/// [`seq_gaps`] before treating this as a completeness claim; the gap is reported as data,
/// never silently absorbed (principle 9 — this crate ships the mechanism, the slice that
/// owns capture decides the policy).
///
/// Returns `None` rather than panicking if `report` was computed from a DIFFERENT medium
/// than `m`: the two arguments have no compile-time relationship, and an out-of-range
/// `verified_through` used to index straight off the end of `m.segments` — a panic on a
/// restore path, from a caller error the type system does not catch.
pub fn watermark(m: &MediumV3, report: &ChainReport, plane: Plane) -> Option<i64> {
    let through = report.verified_through?;
    m.segments
        .get(..=through)?
        .iter()
        .filter(|s| s.plane == plane)
        .flat_map(|s| s.records.iter().map(|r| r.source_seq))
        .max()
}

/// Every hole in `plane`'s `source_seq` run over the verified prefix, as `(after, before)`
/// pairs: a gap of `(3, 7)` means seqs 4, 5 and 6 are absent between the 3 and the 7 this
/// medium holds.
///
/// Empty when the run is contiguous — which is what lets a caller treat [`watermark`] as a
/// completeness claim, and only then. This says nothing about seqs BELOW the medium's lowest
/// (a medium that legitimately starts at seq 100 has no gap, it has a floor); it reports only
/// holes between records actually present, which is the part this crate can honestly know.
pub fn seq_gaps(m: &MediumV3, report: &ChainReport, plane: Plane) -> Vec<(i64, i64)> {
    let Some(through) = report.verified_through else {
        return Vec::new();
    };
    let Some(prefix) = m.segments.get(..=through) else {
        return Vec::new();
    };
    let mut seqs: Vec<i64> = prefix
        .iter()
        .filter(|s| s.plane == plane)
        .flat_map(|s| s.records.iter().map(|r| r.source_seq))
        .collect();
    seqs.sort_unstable();
    seqs.dedup();
    seqs.windows(2)
        .filter(|w| w[1] - w[0] > 1)
        .map(|w| (w[0], w[1]))
        .collect()
}

/// Verify every record's SIGNATURE, across every segment, in file order.
///
/// Separate from [`chain_report`] because it answers a different question and one does not
/// imply the other. The chain pass checks attestations and COMMITMENTS, and a commitment is
/// taken over content addresses — which a tampered blob still has, just a different one. In
/// a signed segment that is enough (the commitment fails), but an UNSIGNED segment has no
/// attestation, so without this pass nothing checks its bytes at all.
///
/// Reuses [`verify_events`] unchanged, so a record on a medium faces exactly the check a
/// replicated event faces at the apply door: no second definition of "valid".
///
/// Its `first_bad` is a flat index into every record on the medium in file order — a COUNT,
/// not a location, which is why [`crate::health::MediumHealth`] pairs it with the segment
/// that holds it via [`locate_record`].
pub fn verify_records(m: &MediumV3) -> VerifyReport {
    let events: Vec<Vec<u8>> = m
        .segments
        .iter()
        .flat_map(|s| s.records.iter().map(|r| r.signed_bytes.clone()))
        .collect();
    verify_events(&events)
}

/// Map a flat record ordinal (as [`VerifyReport::first_bad`] reports it) back to the segment
/// that holds it: `(position, plane, index, ordinal_within_segment)`.
///
/// Invariant 9 says a fault is LOCATED, never merely counted — and "record 14372 of 20000"
/// is a count. This is what turns it into "clinical segment 7, its 42nd record".
pub fn locate_record(m: &MediumV3, flat_ordinal: usize) -> Option<(usize, Plane, u32, usize)> {
    let mut seen = 0usize;
    for (position, seg) in m.segments.iter().enumerate() {
        if flat_ordinal < seen + seg.records.len() {
            return Some((position, seg.plane, seg.index, flat_ordinal - seen));
        }
        seen += seg.records.len();
    }
    None
}

/// Which node this medium belongs to, from the LAST verified signed segment.
///
/// Two binds, mirroring the CAIRNB2 marker's: the attestation must verify against its own
/// segment (done in `chain_report`), and the node it names must have a genesis
/// (`node.enrolled`) present on THIS medium, signed by the SAME key that signed the
/// attestation. The second bind is what makes a foreign attestation unusable: only the
/// node that signed its own genesis could have signed this.
///
/// **Returns the ATTESTED id — never `Segment::self_node_id_hex`.** That plaintext field is
/// untrusted and is not bound to the signed one; returning it would hand a caller an
/// attacker-supplied string as an identification, and restore would then record an immutable
/// supersede edge against the wrong node (the exact failure issue #53 exists to prevent).
/// `chain`'s tests pin this with a fixture whose plaintext field deliberately disagrees.
///
/// `None` on any doubt. Fail closed — a withheld identification falls back to an operator
/// choice, whereas a wrong one is unrecoverable.
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

    // `get` rather than `[..=through]`: see `watermark`'s note on a mismatched pair.
    for seg in m.segments.get(..=through)?.iter().rev() {
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

// Tests live in `chain/tests.rs` (Rust's standard non-`mod.rs` sibling-file layout) purely
// to keep this file under the crate's 500-line cap (house rule 4) — no production code
// moved, no public API changed. See that file's header for the full rationale.
#[cfg(test)]
mod tests;
