//! Signed releases (ADR-0055): document shapes + encoding, and the pure verifier that
//! both the node's intake and the terminal's installer call.

mod docs;
mod refusal;
mod verify;

#[cfg(test)]
mod docs_tests;
#[cfg(test)]
mod refusal_tests;
#[cfg(test)]
mod verify_release_tests;
#[cfg(test)]
mod verify_tests;

pub use docs::*;
pub use refusal::{Refusal, Who};
pub use verify::*;
