//! The §5.3/§5.8 search-before-create funnel, as the window's front door (slice 2c).
//!
//! # A shell state, not a tab
//!
//! A tab presupposes a patient, and the funnel is how a patient is arrived at. So when no chart
//! is open the window shows the front door — browse, and register only if nothing fits — and
//! opening a chart (by picking, by recognising one in the step-3 prompt, or by registering)
//! lands on the chart surface under a persistent identity header. That header is the
//! wrong-chart affordance: possession, visible at every later step, not a confirmation dialog
//! (§1.2 rejects those).
//!
//! # Where each piece lives
//!
//! - [`window`] — the window's state for the funnel: which chart is open, the mock
//!   constructor, and the one accessor every chart command asks.
//! - [`backend`] — mock or live, dispatched on the mode the window launched in.
//! - [`commands`] — the Tauri commands: thin forwarders onto plain `*_impl` functions, so the
//!   whole front-door walk is testable against `--mock` with no Tauri runtime.
//! - [`view`] — every sentence the clerk reads and every payload the webview renders, as pure
//!   functions. No Tauri, no database: this is where the rules about WORDING are tested.
//!
//! The rules about the funnel itself (the trigger, the bounded prompt, token custody, the
//! session) are one layer down in `cairn-gui-funnel`; nothing here re-derives them.
pub mod backend;
pub mod commands;
pub mod view;
pub mod window;
