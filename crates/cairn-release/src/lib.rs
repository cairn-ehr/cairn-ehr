//! Signed releases (ADR-0055): document shapes + encoding. The verifier joins in Task 3.

mod docs;
mod refusal;

#[cfg(test)]
mod docs_tests;

pub use docs::*;
pub use refusal::{Refusal, Who};
