//! How a failed NODE write is handed to the webview as an `ErrorView` (sign-off, cease, and the
//! withhold and release of `hold`, #832).
//!
//! Sign-off and cease used to return a bare `String`, so the webview could not tell "the record
//! moved under you" from "the node refused" from "the node is unreachable" and treated all of
//! them as a dead end. The node's failure already carries that distinction (the funnel's
//! `data_error_from` classification), so this module only maps it onto the funnel's `Retry`
//! vocabulary, passing the node's own legible sentence (§9.6) through untouched. One case is
//! worded here instead: a commit lost mid-flight, which may have written, so it is neither
//! "failed" nor done (#814, #818: a sign-off's `sign_off_outcome`, a cease's `cease_outcome`, a
//! withhold's or release's `hold::outcome`).
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
        // The failing step wrote nothing: an outage. (A commit that MAY have written is
        // `OutcomeUnknown`, below; a sign-off that signed a line returns no error at all, #818.)
        DataError::Unavailable(_) => Retry::Now,
        // A commit that may have written: re-read, never a live retry (#713, #815).
        DataError::OutcomeUnknown(_) => Retry::AfterReread(Recorded::Unknown),
    }
}

/// A failed node write as the view the webview reads: the node's own text, the retry its
/// classification earns. A failure that is not a classified node verdict is an outage
/// (`Retry::Now`), which is what `data_error_from` makes of it — except a commit the node marked
/// outcome-unknown (#713): `data_error_from` classifies that as `OutcomeUnknown` (#815), so it
/// re-reads (`Unknown`), never `Now`, whether or not the caller words it first. And except a
/// lost connection the server refused to reopen, which needs an operator (`AfterOperator`, #880).
pub(crate) fn node_failure(e: &anyhow::Error) -> ErrorView {
    ErrorView {
        text: format!("{e:#}"),
        retry: retry_for(&cairn_gui_live::error::data_error_from(e)),
    }
}

/// Whether a cease or release that has run its writes must answer with a REFUSAL instead of a
/// report, and with which class (#840). A refusal exactly when NOTHING was written: no write
/// succeeded (`written == 0`), none has an unknown outcome (`unknown` empty — that may have been
/// written, so it is reported), and at least one was tried and failed (`retries`, those
/// failures' classes in order). As a report it would wipe the typed reason and drop the class.
///
/// Stale wins: one failure that proved the list out of date makes the whole gesture "re-read
/// first" — the safe superset, as a live retry over a stale list is only refused again.
/// Otherwise the first failure's class. `None` when something was written, an outcome is
/// unknown, or nothing was tried (every thread held back — only a cease reaches the loop so; a
/// release answers that before the key): the caller's report stands.
pub(crate) fn retry_when_nothing_written(
    written: usize,
    unknown: &[UnknownThread],
    retries: &[Retry],
) -> Option<Retry> {
    if written > 0 || !unknown.is_empty() {
        return None;
    }
    let stale = Retry::AfterReread(Recorded::No);
    if retries.contains(&stale) {
        return Some(stale);
    }
    retries.first().copied()
}

/// What one sign-off did, as the outcome line says it (ADR-0060: partial completion is reported,
/// never implied): the lines signed, those NOT signed (`failed`, each already named), and — when
/// the gesture stopped early (#818) — the line it stopped at and the lines it never tried. A line
/// whose commit got no ordinary answer (#814) may be signed, so it is neither, and "Signed 0" is
/// left out beside it (the count would claim that line is not signed). `notes` are the chart's
/// own reports (withheld, missing, elsewhere, linked charts), appended as given. Rust, not the webview, so
/// every combination is tested.
///
/// Returns the sentence AND the report's `behind_the_record` (`Unknown` exactly when the sentence
/// says a line may be signed), from one match, so the two cannot disagree.
pub(crate) fn sign_off_outcome(
    signed: usize,
    failed: &[String],
    interrupted: Option<&cairn_node::medication::signoff::SignOffInterrupted>,
    notes: &[Option<String>],
) -> (String, Option<Recorded>) {
    use cairn_node::medication::signoff::StoppedLine;
    let unknown = match interrupted.map(|i| i.line) {
        Some(StoppedLine::MayBeSigned { event }) => Some(event),
        Some(StoppedLine::NotSigned) | None => None,
    };
    let mut parts = Vec::new();
    if signed > 0 || unknown.is_none() {
        parts.push(format!("Signed {signed} medication thread(s)."));
    }
    if !failed.is_empty() {
        parts.push(format!(
            "{} line(s) could NOT be signed and were left unsigned: {}",
            failed.len(),
            failed.join("; ")
        ));
    }
    if let Some(stopped) = interrupted {
        let line = stopped.medication_id;
        let cause = &stopped.cause;
        parts.push(match unknown {
            Some(event) => format!(
                "Whether thread {line} was signed is NOT known — no ordinary answer came back \
                 while event {event} was being saved ({cause}). Once the window can reach the \
                 node again, the chart shows whether it is signed."
            ),
            None => {
                format!("The sign-off stopped at thread {line}, which was NOT signed ({cause}).")
            }
        });
        if !stopped.not_attempted.is_empty() {
            let untried: Vec<String> = stopped
                .not_attempted
                .iter()
                .map(|t| t.to_string())
                .collect();
            parts.push(format!(
                "{} line(s) after it were not tried and are still unsigned: {}.",
                untried.len(),
                untried.join(", ")
            ));
        }
    }
    parts.extend(notes.iter().flatten().cloned());
    (parts.join(" "), unknown.map(|_| Recorded::Unknown))
}

/// The sign-off orchestrator words its failures as TEXT: a stop by its ROOT cause, a failed line
/// by its chain. For a hold the window cut off both end in only "connection closed", the text of
/// a node restart too, so name the cut in them (#893; `name_cut_in`). The failed lines matter as
/// much as the stop: a one-line sign-off cut during its attest has no stop at all. A line refused
/// for its own reason keeps its words. No I/O.
pub(crate) fn name_the_cut(
    stopped: Option<&mut cairn_node::medication::signoff::SignOffInterrupted>,
    failed: &mut [cairn_node::medication::signoff::FailedLine],
    cut: Option<cairn_gui_live::CutOff>,
) {
    if let Some(stopped) = stopped {
        stopped.cause = cairn_gui_live::name_cut_in(&stopped.cause, cut);
    }
    for line in failed {
        line.error = cairn_gui_live::name_cut_in(&line.error, cut);
    }
}

/// A thread whose write (a cease, a withhold, a release) got no ordinary answer at commit
/// (#814): the write may or may not have landed, so the thread is neither done nor failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UnknownThread {
    /// The member thread being written.
    pub thread: uuid::Uuid,
    /// The event that may have landed.
    pub event: uuid::Uuid,
    /// The error's root cause, as the clinician reads it.
    pub cause: String,
}

/// What one cease did, as the outcome line says it (ADR-0060: partial completion is reported,
/// never implied): the threads stopped, those NOT stopped (`failed`, each already named), and
/// those whose commit got no ordinary answer (#814), which may or may not be stopped, so they
/// are neither. "Stopped 0" is left out when one is unknown: that thread may be stopped, so the
/// count would be false. Rust, not the webview, so every combination is tested.
pub(crate) fn cease_outcome(ceased: usize, failed: &[String], unknown: &[UnknownThread]) -> String {
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
            .map(|u| format!("thread {} (event {}: {})", u.thread, u.event, u.cause))
            .collect();
        parts.push(format!(
            "Whether {} thread(s) of this drug were stopped is NOT known — no ordinary answer \
             came back while saving {}. Once the window can reach the node again, the chart \
             shows whether each is still active.",
            unknown.len(),
            named.join("; ")
        ));
    }
    parts.join(" ")
}

/// The note a cease outcome carries when the press-time read shows the line ALREADY stopped
/// (#843): a colleague's cease landed after this screen was drawn. The stop is still written,
/// so this does not refuse; it says what the record already held, so "Stopped 1" is not read
/// as "this press stopped it". The sentence and the "already stopped" rule are the CLI's too
/// (`MedicationRow::already_stopped_note`, #854). `None` for a line still active, and unless
/// this press `landed_in_full` (every thread stopped for certain): beside a commit with no
/// answer (#814) or a failed thread, "your stop is recorded" would sit next to the outcome's
/// own "NOT known" / "NOT stopped" (#853 review). Pure.
pub(crate) fn already_stopped_note(
    row: &cairn_medication_view::MedicationRow,
    landed_in_full: bool,
) -> Option<String> {
    if !landed_in_full {
        return None;
    }
    row.already_stopped_note()
}

/// The chart read a write starts with failed (sign-off's set check, cease's line lookup): an
/// OUTAGE — nothing was decided, let alone written — so the act may be retried (`Retry::Now`), the
/// retry `link::chart_set_of` gives the same fault. Never `refused`, whose `Never` would claim a
/// verdict that does not exist (#797 review). The read's own text passes through unprefixed.
/// Except a lost connection, which keeps its own class and wording ("Nothing was done: the …",
/// `ErrorView: From<ConnectionLost>`): this read is where a write gesture reopens it, and a
/// reopen the server refused needs an operator, not a retry-now (#880).
pub(crate) fn chart_read_failed(e: anyhow::Error) -> ErrorView {
    match e.downcast::<cairn_gui_live::ConnectionLost>() {
        Ok(lost) => lost.into(),
        Err(e) => ErrorView {
            text: format!("{e:#}"),
            retry: Retry::Now,
        },
    }
}

/// No key signed this act (`crate::signer::KeyRefusal`): not a verdict on the gesture. The
/// identical act succeeds once the clinician types a right staff ID and passphrase (or waits
/// out a back-off), so it keeps the button (`Retry::Now`) - the vocabulary of
/// `link::judgement_view::key_locked`. The text is the refusal's own sentence, which carries
/// no database text and never the passphrase.
pub(crate) fn key_locked(refusal: &crate::signer::KeyRefusal) -> ErrorView {
    ErrorView {
        text: refusal.sentence(),
        retry: Retry::Now,
    }
}

/// The window's connection was lost and could not be reopened (#876): nothing was done. The
/// retry is `DataError`'s, so ONE place classifies a lost connection into a retry: the next
/// press, or an operator when the server refused the reopen (#880). The text carries the cause.
impl From<cairn_gui_live::ConnectionLost> for ErrorView {
    fn from(lost: cairn_gui_live::ConnectionLost) -> Self {
        ErrorView {
            text: format!("Nothing was done: the {lost}"),
            retry: retry_for(&lost.into()),
        }
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
