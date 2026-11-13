//! §3.3 medication recording — the clinical-content builders.
//!
//! Pure: no clock, no randomness, no I/O. The cairn-node edge mints the ids,
//! stamps the HLC, and signs; these functions only shape the `payload` JSON that
//! becomes `EventBody.payload`. Optional fields are inserted only when present —
//! never serialized as null — so an added-later field never changes an existing
//! event's content address (principle 11, the demographics idiom).
//!
//! Verbs over an immortal `medication_id` thread: an *assertion* (`assert`) mints
//! the thread; a *cessation* (`cessation`) ends it; a *dose change / correction*
//! (`dose`, slice 2) overlays the dose over time. A *reconciliation / separation*
//! (`reconciliation`, slice 3) is the one verb about TWO threads — it links (or
//! splits) two threads declared to be the same real drug (never-merge-always-link). A *withhold* (`withhold`) is a time-bounded hold on a
//! thread (ADR-0081) and its *release* ends named holds early; neither ends the thread.
pub mod assert;
pub mod attestation;
pub mod cessation;
pub mod charting;
pub mod coding;
pub mod dose;
pub mod reconciliation;
pub mod set_aside;
pub mod source_note;
pub mod withhold;

pub use assert::{
    drug_line_twin, medication_assertion_body, render_medication_twin, MedicationAssertion,
    SubstanceCoding,
};
pub use attestation::{
    medication_attestation_body, render_medication_attestation_twin, MedicationAttestation,
};
pub use cessation::{
    medication_cessation_body, render_medication_cessation_twin, MedicationCessation,
};
pub use charting::{
    charting_twin_parts, insert_charting, ChartedAs, Charting, Component, OverrideRef, TwinParts,
    WarningOverride, ALLERGY_SOURCE,
};
pub use coding::{
    medication_coding_body, medication_coding_correction_body,
    render_medication_coding_correction_twin, render_medication_coding_twin, CodingClaim,
    MedicationCoding, MedicationCodingCorrection,
};
pub use dose::{
    dose_change_body, dose_correction_body, render_dose_change_twin, render_dose_correction_twin,
    DoseChange, DoseCorrection,
};
pub use reconciliation::{
    reconciliation_body, render_reconciliation_twin, render_separation_twin, separation_body,
    ReconciliationAssertion,
};
pub use set_aside::{
    render_set_aside_twin, set_aside_body, MedicationSetAside, SET_ASIDE_EVENT_TYPE,
    SET_ASIDE_SCHEMA,
};
pub use source_note::{insert_source_note, source_note_twin, SourceNote};
pub use withhold::{
    medication_withhold_body, medication_withhold_release_body,
    render_medication_withhold_release_twin, render_medication_withhold_twin, HoldUntil,
    MedicationWithhold, MedicationWithholdRelease,
};
