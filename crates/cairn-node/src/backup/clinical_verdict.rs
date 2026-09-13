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
//!   describes this medium and records a newer clinical watermark than the medium holds
//!   ([`shortfall`]). Maintainer decision, 2026-09-13.
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
    /// A CAIRNB1/CAIRNB2 medium: its format predates the clinical plane entirely.
    pub legacy: bool,
    /// The clinical watermark this node's last backup recorded FOR THIS MEDIUM, or `None`
    /// whenever there is no such evidence. Build it with [`recorded_watermark_for`], which is
    /// what keeps a sidecar about some other drive from counting.
    pub recorded_for_this_medium: Option<i64>,
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
        refusal: shortfall(newest, facts.recorded_for_this_medium)
            .map(|recorded| short_refusal(recorded, newest)),
    }
}

/// PURE. `Some(recorded)` when the medium holds LESS than this node's last backup recorded for
/// it: nothing at all, or a newest seq below the recorded one. `None` when there is no evidence,
/// or the medium holds at least what was recorded.
///
/// Level is complete. AHEAD is also fine: a backup can write the medium durably and then fail to
/// write its sidecar, which leaves the medium holding more than the sidecar says.
pub fn shortfall(medium_newest: Option<i64>, recorded: Option<i64>) -> Option<i64> {
    let recorded = recorded?;
    match medium_newest {
        Some(held) if held >= recorded => None,
        _ => Some(recorded),
    }
}

/// The evidence rule's ONE impure step: the sidecar's clinical watermark, but only when that
/// sidecar describes the medium under test.
///
/// `backup-status.json` is node-global — one file beside the signing key, rewritten by every
/// backup to any path. A sidecar naming another path is a statement about another artifact,
/// possibly another node's (this command does not bind a medium to `--key`'s node), so it is not
/// evidence about this one. `health_describes_medium` canonicalizes paths, which is why this
/// is not pure and why it stays out of [`clinical_plane_verdict`].
pub fn recorded_watermark_for(health: Option<&BackupHealth>, medium: &Path) -> Option<i64> {
    health
        .filter(|h| super::health_describes_medium(&h.medium_path, medium))
        .and_then(|h| h.clinical_watermark)
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

fn short_refusal(recorded: i64, newest: Option<i64>) -> String {
    let held = match newest {
        Some(seq) => format!("clinical records only through seq {seq}"),
        None => "no clinical records at all".to_string(),
    };
    format!(
        "backup SHORT: this node's last backup to this medium recorded clinical events through \
         seq {recorded}, but the medium holds {held}. The file at this path is not what that \
         backup wrote — a truncated copy, or an older one put back in its place — and a restore \
         from it would bring back less than this node last captured. Remedy: run `backup --to` \
         this path again while this node still holds its events, or locate the complete copy. \
         (Rotating drives through one mount point? The drive that missed the latest backup \
         reads SHORT until its own next backup catches it up — and until then it really would \
         restore less.)"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_medium::MediumRecord;

    /// Runtime-derived bytes for a fixture field — never a literal (house rule 6: a byte
    /// literal in a custody field trips CodeQL's hard-coded-cryptographic-value query).
    fn filler(seed: u8, len: usize) -> Vec<u8> {
        (0..len).map(|i| seed.wrapping_add(i as u8)).collect()
    }

    /// One record at `seq`. `variant` changes the custody sidecar, so two records at one seq
    /// with different variants are DIFFERENT records (a straddled re-capture).
    fn rec(seq: i64, variant: Option<u8>) -> MediumRecord {
        MediumRecord {
            signed_bytes: filler(1, 8),
            attestation: None,
            attester_key: None,
            dek_wrapped: variant.map(|v| filler(v, 16)),
            source_seq: seq,
        }
    }

    fn plane(records: Vec<MediumRecord>, collapsed: usize) -> PlaneRecords {
        PlaneRecords {
            records,
            gated_out: 0,
            collapsed,
        }
    }

    fn verdict(acc: &PlaneRecords, legacy: bool, recorded: Option<i64>) -> ClinicalPlaneVerdict {
        clinical_plane_verdict(&ClinicalPlaneFacts {
            accounting: acc,
            legacy,
            recorded_for_this_medium: recorded,
        })
    }

    fn health_for(medium_path: &str, clinical_watermark: Option<i64>) -> BackupHealth {
        BackupHealth {
            version: super::super::SUPPORTED_HEALTH_VERSION,
            last_backup_unix: 0,
            medium_path: medium_path.into(),
            medium_bytes: 0,
            node_events: 1,
            clinical_events: 0,
            clinical_watermark,
            export_covers_seq: None,
            extra: serde_json::Map::new(),
        }
    }

    // --- the shortfall rule, alone -------------------------------------------------------

    #[test]
    fn no_evidence_is_never_a_shortfall() {
        assert_eq!(shortfall(None, None), None);
        assert_eq!(shortfall(Some(40), None), None);
    }

    #[test]
    fn an_empty_medium_against_evidence_is_short() {
        assert_eq!(shortfall(None, Some(40)), Some(40));
    }

    #[test]
    fn a_medium_behind_the_evidence_is_short() {
        assert_eq!(shortfall(Some(39), Some(40)), Some(40));
    }

    /// The boundary: holding exactly what the last backup recorded is complete.
    #[test]
    fn a_medium_level_with_the_evidence_is_not_short() {
        assert_eq!(shortfall(Some(40), Some(40)), None);
    }

    /// A backup that wrote the medium durably and then failed to write its sidecar leaves the
    /// medium AHEAD of the evidence. Holding more than recorded is not a shortfall.
    #[test]
    fn a_medium_ahead_of_the_evidence_is_not_short() {
        assert_eq!(shortfall(Some(41), Some(40)), None);
    }

    // --- the verdict ---------------------------------------------------------------------

    #[test]
    fn records_present_without_evidence_report_count_and_newest_seq() {
        let acc = plane(vec![rec(3, None), rec(5, None), rec(8, None)], 0);
        let v = verdict(&acc, false, None);
        assert_eq!(
            v.summary,
            "clinical-plane records OK: 3 verified, newest seq 8"
        );
        assert_eq!(v.advisory, None);
        assert_eq!(v.refusal, None);
    }

    #[test]
    fn collapsed_re_captures_are_named_in_the_summary() {
        let acc = plane(vec![rec(1, None), rec(2, None)], 4);
        let v = verdict(&acc, false, None);
        assert_eq!(
            v.summary,
            "clinical-plane records OK: 2 verified, newest seq 2, 4 byte-identical \
             re-capture(s) collapsed"
        );
    }

    #[test]
    fn a_straddled_duplicate_is_advisory_and_never_refuses() {
        let acc = plane(vec![rec(7, Some(4)), rec(7, None), rec(8, None)], 0);
        let v = verdict(&acc, false, Some(8));
        let advisory = v
            .advisory
            .expect("two different records at one seq must be named");
        assert!(
            advisory.contains(": 7."),
            "the position is named: {advisory}"
        );
        assert!(
            advisory.contains("A restore would apply all of them"),
            "worded for a check that has applied nothing: {advisory}"
        );
        assert!(
            !advisory.contains("were applied"),
            "restore's past tense would be false here: {advisory}"
        );
        assert_eq!(v.refusal, None, "a straddle is not a restorability failure");
    }

    #[test]
    fn an_empty_plane_without_evidence_says_empty_and_does_not_refuse() {
        let acc = plane(vec![], 0);
        let v = verdict(&acc, false, None);
        assert!(
            v.summary.starts_with("clinical plane: EMPTY — "),
            "{}",
            v.summary
        );
        assert!(
            v.summary.contains("restore NO patient data"),
            "{}",
            v.summary
        );
        assert_eq!(
            v.refusal, None,
            "a fresh clinic's empty plane is a correct backup"
        );
    }

    #[test]
    fn a_legacy_medium_without_evidence_says_none_and_does_not_refuse() {
        let acc = plane(vec![], 0);
        let v = verdict(&acc, true, None);
        assert!(
            v.summary.starts_with("clinical plane: NONE — "),
            "{}",
            v.summary
        );
        assert!(v.summary.contains("CAIRNB1/CAIRNB2"), "{}", v.summary);
        assert_eq!(v.refusal, None);
    }

    #[test]
    fn an_empty_plane_against_evidence_refuses_naming_both_sides() {
        let acc = plane(vec![], 0);
        let v = verdict(&acc, false, Some(4812));
        let refusal = v
            .refusal
            .expect("the sidecar proves this path held clinical events");
        assert!(refusal.starts_with("backup SHORT: "), "{refusal}");
        assert!(refusal.contains("through seq 4812"), "{refusal}");
        assert!(refusal.contains("no clinical records at all"), "{refusal}");
        assert!(
            refusal.contains("run `backup --to`"),
            "the remedy: {refusal}"
        );
        assert!(
            v.summary.starts_with("clinical plane: EMPTY"),
            "the plane line still prints first: {}",
            v.summary
        );
    }

    #[test]
    fn a_plane_behind_evidence_refuses_naming_both_seqs() {
        let acc = plane(vec![rec(1, None), rec(30, None)], 0);
        let refusal = verdict(&acc, false, Some(40)).refusal.expect("30 < 40");
        assert!(refusal.contains("through seq 40"), "{refusal}");
        assert!(refusal.contains("only through seq 30"), "{refusal}");
    }

    /// A legacy file at a path where this node last wrote clinical events is not what that
    /// backup wrote either: `backup` converts a legacy medium to CAIRNB3 on its next capture.
    #[test]
    fn a_legacy_medium_against_evidence_refuses() {
        let acc = plane(vec![], 0);
        assert!(verdict(&acc, true, Some(12)).refusal.is_some());
    }

    #[test]
    fn a_plane_level_with_evidence_does_not_refuse() {
        let acc = plane(vec![rec(40, None)], 0);
        assert_eq!(verdict(&acc, false, Some(40)).refusal, None);
    }

    // --- the evidence adapter ------------------------------------------------------------

    #[test]
    fn a_sidecar_describing_this_medium_is_evidence() {
        let h = health_for("/nonexistent-567/cairn.medium", Some(40));
        assert_eq!(
            recorded_watermark_for(Some(&h), Path::new("/nonexistent-567/cairn.medium")),
            Some(40)
        );
    }

    /// The case only a node-global sidecar can create: a rotation drive at another path.
    /// That sidecar is about some other artifact — possibly another node's — never this one.
    #[test]
    fn a_sidecar_describing_another_medium_is_not_evidence() {
        let h = health_for("/nonexistent-567/drive-a.medium", Some(40));
        assert_eq!(
            recorded_watermark_for(Some(&h), Path::new("/nonexistent-567/drive-b.medium")),
            None
        );
    }

    #[test]
    fn no_sidecar_or_no_recorded_watermark_is_not_evidence() {
        let medium = Path::new("/nonexistent-567/cairn.medium");
        assert_eq!(recorded_watermark_for(None, medium), None);
        let v1_shaped = health_for("/nonexistent-567/cairn.medium", None);
        assert_eq!(recorded_watermark_for(Some(&v1_shaped), medium), None);
    }
}
