//! The reference shell's framework-agnostic state.
//!
//! The iced rendering layer that used to live here was retired on 2026-08-03: released
//! iced 0.14 ships no accessibility tree (spike 0004), so the reference UI moved to Tauri
//! 2. What survives is what was never framework-specific — the pane/tab workspace state
//! machine and the freshness rules — kept because the Tauri shell wires them in a later
//! slice. Tested code awaiting a consumer, not dead code.
//!
//! The `a11y_dump` module went with it. Its shell-chrome node was a hand-written *target*
//! tree for what iced's `pane_grid` was supposed to announce, and a target describing a
//! renderer nobody builds any more is worse than none — it would read as a claim about the
//! shipped window. Each tab's accessibility contract is declared by its own `Semantic` impl
//! and asserted in that tab's tests (see `cairn-gui-tab-medications`).
pub mod freshness;
pub mod workspace;
pub use freshness::{freshness, Freshness, Loaded};
pub use workspace::{Side, Workspace};
