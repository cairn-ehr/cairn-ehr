//! How a failed NODE write is handed to the webview as an `ErrorView` (sign-off, cease).
//!
//! Sign-off and cease used to return a bare `String`, so the webview could not tell "the record
//! moved under you" from "the node refused" from "the node is unreachable" and treated all of
//! them as a dead end. The node's failure already carries that distinction (the funnel's
//! `data_error_from` classification), so this module only maps it onto the funnel's `Retry`
//! vocabulary. The TEXT is untouched: the node's own legible sentence (§9.6) is the one thing
//! the clinician needs, so it is passed through exactly as the `String` version did.
use crate::funnel::view::{ErrorView, Retry};
use crate::link::judgement_view::refused;
use cairn_gui_data::port::DataError;

/// The retry a classified failure earns. Pure, so every arm is tested without a database
/// (the node-side verdict types are `pub(crate)` to `cairn-node`, so this crate cannot build
/// the real errors).
pub(crate) fn retry_for(error: &DataError) -> Retry {
    match error {
        // The screen is behind the record: re-read the whole chart, do not offer the button.
        DataError::Stale(_) => Retry::AfterReread,
        DataError::Refused(_) | DataError::NotFound => Retry::Never,
        DataError::NotProvisioned(_) => Retry::AfterOperator,
        // Nothing was confirmed either way, and the act can simply be pressed again.
        DataError::Unavailable(_) => Retry::Now,
    }
}

/// A failed node write as the view the webview reads: the node's own text, the retry its
/// classification earns. A failure that is not a classified node verdict is an outage
/// (`Retry::Now`), which is what `data_error_from` makes of it.
pub(crate) fn node_failure(e: &anyhow::Error) -> ErrorView {
    let class = cairn_gui_live::error::data_error_from(e);
    match retry_for(&class) {
        Retry::Never => refused(format!("{e:#}")),
        retry => ErrorView {
            text: format!("{e:#}"),
            retry,
        },
    }
}

#[cfg(test)]
#[path = "write_failure_tests.rs"]
mod tests;
