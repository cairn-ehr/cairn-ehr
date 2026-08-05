//! §5.3/§5.8 patient registration and the search that precedes it.
//!
//! `search` is the ONE mapping from this node's projections to the shared candidate model —
//! the CLI reads through it, and the future picker window and native API (ADR-0023) are
//! expected to wrap this same function rather than re-derive the joins. A later task adds
//! `register`, the act `search` feeds (a chart is never registered without first offering
//! the clerk the candidates already on file — that ordering is why `search` lands first).
pub mod search;
