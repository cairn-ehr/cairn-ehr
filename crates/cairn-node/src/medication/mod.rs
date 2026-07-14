//! §3.15/§3.16 medication recording — the node authoring surface. Device-additive
//! by default (signed by the node key, a `recorded` contributor, no attestation);
//! the slice-4 attestation path (`attestation.rs`) layers human responsibility as a
//! separable overlay. Orchestrators over an immortal `medication_id` thread:
//! assert / cease / change-dose / correct-dose, plus reconcile / separate over a
//! thread PAIR. Offline-first throughout (no event requires its target thread to be
//! present locally). Split by verb to keep each file focused (house rule 4).
mod assert;
mod cessation;
mod dose;
mod reconciliation;

pub use assert::{assert_medication, build_assert_body, validate_term, AssertMedicationInput};
pub use cessation::{build_cease_body, cease_medication, CeaseMedicationInput};
pub use dose::{
    build_dose_change_body, build_dose_correction_body, change_dose, correct_dose,
    resolve_correction_target, ChangeDoseInput, CorrectDoseInput,
};
pub use reconciliation::{
    build_reconcile_body, build_separate_body, reconcile_medications, separate_medications,
    validate_distinct_subjects, ReconcileInput,
};
