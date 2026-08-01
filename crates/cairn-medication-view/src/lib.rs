//! Shared, pure medication read model + the sign-off targeting rule. See `row.rs` for why
//! this is its own crate rather than living in the node or the GUI.
pub mod row;
pub mod targeting;

pub use row::{MedicationRow, MedicationStatus, MemberVouch, VouchState};
pub use targeting::sign_off_targets;
