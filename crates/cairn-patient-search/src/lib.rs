//! The shared, pure patient-search read model: what a candidate IS, the order candidates are
//! shown in (`rank`), how many of them a prompt shows (`prompt_bound`), and the one definition
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
pub mod list_wire;
pub mod person;
pub mod prompt_bound;
pub mod query;
pub mod rank;

pub use attestation::SearchAttestation;
pub use candidate::{age_years, Age, Candidate, CandidateList, PatientKind, TrustState};
pub use list_wire::DoubtedRowOutOfRange;
pub use person::{group_by_person, MissingComponent, PersonRow};
pub use prompt_bound::{bound_for_prompt, node_reason, PromptCounts, PromptList, PROMPT_CAP};
pub use query::{name_tokens, SearchQuery};
pub use rank::{
    callsign_typed_whole, is_dob_near_miss, rank_candidates, tokens_exactly_matched,
    tokens_matched, RankKey,
};
