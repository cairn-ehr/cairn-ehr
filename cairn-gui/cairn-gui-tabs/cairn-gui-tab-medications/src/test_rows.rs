//! Test-only builders shared by the view-model tests in `view.rs` and `row_view.rs`.
//!
//! One copy, because the two test modules describe the SAME chart shapes: a second copy of
//! `row()` would drift the moment one of them grew a field (as `source_charts` did in R1),
//! and a fixture that differs between the two files makes their assertions incomparable.
use cairn_medication_view::{
    ChartSet, MedicationRow, MedicationStatus, MemberVouch, PatientMedicationList, VouchState,
    WrongChartReasons,
};
use std::collections::BTreeMap;
use uuid::Uuid;

pub(crate) fn uid(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

/// One drug line on chart 999, dose 500 mg, with the given members.
pub(crate) fn row(
    group: u128,
    status: MedicationStatus,
    members: Vec<MemberVouch>,
) -> MedicationRow {
    MedicationRow {
        group_id: uid(group),
        display_chart: uid(999),
        term: "metformin".into(),
        coding_display: None,
        formulation: None,
        dose_amount: Some("500".into()),
        dose_unit: Some("mg".into()),
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

/// One member thread on chart 999.
pub(crate) fn member(id: u128, vouch: VouchState) -> MemberVouch {
    MemberVouch {
        medication_id: uid(id),
        vouch,
        patient_id: uid(999),
    }
}

/// A chart of exactly these rows, with nothing hidden and nothing to repair — the
/// normal case, which is what most of these tests are about.
pub(crate) fn chart(rows: Vec<MedicationRow>) -> PatientMedicationList {
    PatientMedicationList {
        rows,
        groups_missing_from_chart: vec![],
        separation_targets: BTreeMap::new(),
        charts: ChartSet::single(uid(999)),
    }
}
