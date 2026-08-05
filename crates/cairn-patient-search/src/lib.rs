//! The shared, pure patient-search read model: what a candidate IS, and the one definition
//! of what a registration attests to.
//!
//! # Why this is its own crate
//!
//! Same reason as `cairn-medication-view`: a future picker window cannot depend on a crate
//! carrying a Postgres driver, and the node and the window must not be able to answer
//! *"what was displayed?"* differently. A divergence there means a registration swearing to
//! candidates the clerk never saw — the exact forensic record the funnel exists to produce.
//!
//! Deliberately dependency-light (uuid + serde). No database, no clock: `age_years` takes
//! `today` as an argument so the whole crate is unit-testable and the edge owns the clock.
pub mod attestation;
pub mod candidate;
pub mod query;

pub use attestation::SearchAttestation;
pub use candidate::{age_years, Age, Candidate, CandidateList, TrustState};
pub use query::SearchQuery;
