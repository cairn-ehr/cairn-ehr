//! Shared, pure medication read model + the sign-off targeting rule. See `row.rs` for why
//! this is its own crate rather than living in the node or the GUI.
//!
//! # The `fixtures` feature
//!
//! `fixtures` carries a realistic sample chart for the `--mock` window and the GUI tests.
//! It is OFF by default, so `cairn-node` and `cairn-sync` — which depend on this crate for
//! the read model alone — do not link demo clinical content into a production node binary.
//! The GUI crates that need it turn it on. `--mock` is still a shipped mode, just not one
//! the node has to carry.
pub mod chart;
pub mod display;
#[cfg(feature = "fixtures")]
pub mod fixtures;
pub mod row;
pub mod targeting;

pub use chart::{format_hazard_groups, PatientMedicationList, SEPARATION_INSTRUCTION};
pub use display::{short_kid, DISPLAYED_KID_CHARS};
pub use row::{MedicationRow, MedicationStatus, MemberVouch, VouchState};
pub use targeting::{sign_off_targets, withheld_rows};
