//! The §5.3/§5.8 search-before-create funnel's rules, as pure functions.
//!
//! # Why this crate exists
//!
//! The funnel is the reference UI's front door: a clerk browses for an existing chart, and
//! only if nothing fits do they register a new one — an act that permanently records the
//! search which preceded it ([ADR-0061](../../../docs/spec/decisions/0061-registration-is-an-act-that-carries-its-search.md)).
//! Every *decision* in that workflow is a clinical decision, not a layout detail: how many
//! existing charts a clerk is shown before being allowed to create another, when the machine
//! searches unasked, and what a registration is permitted to swear it displayed.
//!
//! Those rules live here, in a crate with no window and no database, so that all of them are
//! testable with no fixture beyond a struct literal, and none of them can be re-derived —
//! differently — by whichever surface happens to need one. (The `cairn-gui` workspace is
//! `exclude`d from the root one, so its gate runs as
//! `cargo test --manifest-path cairn-gui/Cargo.toml`, not as a plain root `cargo test`.) That is the same discipline `cairn-patient-search`
//! states for the candidate model it owns, one layer down: *the surface that displays
//! candidates and the act that attests to them must not be able to disagree.*
//!
//! # What is deliberately NOT here
//!
//! Anything that cannot be stated without Tauri. The commands, the window's shell state, the
//! frontend and the database-backed implementations of the ports are slice 2b's; a rule that
//! needs a window to express is a rule no test can pin cheaply, which is the whole reason for
//! the split.
//!
//! # The four rules
//!
//! - [`trigger`] — when the machine runs the registration search *unasked*. **Advisory, never
//!   a gate.**
//! - [`prompt`] — how many candidates that search may show, and how it admits to the ones it
//!   did not.
//! - [`token`] — the pairing of a query with the list it produced, which is the only thing a
//!   registration may attest to.
//! - [`session`] — one window's form: the raw typed name kept beside its token, and searches
//!   for a form that has since been edited dropped rather than recorded.

pub mod prompt;
pub mod session;
pub mod token;
pub mod trigger;

pub use prompt::{bound_for_prompt, PromptList, PROMPT_CAP};
pub use session::{FormSnapshot, FunnelSession, Recorded};
pub use token::{AttestedSearch, Restored, SearchToken, TokenError, TokenStore};
pub use trigger::{trigger_state, MissingPart, TriggerState, MIN_NAME_TOKENS};
