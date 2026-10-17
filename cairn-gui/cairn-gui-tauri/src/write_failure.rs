//! How a failed NODE write is handed to the webview as an `ErrorView` (sign-off, cease).
//!
//! Sign-off and cease used to return a bare `String`, so the webview could not tell "the record
//! moved under you" from "the node refused" from "the node is unreachable" and treated all of
//! them as a dead end. The node's failure already carries that distinction (the funnel's
//! `data_error_from` classification), so this module only maps it onto the funnel's `Retry`
//! vocabulary. The TEXT is untouched: the node's own legible sentence (§9.6) is the one thing
//! the clinician needs, so it is passed through exactly as the `String` version did.
use crate::funnel::view::{ErrorView, Retry};
use cairn_gui_data::port::DataError;

/// The retry a classified failure earns. Pure, so every arm is tested on a `DataError` directly;
/// that real node errors classify into those arms is pinned separately, through `node_failure`.
pub(crate) fn retry_for(error: &DataError) -> Retry {
    match error {
        // The screen is behind the record: re-read the whole chart, do not offer the button.
        DataError::Stale(_) => Retry::AfterReread,
        DataError::Refused(_) | DataError::NotFound => Retry::Never,
        DataError::NotProvisioned(_) => Retry::AfterOperator,
        // The outcome is not confirmed (only a judgement's commit is marked as unknown; a
        // sign-off or cease is not). The window re-reads the chart right after the failure,
        // which shows whether the write landed.
        DataError::Unavailable(_) => Retry::Now,
    }
}

/// A failed node write as the view the webview reads: the node's own text, the retry its
/// classification earns. A failure that is not a classified node verdict is an outage
/// (`Retry::Now`), which is what `data_error_from` makes of it.
pub(crate) fn node_failure(e: &anyhow::Error) -> ErrorView {
    ErrorView {
        text: format!("{e:#}"),
        retry: retry_for(&cairn_gui_live::error::data_error_from(e)),
    }
}

/// The chart read a write starts with failed (sign-off's set check, cease's line lookup): an
/// OUTAGE — nothing was decided, let alone written — so the act may be retried (`Retry::Now`),
/// as `link::chart_set_of` words the same fault. Never `refused`, whose `Never` would claim a
/// verdict that does not exist (#797 review). The read's own text passes through.
pub(crate) fn chart_read_failed(text: String) -> ErrorView {
    ErrorView {
        text,
        retry: Retry::Now,
    }
}

/// The signing key is locked: not a verdict. The identical act succeeds once the clinician
/// unlocks it, so it keeps the button (`Retry::Now`) - the vocabulary of
/// `link::judgement_view::key_locked`.
pub(crate) fn key_locked(text: &str) -> ErrorView {
    ErrorView {
        text: text.into(),
        retry: Retry::Now,
    }
}

/// This node cannot write at all (no node key, no database connection): only the operator can
/// mend that, so `Retry::AfterOperator`.
pub(crate) fn node_cannot_write(text: &str) -> ErrorView {
    ErrorView {
        text: text.into(),
        retry: Retry::AfterOperator,
    }
}

#[cfg(test)]
#[path = "write_failure_tests.rs"]
mod tests;
