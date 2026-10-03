//! The one definition of what a single sign-off gesture attests (#288).
use crate::row::{MedicationRow, MedicationStatus, WrongChartReasons};
use uuid::Uuid;

/// Whether a displayed line is one this gesture may sign at all.
///
/// Three exclusions, for different reasons:
///
/// - **Ceased.** A struck line on a paper chart is not re-signed. Ceased rows stay visible
///   for parity, but they are never targets.
/// - **Cross-patient.** The row's group holds member threads belonging to more than one
///   patient (issue #334). `patient_medication_current` takes the line's dose from
///   `medication_group_current_dose`, which picks one member across the WHOLE group
///   regardless of patient — so this line can display the other patient's dose over this
///   patient's drug name. A signature is a claim of responsibility for what the line SAYS,
///   and the node knows it may be saying something it cannot stand behind.
///
/// - **Doubted link (#697 (b)).** The set's links are in doubt (db/054: a link the hard
///   identity check clashes on, or a clinician's unlink between charts other links still join)
///   and this line is not recorded only on the opened chart: a signature is a claim about a
///   person, and the node has positive evidence the other member may be someone else.
///
/// All of these are line-level. The rest of the chart stays signable — see `withheld_rows`.
fn is_signable_line(row: &MedicationRow) -> bool {
    row.status == MedicationStatus::Active && !row.is_wrong_chart_hazard()
}

/// Which threads a single sign-off gesture attests.
///
/// Paper drug-chart semantics: each drug line carries the signature of the person
/// responsible for THAT drug, so a thread already holding a non-stale vouch keeps its
/// existing signatory untouched. Returns THREAD ids, not group ids, because ADR-0049
/// attestation is per-thread.
///
/// The result is sorted and deduplicated. That is not cosmetic: the orchestrator mints one
/// HLC per target in this order, so an unstable order would make two runs over the same
/// list assign different HLCs to the same threads.
pub fn sign_off_targets(rows: &[MedicationRow]) -> Vec<Uuid> {
    let mut targets: Vec<Uuid> = rows
        .iter()
        .filter(|row| is_signable_line(row))
        .flat_map(|row| row.members.iter())
        .filter(|member| member.vouch.needs_signature())
        .map(|member| member.medication_id)
        .collect();
    targets.sort();
    targets.dedup();
    targets
}

/// One displayed line that still needs a signature and will deliberately not get one, with
/// the reasons it is withheld — so every surface can word each reason with its own remedy
/// (#697), before the gesture and after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WithheldLine {
    pub group_id: Uuid,
    pub reasons: WrongChartReasons,
}

/// The lines this gesture deliberately leaves UNSIGNED although they still need a
/// signature — returned as GROUP ids, because a group is the line the clinician sees.
///
/// WHY THIS IS A SEPARATE, PUBLIC FUNCTION. Withholding a line silently would be the worst
/// of both worlds: the clinician sees "signed off 11 medications", assumes the chart is
/// done, and the twelfth drug stays unvouched with nobody aware of it. So the caller is
/// handed the list and is expected to say so out loud. It lives here, beside
/// `sign_off_targets`, so the rule for *what is signed* and the rule for *what is reported
/// as not signed* cannot drift apart — the same reason the whole crate exists.
///
/// Each line carries its reasons, so a renderer never words a doubted-link line with the
/// separation remedy.
///
/// Only lines that would OTHERWISE have been signed are reported. A cross-patient line
/// everyone has already vouched is not an outstanding action, and warning about it would
/// train the reader to ignore the warning.
pub fn withheld_rows(rows: &[MedicationRow]) -> Vec<WithheldLine> {
    let mut withheld: Vec<WithheldLine> = rows
        .iter()
        .filter(|row| row.status == MedicationStatus::Active)
        .filter(|row| row.members.iter().any(|m| m.vouch.needs_signature()))
        .filter_map(|row| {
            row.withheld_because().map(|reasons| WithheldLine {
                group_id: row.group_id,
                reasons,
            })
        })
        .collect();
    withheld.sort_by_key(|line| line.group_id);
    withheld.dedup_by_key(|line| line.group_id);
    withheld
}

/// The group ids of `lines`, in order — the argument `format_hazard_groups` takes.
pub fn withheld_group_ids(lines: &[WithheldLine]) -> Vec<Uuid> {
    lines.iter().map(|line| line.group_id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::row::{MedicationRow, MedicationStatus, MemberVouch, VouchState, WrongChartReasons};

    /// Deterministic uuids without a random source: `Uuid::from_u128` makes each id
    /// readable in a failure message and stable across runs.
    fn uid(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn row(group: u128, status: MedicationStatus, members: Vec<MemberVouch>) -> MedicationRow {
        MedicationRow {
            group_id: uid(group),
            display_chart: uid(999),
            term: "metformin".into(),
            coding_display: None,
            formulation: None,
            dose_amount: None,
            dose_unit: None,
            sig: None,
            started_value: None,
            started_precision: None,
            status,
            members,
            reconciliation_flagged: false,
            coding_conflict: false,
            cross_patient: false,
            wrong_chart: WrongChartReasons::default(),
            source_charts: vec![uid(999)],
        }
    }

    fn member(id: u128, vouch: VouchState) -> MemberVouch {
        MemberVouch {
            medication_id: uid(id),
            vouch,
            patient_id: uid(999),
        }
    }

    #[test]
    fn an_unvouched_thread_is_a_target() {
        let rows = vec![row(
            1,
            MedicationStatus::Active,
            vec![member(1, VouchState::Absent)],
        )];
        assert_eq!(sign_off_targets(&rows), vec![uid(1)]);
    }

    #[test]
    fn a_stale_vouch_is_a_target() {
        let rows = vec![row(
            1,
            MedicationStatus::Active,
            vec![member(1, VouchState::Stale { by: "abc".into() })],
        )];
        assert_eq!(sign_off_targets(&rows), vec![uid(1)]);
    }

    /// THE load-bearing case (#288, drug-chart semantics): another clinician's current
    /// signature on a drug line is left exactly as it is. Signing it over would silently
    /// move responsibility for that drug from them to the current user.
    #[test]
    fn a_fresh_vouch_by_someone_else_is_left_alone() {
        let rows = vec![row(
            1,
            MedicationStatus::Active,
            vec![member(1, VouchState::Fresh { by: "dr_b".into() })],
        )];
        assert!(sign_off_targets(&rows).is_empty());
    }

    /// A reconciled group is ONE displayed row over several threads; only the members
    /// that actually need a signature are signed.
    #[test]
    fn a_mixed_freshness_group_targets_only_its_needy_members() {
        let rows = vec![row(
            1,
            MedicationStatus::Active,
            vec![
                member(1, VouchState::Fresh { by: "dr_b".into() }),
                member(2, VouchState::Stale { by: "dr_b".into() }),
                member(3, VouchState::Absent),
            ],
        )];
        assert_eq!(sign_off_targets(&rows), vec![uid(2), uid(3)]);
    }

    /// A struck line on a paper chart is not re-signed. Ceased rows stay VISIBLE
    /// (refinement 2) but are never targets.
    #[test]
    fn ceased_rows_are_never_targets() {
        let rows = vec![row(
            1,
            MedicationStatus::Ceased,
            vec![member(1, VouchState::Absent)],
        )];
        assert!(sign_off_targets(&rows).is_empty());
    }

    #[test]
    fn an_empty_list_yields_no_targets() {
        assert!(sign_off_targets(&[]).is_empty());
    }

    /// The order is what assigns HLCs in the orchestrator, so it must not depend on
    /// row order or on how many groups a thread appears under.
    #[test]
    fn targets_are_sorted_and_deduplicated() {
        let rows = vec![
            row(
                9,
                MedicationStatus::Active,
                vec![member(9, VouchState::Absent)],
            ),
            row(
                2,
                MedicationStatus::Active,
                vec![member(2, VouchState::Absent), member(2, VouchState::Absent)],
            ),
            // The same thread (medication_id 5) shows up as a member of two DISTINCT
            // groups. Dedup must hold across groups, not just within one row's member
            // list — a thread's id, not its group, is the identity that matters here.
            row(
                3,
                MedicationStatus::Active,
                vec![member(5, VouchState::Absent)],
            ),
            row(
                4,
                MedicationStatus::Active,
                vec![member(5, VouchState::Absent)],
            ),
        ];
        assert_eq!(sign_off_targets(&rows), vec![uid(2), uid(5), uid(9)]);
    }

    #[test]
    fn needs_signature_covers_absent_and_stale_only() {
        assert!(VouchState::Absent.needs_signature());
        assert!(VouchState::Stale { by: "x".into() }.needs_signature());
        assert!(!VouchState::Fresh { by: "x".into() }.needs_signature());
    }

    /// A cross-patient row is a line whose displayed dose may belong to the OTHER patient
    /// in the group (issue #334), so it is withheld from the gesture rather than signed.
    #[test]
    fn a_cross_patient_row_is_never_a_target() {
        let mut rows = vec![row(
            1,
            MedicationStatus::Active,
            vec![member(1, VouchState::Absent)],
        )];
        rows[0].cross_patient = true;
        assert!(sign_off_targets(&rows).is_empty());
    }

    /// The clean lines are still signable — withholding is per LINE, not per chart. A
    /// clinician blocked from signing eleven sound drugs because a twelfth is suspect
    /// would be slower than paper, which §1.2 forbids.
    #[test]
    fn a_cross_patient_row_does_not_block_the_rest_of_the_chart() {
        let mut rows = vec![
            row(
                1,
                MedicationStatus::Active,
                vec![member(1, VouchState::Absent)],
            ),
            row(
                2,
                MedicationStatus::Active,
                vec![member(2, VouchState::Absent)],
            ),
        ];
        rows[0].cross_patient = true;
        assert_eq!(sign_off_targets(&rows), vec![uid(2)]);
        assert_eq!(withheld_group_ids(&withheld_rows(&rows)), vec![uid(1)]);
    }

    /// Withholding is reported by GROUP id, because that is the line the clinician sees.
    /// Only lines that would OTHERWISE have been signed are reported: a cross-patient row
    /// everyone has already vouched is not an outstanding action, and reporting it would
    /// train the reader to ignore the warning.
    #[test]
    fn a_fully_vouched_cross_patient_row_is_not_reported_as_withheld() {
        let mut rows = vec![row(
            1,
            MedicationStatus::Active,
            vec![member(1, VouchState::Fresh { by: "dr_b".into() })],
        )];
        rows[0].cross_patient = true;
        assert!(withheld_rows(&rows).is_empty());
    }

    /// A ceased cross-patient row is not an outstanding action either — a struck line is
    /// never signed, so there is nothing being withheld from the clinician.
    #[test]
    fn a_ceased_cross_patient_row_is_not_reported_as_withheld() {
        let mut rows = vec![row(
            1,
            MedicationStatus::Ceased,
            vec![member(1, VouchState::Absent)],
        )];
        rows[0].cross_patient = true;
        assert!(withheld_rows(&rows).is_empty());
    }

    #[test]
    fn an_ordinary_chart_withholds_nothing() {
        let rows = vec![row(
            1,
            MedicationStatus::Active,
            vec![member(1, VouchState::Absent)],
        )];
        assert!(withheld_rows(&rows).is_empty());
    }

    /// #697 (b): a doubted-link reason alone withholds the line, and the report carries it.
    #[test]
    fn a_doubted_link_line_is_withheld_with_its_reason() {
        let mut rows = vec![row(
            1,
            MedicationStatus::Active,
            vec![member(1, VouchState::Absent)],
        )];
        rows[0].cross_patient = true;
        rows[0].wrong_chart.doubted_link = true;
        assert!(sign_off_targets(&rows).is_empty());
        assert_eq!(
            withheld_rows(&rows),
            vec![WithheldLine {
                group_id: uid(1),
                reasons: WrongChartReasons {
                    outside_set: false,
                    doubted_link: true
                },
            }]
        );
        assert_eq!(withheld_group_ids(&withheld_rows(&rows)), vec![uid(1)]);
    }

    /// A ceased line is not withheld, even under a doubted link: a CEASED line on the other
    /// member of a doubted set is shown, never signed, and never reported as withheld — it
    /// needs no signature, so a "will NOT be signed" report about it would be noise.
    #[test]
    fn a_ceased_doubted_link_line_is_not_reported_as_withheld() {
        let mut rows = vec![row(
            1,
            MedicationStatus::Ceased,
            vec![member(1, VouchState::Absent)],
        )];
        rows[0].cross_patient = true;
        rows[0].wrong_chart.doubted_link = true;
        assert!(withheld_rows(&rows).is_empty());
    }
}
