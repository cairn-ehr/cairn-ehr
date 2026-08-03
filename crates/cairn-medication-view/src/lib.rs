//! Shared, pure medication read model + the sign-off targeting rule. See `row.rs` for why
//! this is its own crate rather than living in the node or the GUI.
pub mod chart;
pub mod fixtures;
pub mod row;
pub mod targeting;

pub use chart::{format_hazard_groups, PatientMedicationList, SEPARATION_INSTRUCTION};
pub use row::{MedicationRow, MedicationStatus, MemberVouch, VouchState};
pub use targeting::{sign_off_targets, withheld_rows};
