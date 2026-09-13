//! What `verify-backup` says about a medium's CLINICAL plane, and when it refuses (#567).
//!
//! # Why this exists
//!
//! Since #554 slice 2d, `restore` applies both planes. `verify-backup` — the cron health check
//! whose one job is *"can I still recover from this medium?"* — reported the federation plane
//! alone, so an operator could read green and rotate a drive whose clinical plane was empty.
//!
//! # What it decides
//!
//! One pure function, [`clinical_plane_verdict`], so the whole policy is testable with no
//! medium, no database and no CLI:
//!
//! - a **summary line** saying what the clinical half of a restore would bring back;
//! - an **advisory** when two DIFFERENT records share a `source_seq` (a re-capture that straddled
//!   a custody change). It never changes the exit code — as in `restore`;
//! - a **refusal**, `backup SHORT`, ONLY ON EVIDENCE: this node's own `backup-status.json`
//!   describes this path, and the medium holds less than that backup recorded — an older newest
//!   clinical seq, or fewer clinical records ([`shortfall`]). Maintainer decision, 2026-09-13;
//!   the record-count axis was added by the branch's final review, 2026-09-14.
//!
//! # What it deliberately does not do
//!
//! (Design: `docs/superpowers/specs/2026-09-13-verify-backup-clinical-plane-design.md`.)
//!
//! - **Warn about records past the last verified chain link.** `verify-backup` has already
//!   refused such a medium as UNSOUND before this runs; `cairn-medium`'s
//!   `a_medium_that_gates_records_out_is_never_sound` pins why that holds.
//! - **Report holes in the `source_seq` run.** Every duplicate apply burns an IDENTITY value, so
//!   holes are routine on a federating node and a gap warning would fire forever (#549).
//! - **Fail an empty plane without evidence.** From the bytes alone a fresh clinic's empty plane
//!   and a copy cut at a section boundary look identical, and only one of them is a bad backup.

use std::path::Path;

use cairn_medium::PlaneRecords;

use super::BackupHealth;

/// The facts `verify-backup` has gathered about one medium's clinical plane. Built by the
/// caller, which alone touches the filesystem; everything downstream of this is pure.
pub struct ClinicalPlaneFacts<'a> {
    /// The trusted records and the derivation's own arithmetic, from
    /// [`super::clinical_plane_accounting`] — never re-derived here.
    pub accounting: &'a PlaneRecords,
    /// EVERY clinical record on the medium, counted raw — byte-identical re-captures included —
    /// from [`super::plane_counts`]. That is the same arithmetic `backup` recorded as the
    /// sidecar's `clinical_events`, so the two compare with no reconciliation.
    pub medium_clinical_records: usize,
    /// A CAIRNB1/CAIRNB2 medium: its format predates the clinical plane entirely.
    pub legacy: bool,
    /// What this node's last backup TO THIS PATH recorded, or `None` when no sidecar describes
    /// this path. Build it with [`last_backup_evidence_for`], which is what keeps a sidecar
    /// about some other drive from counting.
    pub evidence: Option<LastBackupEvidence>,
}

/// What this node's own last backup to one path recorded about the clinical plane — the only
/// evidence `backup SHORT` may act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LastBackupEvidence {
    /// The newest clinical `source_seq` that backup recorded (the sidecar's
    /// `clinical_watermark`). `None` when it recorded none.
    pub newest_seq: Option<i64>,
    /// How many clinical records that backup's medium held, counted raw (the sidecar's
    /// `clinical_events`). `None` for a sidecar older than
    /// [`super::SUPPORTED_HEALTH_VERSION`]: those never recorded a count, and serde defaults
    /// the field to 0 — a 0 nobody wrote is a claim, not a fact.
    pub clinical_records: Option<u64>,
}

/// Which way(s) a medium falls short of the evidence. [`shortfall`] only ever returns one with
/// at least one field set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shortfall {
    /// `Some(recorded)` when the medium's newest clinical seq is absent or below `recorded`.
    pub newest_seq: Option<i64>,
    /// `Some(recorded)` when the medium holds fewer raw clinical records than `recorded`.
    pub clinical_records: Option<u64>,
}

/// What to print, and whether to fail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClinicalPlaneVerdict {
    /// One line for stdout, always present: the clinical half of the command's claim.
    pub summary: String,
    /// A warning for stderr that does not change the exit code.
    pub advisory: Option<String>,
    /// When `Some`, the command must fail with exactly this message (it starts `backup SHORT:`).
    pub refusal: Option<String>,
}

/// PURE. Decide everything `verify-backup` says about the clinical plane. See the module docs.
pub fn clinical_plane_verdict(facts: &ClinicalPlaneFacts<'_>) -> ClinicalPlaneVerdict {
    let newest = newest_seq(facts.accounting);
    ClinicalPlaneVerdict {
        summary: summary_line(facts, newest),
        advisory: straddled_advisory(facts.accounting),
        refusal: shortfall(newest, facts.medium_clinical_records, facts.evidence)
            .map(|short| short_refusal(&short, newest, facts.medium_clinical_records)),
    }
}

/// PURE. `Some` when the medium holds LESS than this node's last backup to this path recorded,
/// on either axis; `None` when there is no evidence, or the medium holds at least what was
/// recorded on both.
///
/// **Why two axes.** The newest seq alone misses a medium that is short BELOW its newest seq.
/// A capture backfills late-committing holes under the watermark (`capture::plane`), so night 1
/// can capture seqs 1–100 while 97 is still uncommitted and night 2 add only 97: the night-1
/// copy put back has the same newest seq and one record fewer. The raw record count sees that.
/// It compares raw against raw — the sidecar's `clinical_events` is `plane_counts` over the
/// medium `backup` wrote, and [`ClinicalPlaneFacts::medium_clinical_records`] is `plane_counts`
/// over the file under test — so duplicates need no reconciliation. The count axis is only
/// consulted when the sidecar recorded one (v2 and newer); the seq axis stays for every sidecar.
///
/// Level is complete on each axis. AHEAD is also fine on each: a backup can write the medium
/// durably and then fail to write its sidecar, which leaves the medium holding more than the
/// sidecar says. The axes are independent — ahead on one never excuses short on the other.
pub fn shortfall(
    medium_newest: Option<i64>,
    medium_records: usize,
    evidence: Option<LastBackupEvidence>,
) -> Option<Shortfall> {
    let evidence = evidence?;
    let short = Shortfall {
        newest_seq: newest_seq_short(medium_newest, evidence.newest_seq),
        clinical_records: record_count_short(medium_records, evidence.clinical_records),
    };
    (short.newest_seq.is_some() || short.clinical_records.is_some()).then_some(short)
}

/// PURE. The newest-seq axis: `Some(recorded)` when the medium has no newest seq, or one below
/// what was recorded.
fn newest_seq_short(medium_newest: Option<i64>, recorded: Option<i64>) -> Option<i64> {
    let recorded = recorded?;
    match medium_newest {
        Some(held) if held >= recorded => None,
        _ => Some(recorded),
    }
}

/// PURE. The record-count axis: `Some(recorded)` when the medium holds fewer raw records.
fn record_count_short(medium_records: usize, recorded: Option<u64>) -> Option<u64> {
    let recorded = recorded?;
    // `usize` is at most 64 bits on every target this crate builds for, so the cast is exact.
    ((medium_records as u64) < recorded).then_some(recorded)
}

/// The evidence rule's ONE impure step: what the sidecar recorded, but only when that sidecar
/// describes the medium under test.
///
/// `backup-status.json` is node-global — one file beside the signing key, rewritten by every
/// backup to any path. A sidecar naming another path is a statement about another artifact,
/// possibly another node's (this command does not bind a medium to `--key`'s node), so it is not
/// evidence about this one. `health_describes_medium` canonicalizes paths, which is why this
/// is not pure and why it stays out of [`clinical_plane_verdict`].
pub fn last_backup_evidence_for(
    health: Option<&BackupHealth>,
    medium: &Path,
) -> Option<LastBackupEvidence> {
    health
        .filter(|h| super::health_describes_medium(&h.medium_path, medium))
        .map(evidence_in)
}

/// PURE. The two facts one sidecar recorded, with the version rule applied: a sidecar older
/// than v2 recorded no per-plane count, so its serde-default 0 is not passed on as evidence
/// (the same rule `describe_health` applies before rendering one).
fn evidence_in(health: &BackupHealth) -> LastBackupEvidence {
    LastBackupEvidence {
        newest_seq: health.clinical_watermark,
        clinical_records: (health.version >= super::SUPPORTED_HEALTH_VERSION)
            .then_some(health.clinical_events),
    }
}

/// The newest trusted clinical `source_seq`: the same number `cairn_medium::watermark` returns
/// over the same verified prefix, which is what `backup` recorded in the sidecar.
fn newest_seq(accounting: &PlaneRecords) -> Option<i64> {
    accounting.records.iter().map(|r| r.source_seq).max()
}

fn summary_line(facts: &ClinicalPlaneFacts<'_>, newest: Option<i64>) -> String {
    match newest {
        Some(seq) => {
            let collapsed = match facts.accounting.collapsed {
                0 => String::new(),
                k => format!(", {k} byte-identical re-capture(s) collapsed"),
            };
            format!(
                "clinical-plane records OK: {} verified, newest seq {seq}{collapsed}",
                facts.accounting.records.len()
            )
        }
        None if facts.legacy => "clinical plane: NONE — this CAIRNB1/CAIRNB2 medium predates \
                                 the clinical plane and carries no patient data at all. If \
                                 this node holds charts, they are NOT on this medium."
            .to_string(),
        None => "clinical plane: EMPTY — this medium would restore NO patient data. If this \
                 node holds charts, they are NOT on this medium."
            .to_string(),
    }
}

/// The straddled-duplicate finding, worded for a check that has applied nothing. `restore`'s
/// [`super::straddled_duplicate_notice`] says the copies "were applied", which would be false here.
fn straddled_advisory(accounting: &PlaneRecords) -> Option<String> {
    let repeated = super::straddled_positions(&accounting.records);
    if repeated.is_empty() {
        return None;
    }
    Some(format!(
        "WARNING: this medium holds two or more DIFFERENT records at the same source \
         position(s): {}. A byte-identical re-capture is collapsed silently and is expected; \
         these differ — typically a capture that straddled an unwrap-key rotation or a \
         crypto-shred, so the copies disagree about CUSTODY. A restore would apply all of them \
         (the apply door is idempotent and refuses custody for an already-shredded target, so \
         nothing erased can come back). This does not fail the check, but review these \
         positions: only you can tell which capture reflects what this node actually held.",
        super::describe_positions(&repeated)
    ))
}

/// The `backup SHORT` message: one sentence per axis that fell short, then what it means and
/// what to do.
///
/// Worded so every clause is true in every case that reaches it (principle 4):
///
/// - **"to this path"**, never "to this medium": the evidence is about a path, and in a rotation
///   the last backup there went to a different drive.
/// - **"newest clinical seq N"**, never "through seq N": the latter implies no gaps below N,
///   which is exactly what a below-watermark backfill makes false.
/// - **A certain loss only when the newest seq is short.** Then the newest recorded event is not
///   on this medium. On the count axis alone the missing records could all have been re-captures
///   (byte-identical or with different custody) of records still present, which a restore
///   collapses or applies regardless.
fn short_refusal(short: &Shortfall, newest: Option<i64>, medium_records: usize) -> String {
    let mut findings = Vec::new();
    if let Some(recorded) = short.newest_seq {
        findings.push(format!(
            "That backup recorded newest clinical seq {recorded}; {}.",
            medium_newest_clause(newest, medium_records)
        ));
    }
    if let Some(recorded) = short.clinical_records {
        findings.push(format!(
            "That backup recorded {recorded} clinical record(s); this medium holds \
             {medium_records}."
        ));
    }
    let consequence = if short.newest_seq.is_some() {
        "a restore from it would bring back less than this node last captured"
    } else {
        "a restore from it would bring back less than this node last captured, unless every \
         missing record was a re-capture (byte-identical or with different custody) of one \
         still present"
    };
    let rotation_suffix = if short.newest_seq.is_some() {
        " — and until then it really would restore less"
    } else {
        " — and until then it may restore less"
    };
    format!(
        "backup SHORT: this medium holds less than this node's last backup to this path \
         recorded. {} The file at this path is not what that backup wrote — typically a \
         truncated copy, or an older one put back in its place — and {consequence}. Remedy: run \
         `backup --to` this path again while this node still holds its events, or locate the \
         complete copy. (Rotating drives through one mount point? The drive that missed the \
         latest backup reads SHORT until its own next backup catches it up{rotation_suffix}.)",
        findings.join(" ")
    )
}

/// What the medium holds on the newest-seq axis, as the second half of that sentence.
///
/// "No clinical records at all" is keyed on the RAW count, because that is what it claims. On a
/// sound medium an absent newest seq always means zero raw records, but this function cannot
/// see soundness, so raw records with none verified get a sentence that says exactly that.
fn medium_newest_clause(newest: Option<i64>, medium_records: usize) -> String {
    match (newest, medium_records) {
        (Some(seq), _) => format!("this medium's newest clinical seq is {seq}"),
        (None, 0) => "this medium holds no clinical records at all".to_string(),
        (None, n) => format!("none of this medium's {n} clinical record(s) is verified"),
    }
}

#[cfg(test)]
mod tests;
