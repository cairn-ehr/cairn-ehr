//! §5.3/§5.8 patient registration and the search that precedes it.
//!
//! `search` maps this node's projections to the shared candidate model — the CLI reads through
//! it, and the future picker window and native API (ADR-0023) are expected to wrap this same
//! function rather than re-derive the joins. `candidate_read` is its display half, shared with
//! the possible-duplicate worklist (`candidates_by_id`), so a chart renders the same in both.
//! `register` is the
//! act `search` feeds: a chart is never registered without first offering the clerk the
//! candidates already on file (that ordering is why `search` landed first) — see
//! `register::register_patient` for the STANDARD create act, and `crate::john_doe` for the
//! search-AFTER-create §5.4 path this module does not cover. `person` (ADR-0076) answers a
//! different question — not who a chart might be, but the FULL set of charts that stand
//! linked together as one person, the queried chart included — and is the one place every
//! combined read (medication list now, allergies and the duplicate banner later) goes to
//! agree on that set.
pub mod candidate_read;
pub mod candidate_text;
pub mod compare;
pub mod edges;
pub mod person;
pub mod register;
pub mod search;
mod search_person;
mod search_rank;
