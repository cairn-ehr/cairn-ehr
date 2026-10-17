//! The outcome and failure sentences of "Different people — not the same person" (R5a, #680),
//! as pure functions: one sentence per pair judged, a failed pair named and never hidden behind
//! another's verdict, and the retry advice several failures share. Moved out of `view.rs` (the
//! banner's own sentences) to keep both files under 500 lines; the command that calls these is
//! `duplicates::different_people_impl`.
use super::view::NOTHING_OPEN;
use crate::funnel::view::{ErrorView, Recorded, Retry};
use crate::link::judgement_view::{judgement_error_view, refused, Charts, LinkReportView};
use cairn_node::chart_link::LinkEffect;
use uuid::Uuid;

/// One pair of a "Different people" judgement, as the window saw its outcome.
pub struct PairResult {
    pub low: Uuid,
    pub high: Uuid,
    pub outcome: Result<LinkEffect, ErrorView>,
}

/// A failed "Different people" pair, classified exactly as a failed link or unlink is — but
/// named for what was pressed. Never "The unlink was …": the two charts were never linked, and
/// that wording would send the clinician looking for a link to undo.
pub fn different_people_error_view(e: &anyhow::Error) -> ErrorView {
    judgement_error_view("\"different people\" judgement", Charts::Kept, e)
}

/// The outcome line for "Different people".
///
/// - Nothing judged → [`NOTHING_OPEN`].
/// - ONE pair, and it failed → that pair's own error, unchanged (so the webview applies its
///   retry advice: an outage — "not confirmed" — keeps the button, a verdict takes it).
/// - SEVERAL pairs, all failed → ONE error naming every pair's failure — never just the first,
///   which could hide a later pair's "commit outcome unknown" behind a refusal (the #713
///   hazard) — carrying the most retryable class among them (`combined_retry`).
/// - Anything recorded → one sentence per pair, and `reload`: the banner must re-read, since
///   a pair that was not confirmed may or may not still be open. `behind_the_record` carries the
///   strongest [`Recorded`] (`max`: `No` over `Unknown`) of the failed pairs that were
///   `Retry::AfterReread`, so that re-read knows the list is stale (#797 review) and words its
///   out-of-date notice for the cause (#811).
///
/// The `reload` is `true` even when every recorded pair was `Outranked` (see
/// [`LinkReportView::reload`]): whatever was recorded, the banner must be re-read.
pub fn different_people_report(results: Vec<PairResult>) -> Result<LinkReportView, ErrorView> {
    match results.as_slice() {
        // `record_different_people` says "nothing open" as `NothingOpen`, never as an empty
        // list; this arm only keeps the function total, with the same honest sentence.
        [] => return Err(refused(NOTHING_OPEN)),
        [PairResult {
            outcome: Err(e), ..
        }] => return Err(e.clone()),
        _ => {}
    }
    if results.iter().all(|r| r.outcome.is_err()) {
        return Err(ErrorView {
            text: report_sentence(&results),
            retry: combined_retry(results.iter().filter_map(|r| r.outcome.as_ref().err())),
        });
    }
    Ok(LinkReportView {
        sentence: report_sentence(&results),
        reload: true,
        charts_after: None,
        behind_the_record: results
            .iter()
            .filter_map(|r| match r.outcome.as_ref().err()?.retry {
                Retry::AfterReread(recorded) => Some(recorded),
                _ => None,
            })
            .max(),
    })
}

/// Every pair's sentence, in order, as one line.
fn report_sentence(results: &[PairResult]) -> String {
    results
        .iter()
        .map(pair_sentence)
        .collect::<Vec<_>>()
        .join(" ")
}

/// The retry advice for several failed pairs. **Pure.**
///
/// [`Retry::AfterReread`] first: if ANY pair's commit was interrupted (#713), that pair may have
/// been recorded, and if any pair was refused because the record changed under the screen
/// (`Stale`, #797), the picture judged from is out of date — either way the window must re-read
/// before anything is offered again, whatever the other pairs said. Of the two, the stale refusal
/// wins (#811 review): it proves the list is out of date. The unknown pair's own sentence in the
/// text still says that its outcome is not known. Otherwise the MOST retryable among them — `Now` over `AfterOperator` over
/// `Never` — because a `Never` beside an outage would tell the webview the outcome was DECIDED
/// and take the button away, claiming a verdict that does not exist. Keeping the button for an
/// outage cannot re-sign blindly: "Different people" re-reads the open pairs fresh on every
/// press, and a refused pair is refused again, before anything is signed.
fn combined_retry<'a>(errors: impl Iterator<Item = &'a ErrorView>) -> Retry {
    let rank = |r: Retry| match r {
        Retry::Never => 0,
        Retry::AfterOperator => 1,
        Retry::Now => 2,
        Retry::AfterReread(Recorded::Unknown) => 3,
        Retry::AfterReread(Recorded::No) => 4,
    };
    // Never empty (only called with two or more failures); were it ever, nothing was decided,
    // so the fallback keeps the button rather than claim a verdict.
    errors
        .map(|e| e.retry)
        .max_by_key(|r| rank(*r))
        .unwrap_or(Retry::Now)
}

/// `text` ending in a full stop, so pair sentences joined by a space read as sentences.
fn full_stop(text: &str) -> String {
    if text.ends_with('.') {
        text.to_string()
    } else {
        format!("{text}.")
    }
}

/// `text` as the continuation of a sentence that something else began: its first word lowered
/// when it is merely capitalised ("The" → "the"), left alone when it is all capitals ("NOT", an
/// acronym) or already lower-case. **Pure.**
pub(crate) fn continuation(text: &str) -> String {
    let mut chars = text.chars();
    let (Some(first), Some(second)) = (chars.next(), chars.next()) else {
        return text.to_string();
    };
    if first.is_uppercase() && second.is_lowercase() {
        first
            .to_lowercase()
            .chain(text[first.len_utf8()..].chars())
            .collect()
    } else {
        text.to_string()
    }
}

/// One pair's sentence. A failed pair is led by its charts and then CONTINUES its own error's
/// sentence (#740) — "For charts A and B, the … was not confirmed: …" — so an outage reads "not
/// confirmed" once, a refusal reads "refused" (decided, never "not confirmed"), and a
/// commit-unknown reads "NOT known". Never "NOT recorded" and never "it stays on the banner": a
/// commit-unknown error may have recorded it, and only the banner's re-read says whether the
/// pair is still open.
fn pair_sentence(r: &PairResult) -> String {
    let (low, high) = (r.low, r.high);
    match &r.outcome {
        Ok(LinkEffect::TookEffect) => format!(
            "Recorded: charts {low} and {high} are different people. This pair is \
             closed on this node, and on every node once the judgement syncs."
        ),
        Ok(LinkEffect::StillJoined) => format!(
            "Recorded that charts {low} and {high} are different people — but they read as one \
             record through other links, so nothing was split. The links joining them are listed \
             under \"How these charts are linked\"."
        ),
        Ok(LinkEffect::Outranked) => format!(
            "Recorded that charts {low} and {high} are different people, but a judgement already \
             standing for that pair outranks it; the charts stay as that judgement left them."
        ),
        Err(e) => full_stop(&format!(
            "For charts {low} and {high}, {}",
            continuation(&e.text)
        )),
    }
}

#[cfg(test)]
#[path = "verdict_tests.rs"]
mod tests;
