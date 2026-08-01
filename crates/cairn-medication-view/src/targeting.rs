//! The one definition of what a single sign-off gesture attests (#288).
use crate::row::{MedicationRow, MedicationStatus};
use uuid::Uuid;

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
        // A struck line is not re-signed. Ceased rows stay visible for parity with a
        // paper chart, but they are never targets.
        .filter(|row| row.status == MedicationStatus::Active)
        .flat_map(|row| row.members.iter())
        .filter(|member| member.vouch.needs_signature())
        .map(|member| member.medication_id)
        .collect();
    targets.sort();
    targets.dedup();
    targets
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::row::{MedicationRow, MedicationStatus, MemberVouch, VouchState};

    /// Deterministic uuids without a random source: `Uuid::from_u128` makes each id
    /// readable in a failure message and stable across runs.
    fn uid(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn row(group: u128, status: MedicationStatus, members: Vec<MemberVouch>) -> MedicationRow {
        MedicationRow {
            group_id: uid(group),
            patient_id: uid(999),
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
        }
    }

    fn member(id: u128, vouch: VouchState) -> MemberVouch {
        MemberVouch {
            medication_id: uid(id),
            vouch,
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
}
