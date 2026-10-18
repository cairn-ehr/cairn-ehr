//! How a failed NODE write is handed to the webview as an `ErrorView` (sign-off, cease).
//!
//! Sign-off and cease used to return a bare `String`, so the webview could not tell "the record
//! moved under you" from "the node refused" from "the node is unreachable" and treated all of
//! them as a dead end. The node's failure already carries that distinction (the funnel's
//! `data_error_from` classification), so this module only maps it onto the funnel's `Retry`
//! vocabulary, passing the node's own legible sentence (§9.6) through untouched. One case is
//! worded here instead: a commit lost mid-flight, which may have written, so it is neither
//! "failed" nor done (#814: `sign_off_failure`, and a cease's `cease_outcome`).
use crate::funnel::view::{ErrorView, Recorded, Retry};
use cairn_gui_data::port::DataError;

/// The retry a classified failure earns. Pure, so every arm is tested on a `DataError` directly;
/// that real node errors classify into those arms is pinned separately, through `node_failure`.
pub(crate) fn retry_for(error: &DataError) -> Retry {
    match error {
        // The screen is behind the record: re-read the whole chart, do not offer the button.
        DataError::Stale(_) => Retry::AfterReread(Recorded::No),
        DataError::Refused(_) | DataError::NotFound => Retry::Never,
        DataError::NotProvisioned(_) => Retry::AfterOperator,
        // The failing step wrote nothing: an outage. (Earlier lines of a sign-off may be signed,
        // #818; a commit that MAY have written never reaches here: `node_failure` checks first.)
        DataError::Unavailable(_) => Retry::Now,
    }
}

/// A failed node write as the view the webview reads: the node's own text, the retry its
/// classification earns. A failure that is not a classified node verdict is an outage
/// (`Retry::Now`), which is what `data_error_from` makes of it — except a commit the node marked
/// outcome-unknown (#713): it may have landed, so it re-reads (`Unknown`), never `Now`, whether or
/// not the caller words it first. Fixed here, once, rather than in every surface.
pub(crate) fn node_failure(e: &anyhow::Error) -> ErrorView {
    let retry = match cairn_node::db_diagnosis::commit_outcome_unknown_event(e) {
        Some(_) => Retry::AfterReread(Recorded::Unknown),
        None => retry_for(&cairn_gui_live::error::data_error_from(e)),
    };
    ErrorView {
        text: format!("{e:#}"),
        retry,
    }
}

/// A failed sign-off as the view the webview reads (#814). One case comes first: a line's COMMIT
/// that got no ordinary answer (the node's `commit_outcome_unknown` marker). That line may be
/// signed, and the lines committed before it are, so "failed" would be false about the
/// clinician's own act. It is worded as not known and re-reads (`Recorded::Unknown`). A second
/// press cannot sign a line twice: the orchestrator targets only lines that still need a signature
/// (`sign_off_targets`), so a line whose commit did land is no longer a target. The gesture ends at
/// that line, so the lines after it were never tried (#818 tracks reporting the rest).
/// Every other failure is [`node_failure`]'s.
pub(crate) fn sign_off_failure(e: &anyhow::Error) -> ErrorView {
    match cairn_node::db_diagnosis::commit_outcome_unknown_event(e) {
        Some(event) => ErrorView {
            text: format!(
                "Whether the sign-off was recorded is NOT known — no answer came back while event \
                 {event} was being saved ({}). Lines signed before it in this sign-off are \
                 recorded; lines after it were not signed. Once the window can reach the node \
                 again, the chart shows which lines are signed.",
                e.root_cause()
            ),
            retry: Retry::AfterReread(Recorded::Unknown),
        },
        None => node_failure(e),
    }
}

/// What one cease did, as the outcome line says it (ADR-0060: partial completion is reported,
/// never implied): the threads stopped, those NOT stopped (`failed`, each already named), and
/// those whose commit got no ordinary answer (#814) — `(thread, event, cause)`, which may or may
/// not be stopped, so they are neither. "Stopped 0" is left out when one is unknown: that thread
/// may be stopped, so the count would be false. Rust, not the webview, so every combination is
/// tested.
pub(crate) fn cease_outcome(
    ceased: usize,
    failed: &[String],
    unknown: &[(uuid::Uuid, uuid::Uuid, String)],
) -> String {
    let mut parts = Vec::new();
    if ceased > 0 || unknown.is_empty() {
        parts.push(format!("Stopped {ceased} thread(s) of this drug."));
    }
    if !failed.is_empty() {
        parts.push(format!(
            "{} thread(s) were NOT stopped and are still active: {}",
            failed.len(),
            failed.join("; ")
        ));
    }
    if !unknown.is_empty() {
        let named: Vec<String> = unknown
            .iter()
            .map(|(thread, event, cause)| format!("thread {thread} (event {event}: {cause})"))
            .collect();
        parts.push(format!(
            "Whether {} thread(s) of this drug were stopped is NOT known — no answer came back \
             while saving {}. Once the window can reach the node again, the chart shows whether \
             each is still active.",
            unknown.len(),
            named.join("; ")
        ));
    }
    parts.join(" ")
}

/// The chart read a write starts with failed (sign-off's set check, cease's line lookup): an
/// OUTAGE — nothing was decided, let alone written — so the act may be retried (`Retry::Now`), the
/// retry `link::chart_set_of` gives the same fault. Never `refused`, whose `Never` would claim a
/// verdict that does not exist (#797 review). The read's own text passes through unprefixed.
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
