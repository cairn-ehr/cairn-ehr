//! The sentences a Withhold and a Release report (#832), as `write_failure::cease_outcome` words
//! a cease's. Pure, so every combination — done, NOT done, NOT known — is tested
//! (`outcome_tests.rs`); the webview only says what it is handed (ADR-0060 decision 2: partial
//! completion is reported, never implied).
use crate::funnel::view::Recorded;
use crate::write_failure::UnknownThread;

/// What one Withhold did. It writes ONE event, so there are only two outcomes worth a sentence:
/// it landed, or its commit got no ordinary answer (#814) and may or may not have landed. An
/// ordinary failure never reaches here — the command returns it as `node_failure`, whose text is
/// the node's own legible refusal (§9.6).
///
/// Returns the sentence AND the report's `behind_the_record` from one match, so the two cannot
/// disagree: `Unknown` exactly when the sentence says the W may be on the chart.
pub(crate) fn withhold_outcome(unknown: Option<&UnknownThread>) -> (String, Option<Recorded>) {
    match unknown {
        None => ("Withheld — the chart now shows the W.".to_string(), None),
        Some(u) => (
            format!(
                "Whether this drug was withheld is NOT known — no ordinary answer came back \
                 while saving thread {} (event {}: {}). Once the window can reach the node \
                 again, the chart shows whether it carries the W.",
                u.thread, u.event, u.cause
            ),
            Some(Recorded::Unknown),
        ),
    }
}

/// What one Release did: the holds released, the holds shown that had already ended (`gone`, left
/// as they are), those NOT released (`failed`: a thread whose write failed, or a hold deliberately
/// held back — each already a named sentence), and threads whose commit got no ordinary answer
/// (#814), which may or may not be released, so they are neither. "Released 0" is left out beside
/// an unknown: that thread's holds may be released, so the count would be false.
pub(crate) fn release_outcome(
    released: usize,
    gone: usize,
    failed: &[String],
    unknown: &[UnknownThread],
) -> String {
    let mut parts = Vec::new();
    if released > 0 || unknown.is_empty() {
        parts.push(format!("Released {released} hold(s) on this drug."));
    }
    if gone > 0 {
        parts.push(format!(
            "{gone} hold(s) shown had already ended and were left as they are."
        ));
    }
    if !failed.is_empty() {
        parts.push(format!(
            "{} were NOT released and are still withheld: {}",
            failed.len(),
            failed.join("; ")
        ));
    }
    if !unknown.is_empty() {
        let named: Vec<String> = unknown
            .iter()
            .map(|u| format!("thread {} (event {}: {})", u.thread, u.event, u.cause))
            .collect();
        parts.push(format!(
            "Whether the holds on {} thread(s) of this drug were released is NOT known — no \
             ordinary answer came back while saving {}. Once the window can reach the node \
             again, the chart shows whether each is still withheld.",
            unknown.len(),
            named.join("; ")
        ));
    }
    parts.join(" ")
}

#[cfg(test)]
#[path = "outcome_tests.rs"]
mod tests;
