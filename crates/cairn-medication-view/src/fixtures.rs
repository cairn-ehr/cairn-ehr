//! A realistic sample chart, shared by the GUI's mock data port and by the view-model
//! tests. One definition, because two copies of "the interesting shapes" drift and the
//! tests then stop covering what the demo actually shows.
//!
//! Not test-only: the `--mock` window is a real, shipped mode — it is what the operator
//! accessibility pass and the timing runbook use on a machine with no database.
//!
//! WHY THE FIXTURE CHART IS NOT A HEALTHY ONE. It deliberately carries a cross-patient
//! group and an invisible group (issue #334), which in real operation are rare. The
//! surfaces that report them are the ones most likely to be wrong and least likely to be
//! exercised: a screen-reader pass over a chart with nothing to warn about proves nothing
//! about whether the warning is announced at all. ADR-0060 decision 2 — partial completion
//! must be *reported*, never implied — is only checkable against a chart that has
//! something to report.
use crate::chart::PatientMedicationList;
use crate::row::{MedicationRow, MedicationStatus, MemberVouch, VouchState};
use std::collections::BTreeMap;
use uuid::Uuid;

/// The patient id the mock chart belongs to.
pub const FIXTURE_PATIENT: &str = "00000000-0000-0000-0000-000000000001";

fn uid(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

fn base(group: u128, term: &str, amount: &str, unit: &str) -> MedicationRow {
    MedicationRow {
        group_id: uid(group),
        patient_id: uid(1),
        term: term.to_string(),
        coding_display: None,
        formulation: Some("tablet".into()),
        dose_amount: Some(amount.into()),
        dose_unit: Some(unit.into()),
        sig: None,
        started_value: None,
        started_precision: None,
        status: MedicationStatus::Active,
        members: vec![],
        reconciliation_flagged: false,
        coding_conflict: false,
        cross_patient: false,
    }
}

fn member(id: u128, vouch: VouchState) -> MemberVouch {
    MemberVouch {
        medication_id: uid(id),
        vouch,
    }
}

/// A chart covering every shape the view model has to get right: unsigned, signed by
/// SOMEONE ELSE, stale, ceased, a reconciled group whose two members disagree, and a
/// cross-patient line that must be shown but never signed.
pub fn sample_rows() -> Vec<MedicationRow> {
    // A deliberately NON-hex label. A fixture attester id never needs to look like a real
    // key id, and a hex-shaped literal in a key-id field trips CodeQL's
    // `rust/hard-coded-cryptographic-value` as a recurring false positive that blocks the
    // scan until a human dismisses it (house rule 6, issue #146) — and it would do so from
    // a NON-test file, where the query is a real defense we want to keep live.
    let other = "fixture-clinician-b".to_string();

    let mut unsigned = base(10, "atorvastatin", "40", "mg");
    unsigned.members = vec![member(10, VouchState::Absent)];

    let mut signed_by_other = base(20, "amlodipine", "5", "mg");
    signed_by_other.members = vec![member(20, VouchState::Fresh { by: other.clone() })];

    let mut stale = base(30, "sertraline", "50", "mg");
    stale.members = vec![member(30, VouchState::Stale { by: other.clone() })];

    let mut ceased = base(40, "ibuprofen", "400", "mg");
    ceased.status = MedicationStatus::Ceased;
    ceased.members = vec![member(40, VouchState::Absent)];

    // A reconciled pair: ONE row, TWO threads, differing freshness. The group/thread
    // asymmetry that the badge and the sign-off count both have to handle.
    let mut reconciled = base(50, "metformin", "1", "g");
    reconciled.coding_display = Some("metformin hydrochloride".into());
    reconciled.members = vec![
        member(50, VouchState::Fresh { by: other }),
        member(51, VouchState::Absent),
    ];
    reconciled.reconciliation_flagged = true;

    // The wrong-chart hazard (#334): this line's group holds a thread belonging to ANOTHER
    // patient, so the dose shown may be that patient's. It is displayed — hiding a drug is
    // worse — but it is never a sign-off target, and the window has to say why.
    let mut cross_patient = base(60, "warfarin", "5", "mg");
    cross_patient.members = vec![member(60, VouchState::Absent)];
    cross_patient.cross_patient = true;

    vec![
        unsigned,
        signed_by_other,
        stale,
        ceased,
        reconciled,
        cross_patient,
    ]
}

/// The whole fixture chart, including what it cannot show.
///
/// `groups_missing_from_chart` holds a group this patient has a thread in that displays on
/// a DIFFERENT patient's chart — the other half of the same #334 defect. There is no row
/// for it by construction; the only honest surface is the report, which is exactly the
/// thing a renderer is most likely to drop.
pub fn sample_chart() -> PatientMedicationList {
    let invisible_group = uid(70);
    let cross_patient_group = uid(60);
    PatientMedicationList {
        rows: sample_rows(),
        groups_missing_from_chart: vec![invisible_group],
        // Both hazardous groups carry their FULL membership, including the other patient's
        // thread — the arguments `medication-separate` takes. A warning that names a remedy
        // without its arguments is the defect #338 review finding 1 was about.
        separation_targets: BTreeMap::from([
            (cross_patient_group, vec![uid(60), uid(61)]),
            (invisible_group, vec![uid(70), uid(71)]),
        ]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fixture's whole job is to carry the shapes the renderer must get right. If a
    /// shape drops out of it, the demo silently stops exercising that path — and the
    /// operator's screen-reader pass stops covering it too.
    #[test]
    fn the_fixture_carries_every_shape_the_renderer_must_handle() {
        let chart = sample_chart();
        let vouches: Vec<&VouchState> = chart
            .rows
            .iter()
            .flat_map(|r| r.members.iter().map(|m| &m.vouch))
            .collect();

        assert!(vouches.iter().any(|v| matches!(v, VouchState::Absent)));
        assert!(vouches
            .iter()
            .any(|v| matches!(v, VouchState::Fresh { .. })));
        assert!(vouches
            .iter()
            .any(|v| matches!(v, VouchState::Stale { .. })));
        assert!(chart
            .rows
            .iter()
            .any(|r| r.status == MedicationStatus::Ceased));
        assert!(chart.rows.iter().any(|r| r.members.len() > 1));
        assert!(chart.rows.iter().any(|r| r.cross_patient));
        assert!(!chart.groups_missing_from_chart.is_empty());
    }

    /// Every hazardous group must be able to name its repair arguments — the displayed one
    /// and the invisible one alike. The invisible group is the harder case: it has no row
    /// at all, so `separation_targets` is the ONLY place its threads can come from.
    #[test]
    fn every_hazardous_group_carries_its_separation_targets() {
        let chart = sample_chart();
        for group in chart
            .rows
            .iter()
            .filter(|r| r.cross_patient)
            .map(|r| r.group_id)
            .chain(chart.groups_missing_from_chart.iter().copied())
        {
            let members = chart
                .separation_targets
                .get(&group)
                .unwrap_or_else(|| panic!("hazardous group {group} has no separation targets"));
            assert!(
                members.len() >= 2,
                "a cross-patient group needs both threads to be repairable: {members:?}"
            );
        }
    }
}
