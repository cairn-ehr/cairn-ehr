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
        // Said BEFORE the re-read, which may fail: claim only what the commit proved.
        None => (
            "Withheld — the W is recorded on this drug.".to_string(),
            None,
        ),
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

/// What one Release did, one sentence per kind of hold:
/// - `released`: hold ids released;
/// - `gone`: holds shown that are no longer on the line, left as they are;
/// - `unseen`: holds on the line it was NOT shown — never ended (ADR-0081 decision 4), but said:
///   information, not a failure;
/// - `failed`: items NOT released — a thread whose write failed (it may carry several holds) or
///   one hold deliberately held back. Each is already a named sentence, so the count is of
///   items, not holds;
/// - `unknown`: threads whose commit got no ordinary answer (#814), which may or may not be
///   released, so they are neither. "Released 0" is left out beside one: that thread's holds
///   may be released, so the count would be false.
pub(crate) fn release_outcome(
    released: usize,
    gone: usize,
    unseen: usize,
    failed: &[String],
    unknown: &[UnknownThread],
) -> String {
    let mut parts = Vec::new();
    if released > 0 || unknown.is_empty() {
        parts.push(format!("Released {released} hold(s) on this drug."));
    }
    if gone > 0 {
        parts.push(format!(
            "{gone} hold(s) shown are no longer on this line (ended or released meanwhile) \
             and were left as they are."
        ));
    }
    parts.extend(unseen_sentence(unseen));
    if !failed.is_empty() {
        parts.push(format!(
            "{} item(s) were NOT released and are still on this line: {}",
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

/// The refusal for a Release whose every shown hold has already gone: nothing was done. It
/// still names holds the clinician was NOT shown (#839): Release is when they decide to give
/// the drug, and if the re-read after this refusal fails, this sentence is all they see.
pub(crate) fn all_shown_gone(unseen: usize) -> String {
    let refusal = "the holds shown are no longer on this line — nothing was done";
    match unseen_sentence(unseen) {
        Some(unseen) => format!("{refusal}. {unseen}"),
        None => refusal.to_string(),
    }
}

/// Holds on the line a Release was not shown, never ended (ADR-0081 decision 4) but said.
/// "Still on this line", never "in force": the count includes pending holds, which have not
/// started (#839).
fn unseen_sentence(unseen: usize) -> Option<String> {
    (unseen > 0).then(|| {
        format!(
            "{unseen} hold(s) added since this line was drawn are still on this line (they \
             were not shown, so were not released)."
        )
    })
}

#[cfg(test)]
#[path = "outcome_tests.rs"]
mod tests;
