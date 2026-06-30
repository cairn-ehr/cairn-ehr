//! Spike 0004 — iced reference-UI viability harness (the pure core).
//!
//! See [`docs/spikes/0004-iced-reference-ui-viability.md`] for the full pass/fail
//! definition. This crate is split deliberately by the §9.1 blast-radius rule:
//!
//! - **Pure core (this library, always built):** [`latency`] statistics (claim
//!   **L1**) and the multi-script [`corpus`] (claim **I** structure). Zero heavy
//!   dependencies, unit-tested in CI without a display, screen reader, or Pi.
//! - **`shaping` feature:** [`shaping`] runs the actual text shaper iced uses
//!   (cosmic-text → rustybuzz) over the corpus — claim **I1**. Needs a font; SKIPs
//!   loudly when none is installed, never a silent pass.
//! - **`gui` feature:** the iced binary (`src/bin/gui.rs`) — the dense clinical
//!   form for the operator-run a11y / IME / live-latency passes.
//!
//! Nothing here touches Cairn's wire core or signs anything: the UI is a pure L3
//! producer (ADR-0021 / §9.5). The harness has no path to author a real event.

pub mod corpus;
pub mod form;
pub mod latency;

#[cfg(feature = "shaping")]
pub mod shaping;
