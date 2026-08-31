//! The chain pass, the watermark, and self-identification for a CAIRNB3 medium (issue
//! #500 slice 2a).
//!
//! WHY A SEPARATE MODULE FROM `verify`: `verify.rs` answers one question — "do these
//! bytes verify" — a flat, whole-set signature check with no notion of segments, chains,
//! or planes. This module answers a different one — "what can this WHOLE MEDIUM be
//! trusted for" — which needs the chain: attestations, commitments, predecessor links,
//! per-plane watermarks, and which node the medium belongs to. This is the same seam the
//! project already used when `attest.rs` was split out of `segment.rs`: a responsibility
//! boundary (contents+framing vs. tamper-evidence), not a line-count cut — though the
//! 500-line cap is also what forced the timing here (task 8 review, #500).
//!
//! FAULT REPORTING: [`SegmentFault`] is the vocabulary of everything that can be wrong
//! with a signed, chained medium. Every variant carries the segment's PLANE and INDEX —
//! the standing rule in this codebase is NAME, NEVER COUNT. [`chain_report`] is the one
//! function that walks every segment and constructs faults; the helpers below
//! ([`watermark`], [`self_id_from_chain`]) READ a `ChainReport`, they never redecide it.

use crate::container::MediumV3;
use crate::segment::Plane;
use crate::verify::{verify_events, VerifyReport};

/// One thing wrong with one segment. Every variant carries the segment's PLANE and INDEX:
/// "clinical segment 7 breaks the chain" sends an operator somewhere, "chain invalid"
/// does not — and the standing rule in this codebase is NAME, NEVER COUNT.
///
/// NOTE on the absence of a `CommitmentMismatch` case: `attest::verify_segment_attestation`
/// folds the commitment comparison into its own verdict, so a records-only mismatch already
/// surfaces as `AttestationInvalid` — a separate `CommitmentMismatch` variant would be
/// unreachable by construction today (task 8 review, #500). Distinguishing "the records
/// changed" from "the signature itself changed" IS a strictly better operator diagnosis, but
/// nothing consumes that distinction until the health and verify-backup surfaces land in a
/// later slice. The finer split is deliberately DEFERRED, not forgotten — reintroduce it
/// together with the surface that displays it, not before.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SegmentFault {
    /// The attestation is present but does not verify against this segment (tampered, the
    /// segment was moved in the chain, or its records no longer hash to what was attested
    /// — see the note above on why a bare records mismatch collapses into this variant).
    AttestationInvalid { plane: Plane, index: u32 },
    /// `prev_commitment` does not match the preceding segment's commitment.
    ChainBroken {
        plane: Plane,
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
    /// Position (into `MediumV3::segments`) of the last segment whose chain link held (and
    /// whose attestation, when SIGNED, also verified and bound to a genesis where one
    /// exists) with every preceding one. `None` when the first segment already fails.
    ///
    /// WHAT THIS DOES NOT MEAN (I5, #500 final review — an earlier doc here overclaimed
    /// "everything at or before this is trustworthy"): `verified_through` is STRUCTURAL
    /// CHAIN CONTINUITY, not a tamper-evidence boundary. Two ways it is weaker than that:
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
    ///      identity CLAIM is forged (`self_node_id_hex` is attacker-supplied, not
    ///      derived). Only "which node captured this" is in doubt, which is what
    ///      [`self_id_from_chain`] (not this field) answers.
    ///
    /// What this DOES bound honestly: a torn or structurally-broken tail costs exactly the
    /// increments after this point, never more, and never silently more than that.
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
///
/// This is also where the SELF-ID bind is checked (task 8 review, #500): for every SIGNED
/// segment whose attestation verifies, the id it claims must belong to a genesis actually
/// present on this medium, signed by the SAME key — the same second bind `self_id_from_chain`
/// uses to decide which claim to trust. Checking it HERE, not only there, is what turns a
/// forged claim into a LOCATED `SelfIdUnbound { plane, index, .. }` instead of a bare `None`
/// that tells an operator nothing: a forged identity claim is the single most
/// security-relevant failure this file can report.
pub fn chain_report(m: &MediumV3) -> ChainReport {
    let mut faults = Vec::new();
    let mut signed = 0;
    let mut unsigned = 0;
    let mut verified_through = None;
    let mut expected_prev = String::new();
    let mut still_good = true;

    // Every verified genesis anywhere on the medium, gathered ONCE up front — a segment's
    // self-id can bind to a genesis anywhere in file order, not only to one already walked.
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
            Some(att) => {
                signed += 1;
                match crate::attest::verify_segment_attestation(seg) {
                    None => {
                        faults.push(SegmentFault::AttestationInvalid {
                            plane: seg.plane,
                            index: seg.index,
                        });
                        ok = false;
                    }
                    Some(claimed_id) if any_genesis => {
                        // Infallible in practice: verify_segment_attestation above already
                        // verified this exact signature via the same call, so re-deriving
                        // the signed body here should never fail. But this is a
                        // hostile-input recovery path (minor, #500 final review) — an
                        // `.expect` here would hand an attacker a process crash instead of
                        // a refusal. Degrade to "cannot confirm the bind" instead, the same
                        // way `self_id_from_chain`'s sibling handles this below.
                        if let Ok(body) = cairn_event::verify_self_described(att) {
                            let attester = body.signer_key_id;
                            let bound = genesis.iter().any(|(gid, gbody)| {
                                *gid == claimed_id && gbody.signer_key_id == attester
                            });
                            if !bound {
                                faults.push(SegmentFault::SelfIdUnbound {
                                    plane: seg.plane,
                                    index: seg.index,
                                    self_node_id_hex: claimed_id,
                                });
                            }
                        }
                    }
                    Some(_) => {} // attestation verifies; no genesis anywhere to bind to yet
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
/// Reuses [`verify_events`] unchanged, so a record on a medium faces exactly the check a
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

// Tests live in `chain/tests.rs` (Rust's standard non-`mod.rs` sibling-file layout) purely
// to keep this file under the crate's 500-line cap (house rule 4) — no production code
// moved, no public API changed. See that file's header for the full rationale.
#[cfg(test)]
mod tests;
