//! "Is this medium sound?" — the one composed verdict, and the only function in this crate a
//! caller should reach for to decide anything.
//!
//! WHY THIS MODULE EXISTS (#500 slice 2a review). Every other verdict in this crate is a
//! PARTIAL answer, and each one returns `true` for a medium that is not sound:
//!
//! | partial verdict            | says nothing about                                    |
//! |----------------------------|-------------------------------------------------------|
//! | `ChainReport::chain_intact`| record signatures; a torn tail; an empty medium        |
//! | `VerifyReport::all_intact` | the chain; vacuously `true` at 0 of 0 records          |
//! | `MediumV3::truncated_tail` | everything else                                       |
//!
//! Four inputs made the loose functions report a healthy medium that was not one — an empty
//! 8-byte file, a medium missing an entire plane, a medium with a torn tail, and a medium
//! with a tampered record in its last unsigned segment. In each case every individual
//! function was honest and the COMPOSITE was a precise untruth. That is issue #500's own
//! signature, reproduced inside the format built to prevent it.
//!
//! This codebase has paid for that shape twice already. `cairn-node`'s `verify-backup` command
//! carries the scar and the rule, and this module is that rule made structural:
//!
//! > "Is this medium internally consistent" and "is this medium worth anything" are different
//! > questions, and only the second belongs to the health check.
//!
//! So [`MediumHealth`] answers BOTH, separately and by name: [`MediumHealth::sound`] for
//! consistency, [`MediumHealth::carries_nothing`] for worth. Neither can be read as the other,
//! and a caller cannot reach a verdict without having been handed both.

use crate::chain::{chain_report, locate_record, verify_records, ChainReport, SegmentFault};
use crate::container::MediumV3;
use crate::segment::Plane;
use crate::verify::VerifyReport;

/// Where a bad record actually sits, rather than its ordinal among all records on the medium.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordLocation {
    /// Index into `MediumV3::segments` — where the reader found it. Trustworthy.
    pub position: usize,
    pub plane: Plane,
    /// The segment's self-declared index. See [`SegmentFault`] on why this is not the same
    /// thing as `position` and is not trustworthy on its own.
    pub index: u32,
    /// Which record within that segment, from 0.
    pub ordinal_in_segment: usize,
}

/// Everything this crate can determine about one medium, in one place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediumHealth {
    /// The structural pass: chain links, attestations, planes, identity claims.
    pub chain: ChainReport,
    /// The signature pass over every record on the medium.
    pub records: VerifyReport,
    /// The first record whose signature failed, LOCATED — not merely counted.
    pub first_bad_record: Option<RecordLocation>,
    /// The final section was cut short: an interrupted append. Everything before it is
    /// intact. Remedy: run the backup again — the opposite of a damaged medium's remedy.
    pub truncated_tail: bool,
    /// Offset one past the last COMPLETE section. A writer recovering from a torn medium
    /// MUST truncate to this before appending; see [`MediumV3::complete_bytes`].
    pub complete_bytes: usize,
    /// Records carried by planes this build cannot route. These are NOT counted as verified
    /// or unverified by anything else here — naming them is what keeps "1000 of 1000 intact"
    /// from being true and materially misleading on a medium holding 6000 records.
    pub records_in_unknown_planes: usize,
}

impl MediumHealth {
    /// Every check this crate can make, passed.
    ///
    /// Deliberately conjunctive and deliberately strict: a torn tail makes a medium
    /// un-sound even though its remedy is mild, because "sound" must never be true for a
    /// medium a caller would be wrong to treat as complete. Callers that want to distinguish
    /// the mild case read [`MediumHealth::truncated_tail`] — the whole point of this type is
    /// that they were handed it.
    ///
    /// NOTE what this still does not claim: soundness is not COMPLETENESS. A medium can be
    /// perfectly sound and hold only half a clinic's events, because nothing on the medium
    /// knows what was supposed to be on it. See [`MediumHealth::carries_nothing`] for the
    /// only completeness question answerable from the bytes alone, and `chain::seq_gaps` for
    /// holes in a plane's cursor run.
    pub fn sound(&self) -> bool {
        self.chain.chain_intact() && self.records.all_intact() && !self.truncated_tail
    }

    /// This medium would restore nothing at all.
    ///
    /// Kept OUT of [`MediumHealth::sound`] on purpose, mirroring the rule `cairn-node`'s
    /// `verify-backup` already learned the hard way (#502 item 2): an internally-consistent
    /// empty medium IS internally consistent, and a node that genuinely holds no events yet
    /// must still be able to write its first medium. But a health check that reports "OK"
    /// over an artifact that restores nothing is a green light on an empty file, whose only
    /// other refusal comes at the disaster, after the disk is gone. So the two questions are
    /// both answered, and neither is allowed to stand in for the other.
    pub fn carries_nothing(&self) -> bool {
        self.records.total == 0 && self.records_in_unknown_planes == 0
    }

    /// The medium is structurally fine but this build cannot read all of it — a NEWER Cairn
    /// wrote a plane we do not know. The remedy is "upgrade this node", never "fetch another
    /// copy", and never "run the backup again": appending to it would write against an
    /// incomplete picture.
    pub fn needs_a_newer_build(&self) -> bool {
        self.chain
            .faults
            .iter()
            .any(|f| matches!(f, SegmentFault::UnknownPlane { .. }))
    }
}

/// Assess a whole medium: the chain pass, the signature pass, the tail, and the planes this
/// build cannot route — composed into one verdict a caller cannot partially read.
///
/// This is the documented entry point. `chain_report` and `verify_records` remain public
/// because an operator surface legitimately wants the detail, but each answers only its own
/// question, and reaching a conclusion from one alone is the defect this function exists to
/// make hard.
pub fn assess(m: &MediumV3) -> MediumHealth {
    let chain = chain_report(m);
    let records = verify_records(m);
    let first_bad_record = records.first_bad.and_then(|flat| {
        locate_record(m, flat).map(
            |(position, plane, index, ordinal_in_segment)| RecordLocation {
                position,
                plane,
                index,
                ordinal_in_segment,
            },
        )
    });
    let records_in_unknown_planes = m
        .segments
        .iter()
        .filter(|s| !s.plane.is_known())
        .map(|s| s.records.len())
        .sum();
    MediumHealth {
        chain,
        records,
        first_bad_record,
        truncated_tail: m.truncated_tail,
        complete_bytes: m.complete_bytes,
        records_in_unknown_planes,
    }
}

#[cfg(test)]
mod tests;
