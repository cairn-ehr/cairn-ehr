//! §3.15/§3.16 medication recording — the clinical-content builders.
//!
//! Pure: no clock, no randomness, no I/O. The cairn-node edge mints the ids,
//! stamps the HLC, and signs; these functions only shape the `payload` JSON that
//! becomes `EventBody.payload`. Optional fields are inserted only when present —
//! never serialized as null — so an added-later field never changes an existing
//! event's content address (principle 11, the demographics idiom).
//!
//! Verbs over an immortal `medication_id` thread: an *assertion* (`assert`) mints
//! the thread; a *cessation* (`cessation`) ends it; a *dose change / correction*
//! (`dose`, slice 2) overlays the dose over time.
pub mod assert;
pub mod cessation;

pub use assert::{medication_assertion_body, render_medication_twin, MedicationAssertion};
pub use cessation::{
    medication_cessation_body, render_medication_cessation_twin, MedicationCessation,
};
