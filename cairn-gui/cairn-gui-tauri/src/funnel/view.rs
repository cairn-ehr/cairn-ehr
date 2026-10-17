//! Every sentence the clerk reads at the front door, and every payload the webview renders, as
//! pure functions.
//!
//! # Why the wording lives in Rust
//!
//! The webview renders and decides nothing (see `src-ui/main.js`'s header). A sentence decided
//! in JavaScript is a sentence no test pins, and on this screen the wording IS the safety
//! content: "the search failed" and "nobody matched" differ by one word and lead to opposite
//! acts — the second licenses a new chart, the first must never (principle 4). So each
//! failure the funnel can meet maps here to exactly one sentence and one piece of retry
//! advice, and the webview only shows them.
//!
//! # The three kinds of retry advice
//!
//! [`Retry`] is the #648 split carried to the screen. An outage (`Unavailable`) decided
//! nothing, so the same act may succeed now. A verdict (`Refused`) will decide the same way
//! every time, so a retry button would be a precise untruth — the clerk's way forward is to
//! change what was typed. A node-state verdict (`NotProvisioned`) is pointless to retry until
//! an operator has acted, and then succeeds.
use crate::funnel::rows::people_phrase;
use cairn_gui_data::port::DataError;
use cairn_gui_funnel::{
    MissingPart, PromptCounts, Restored, TokenError, TriggerState, MIN_NAME_TOKENS,
};
use cairn_node::actor_enrolment::{
    ambiguous_actor_refusal, not_enrolled_refusal, retired_actor_refusal, ActorStanding,
};
use cairn_patient_search::{Candidate, TrustState};
use serde::Serialize;
use uuid::Uuid;

/// What the webview may offer after a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Retry {
    /// Nothing was decided; the same act may succeed now. Keep the button live.
    Now,
    /// This node may not write until its state changes — an operator acts, or sync delivers what
    /// is missing (a link over a chart not held here yet) — and then the same act succeeds.
    AfterOperator,
    /// A verdict, or nothing left to retry with. The way forward is to change the form or wait
    /// for the next search, never to press the same button again. The judgement surfaces also
    /// read it as "the banner may be stale": link.js's `compare` and `sendJudgement` re-read the
    /// possible-duplicate banner on it (#745 point 3) — a new `Never` producer gets that re-read.
    Never,
    /// The screen is behind the record: the window re-reads the WHOLE chart and takes the act's
    /// button away — never a live retry. Two causes share it because the window does the same
    /// for both, and [`Recorded`] says which, because the window must not word them the same
    /// (#811): after an interrupted commit, a change the re-read shows may be the clinician's own.
    AfterReread(Recorded),
}

/// Whether the act a [`Retry::AfterReread`] answers was recorded. On the wire it is
/// `{"after_reread": "no"}` / `{"after_reread": "unknown"}`, read by `chart_stale.js`'s
/// `behindTheRecord`.
///
/// It words only the out-of-date notice a failed re-read leaves (`markListStale`). What an
/// interrupted act may have changed is said in that act's own sentence
/// (`judgement_error_view`), which the window keeps as the re-read's lead.
///
/// Ordered `Unknown < No`, so the value for several failures is their `max`, and `No` wins (#811
/// review): one stale refusal PROVES the list is out of date, which outranks "may be". An
/// unknown pair's own sentence still says that its outcome is unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Recorded {
    /// A write's COMMIT was interrupted (#713): it may have been recorded.
    Unknown,
    /// The record changed under the clinician (a `Stale` refusal, or the window's own set
    /// check): nothing was recorded.
    No,
}

/// A failure, as the clerk reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ErrorView {
    pub text: String,
    pub retry: Retry,
}

/// Where this node stands, as the chrome says it — `None` when there is nothing to say.
///
/// Reuses `cairn-node`'s OWN refusal sentences rather than writing new ones, so the window and
/// the CLI cannot word one node state two ways. `kid` MUST be the key the standing was probed
/// for: a standing carries no subject (#670), and pairing it with the wrong key would hand an
/// operator an identity-level remedy for somebody else's key.
pub fn standing_sentence(standing: ActorStanding, kid: &str) -> Option<String> {
    let refusal = match standing {
        ActorStanding::Enrolled => return None,
        ActorStanding::NeverEnrolled => not_enrolled_refusal(kid),
        ActorStanding::Retired => retired_actor_refusal(kid),
        ActorStanding::Ambiguous => ambiguous_actor_refusal(kid),
    };
    Some(format!(
        "Registering is unavailable at this workstation until an operator acts. {refusal:#}"
    ))
}

/// A search that failed — browse or step 3.
///
/// The outage sentence says NOT in capitals on purpose: an empty-looking list after a failure
/// is read as "nobody matched", which on this screen means "create a new chart".
pub fn search_error_view(e: &DataError) -> ErrorView {
    match e {
        DataError::Unavailable(t) => ErrorView {
            text: format!(
                "The search FAILED — this is NOT a \"no match\". Do not register on the strength \
                 of it. ({t}) Try again."
            ),
            retry: Retry::Now,
        },
        DataError::Refused(t) => ErrorView {
            text: format!("The record refused this search as typed: {t}"),
            retry: Retry::Never,
        },
        DataError::NotProvisioned(t) => ErrorView {
            text: t.clone(),
            retry: Retry::AfterOperator,
        },
        // Search never raises `Stale`, but a match must not hide a new cause. `Never`, not
        // `AfterReread`: the funnel has no chart on screen to re-read and funnel.js never reads
        // `after_reread`, so that label would be one the code does not honour.
        DataError::Stale(t) => ErrorView {
            text: t.clone(),
            retry: Retry::Never,
        },
        DataError::NotFound => ErrorView {
            text: "No such chart.".to_string(),
            retry: Retry::Never,
        },
    }
}

/// A registration that failed, given what happened to its search.
///
/// Two inputs because the advice depends on both: an outage whose search was KEPT can simply
/// be retried, while any failure whose search was dropped (the form changed while it was
/// saving) has nothing left to retry with — the clerk waits for the new search.
pub fn register_error_view(e: &DataError, restored: Restored) -> ErrorView {
    let (base, retry_if_kept) = match e {
        DataError::Unavailable(t) => (
            format!("Nothing was saved; the node could not be reached ({t})."),
            Retry::Now,
        ),
        DataError::Refused(t) => (
            format!(
                "The record refused this registration as typed, and will refuse it again: {t}. \
                 Change what was typed."
            ),
            Retry::Never,
        ),
        DataError::NotProvisioned(t) => (
            format!(
                "This workstation's node may not write yet: {t} Nothing was saved; an operator \
                 must act before registering."
            ),
            Retry::AfterOperator,
        ),
        // Registration never raises `Stale`; if it ever does, nothing was saved. `Never`, not
        // `AfterReread`: no chart is on screen to re-read and funnel.js never reads it.
        DataError::Stale(t) => (format!("Nothing was saved: {t}"), Retry::Never),
        DataError::NotFound => ("Nothing was saved.".to_string(), Retry::Never),
    };
    match restored {
        Restored::Kept if retry_if_kept == Retry::Now => ErrorView {
            text: format!("{base} Press Register again."),
            retry: Retry::Now,
        },
        Restored::Kept => ErrorView {
            text: base,
            retry: retry_if_kept,
        },
        Restored::SupersededAndDropped => ErrorView {
            text: format!(
                "{base} The form changed while this was saving — wait for the new search before \
                 registering."
            ),
            retry: Retry::Never,
        },
    }
}

/// A registration the token store refused before anything was sent. Its own `Display` text
/// already says what to do (`cairn_gui_funnel::token`); it is never retried as is.
pub fn token_error_view(e: TokenError) -> ErrorView {
    ErrorView {
        text: e.to_string(),
        retry: Retry::Never,
    }
}

/// What the step-3 search is waiting for, or `None` when it runs now.
///
/// Ends by saying Register still works: the trigger is ADVISORY, never a gate — a mononymous
/// patient or an unknown date of birth never trips it, and must still be registrable
/// (principle 4, the design's *Risks*).
pub fn waiting_sentence(state: &TriggerState) -> Option<String> {
    let TriggerState::Waiting(parts) = state else {
        return None;
    };
    let parts: Vec<String> = parts
        .iter()
        .map(|part| match part {
            MissingPart::NameTokens { have } => format!(
                "at least {MIN_NAME_TOKENS} words of the name ({have} of {MIN_NAME_TOKENS} typed)"
            ),
            MissingPart::BirthDate => "a date of birth".to_string(),
        })
        .collect();
    Some(format!(
        "The record will be searched for this person automatically once it has {}. Register \
         still searches on whatever is typed.",
        parts.join(" and ")
    ))
}

/// The sentence that announces a step-3 prompt, for sighted and screen-reader clerks alike.
///
/// Said out loud because the prompt appears while the clerk is still typing, and because it
/// changes what the Register button MEANS: from "search first" to "none of these". A clerk who
/// is never told either has not been shown the list the registration will swear they saw.
///
/// `incomplete` is the SEARCH's partiality (the node could not read some chart it matched).
/// It decides the wording, not only a separate note: "No existing chart matched" licenses a
/// new chart, so a search that did not finish must never say it (principle 4) — even when it
/// showed nobody, which is exactly when the list, and anything nested in it, is hidden.
///
/// `withheld` is how many further PEOPLE matched but are not shown (ADR-0075, #671): said
/// as "the N closest of M", a count and a way to narrow — never as "not complete". That word
/// is reserved for `incomplete`, because it changes whether "no match" can be trusted, and
/// being cut to five does not. The prompt is a nudge; being cut is its normal state.
///
/// Takes [`PromptCounts`] (read off the prompt by `PromptList::counts`) rather than three bare
/// values: two adjacent `usize` arguments compile just as well swapped (review of #678).
pub fn prompt_summary(counts: &PromptCounts) -> String {
    let PromptCounts {
        shown,
        shown_charts,
        withheld,
        incomplete,
    } = *counts;
    // Appended to a list with rows when the search itself was partial.
    let partial = if incomplete {
        ", but the list is not complete (the reason follows)"
    } else {
        ""
    };
    match shown {
        0 if incomplete => "The search did not finish, and showed nobody — this is NOT a \
                            \"no match\" (the reason follows). Registering now records that \
                            incomplete search."
            .to_string(),
        // Showed nobody because everything was CUT: never the "no match" sentence, which
        // licenses a new chart. Unreachable with a cap of five; kept safe if that changes.
        0 if withheld > 0 => format!(
            "{withheld} existing patient(s) matched but none could be shown here — type more to \
             narrow before registering."
        ),
        0 => "No existing chart matched what is typed. Registering will record that search."
            .to_string(),
        n if withheld > 0 => format!(
            "{} might be this person — the {n} closest of {} matches, \
             listed below{partial}; type more to narrow. Pressing Register now means none of \
             these.",
            people_phrase(n, shown_charts),
            counts.total()
        ),
        n => format!(
            "{} might be this person — listed below{partial}. Pressing \
             Register now means none of these.",
            people_phrase(n, shown_charts)
        ),
    }
}

/// The sentence that announces a browse answer — the step-1 twin of [`prompt_summary`].
///
/// Browse attests nothing, but a clerk who reads "No existing chart matched" here goes on to
/// register, so the same rule holds: a partial answer never reads as a complete one.
pub fn browse_summary(people: usize, charts: usize, incomplete: bool) -> String {
    match (people, incomplete) {
        (0, false) => "No existing chart matched.".to_string(),
        (0, true) => {
            "The search did not finish, and showed nobody — this is NOT a \"no match\".".to_string()
        }
        // One chart per person: today's text, verbatim. Otherwise count people, name charts.
        (n, false) if charts == n => format!("{n} existing chart(s) found."),
        (n, true) if charts == n => {
            format!("{n} existing chart(s) found — the list is not complete.")
        }
        (n, false) => format!("{n} existing patient(s) found ({charts} charts)."),
        (n, true) => {
            format!("{n} existing patient(s) found ({charts} charts) — the list is not complete.")
        }
    }
}

/// A Register that arrived while a chart was already open — "This is them" was clicked while
/// the registration was on its way. The clerk's LAST act was recognising an existing patient,
/// so nothing is written behind that chart.
pub fn register_behind_open_chart_view(open: &ChartHeaderView) -> ErrorView {
    ErrorView {
        text: format!(
            "Nothing was registered: you had already opened the chart of {} (chart {}). To \
             register a new patient, return to the front door first.",
            open.name, open.patient_id
        ),
        retry: Retry::Never,
    }
}

/// A registration that COMMITTED after the clerk had opened another chart. The new chart
/// exists and cannot be un-made (append-only); the window stays on the chart the clerk chose
/// and says, rather than silently switching, that the new one may be a duplicate of it.
pub fn registered_behind_open_chart_view(
    registered: &ChartHeaderView,
    open: &ChartHeaderView,
) -> ErrorView {
    ErrorView {
        text: format!(
            "A new chart was registered for {} (chart {}) while you opened the chart of {} \
             (chart {}). You are still on {}'s chart. If that is this patient, the new chart is \
             a duplicate — report it so the two can be linked.",
            registered.name, registered.patient_id, open.name, open.patient_id, open.name
        ),
        retry: Retry::Never,
    }
}

/// One candidate as a list row shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CandidateView {
    /// The full chart id — what the webview sends back to open this chart.
    pub patient_id: String,
    pub name: String,
    /// `"46 y"`, or `"age not recorded"` — absence named, never a blank.
    pub age: String,
    pub trust: String,
}

/// The age as a row shows it. Absence is named (principle 4: a blank reads as a value).
fn age_label(c: &Candidate) -> String {
    c.age.as_ref().map_or_else(
        || "age not recorded".to_string(),
        |a| format!("{} y", a.years),
    )
}

pub fn candidate_view(c: &Candidate) -> CandidateView {
    CandidateView {
        patient_id: c.patient_id.to_string(),
        name: c.display_name.clone(),
        age: age_label(c),
        trust: c.trust.as_str().to_string(),
    }
}

/// The persistent identity header over an open chart — the wrong-chart affordance.
///
/// `born` carries an age or a date, depending on what was known when the chart was opened: a
/// candidate carries an age and no date of birth (`Candidate` has no dob field), a
/// registration carries the date that was typed. Each says which it is in its text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChartHeaderView {
    pub patient_id: String,
    pub name: String,
    pub born: String,
    pub trust: String,
}

/// The header for a chart picked from a list: exactly what that list row showed.
pub fn header_from_candidate(c: &Candidate) -> ChartHeaderView {
    ChartHeaderView {
        patient_id: c.patient_id.to_string(),
        name: c.display_name.clone(),
        born: age_label(c),
        trust: c.trust.as_str().to_string(),
    }
}

/// The header for a chart just registered: what was typed, with absence named.
///
/// Trust is what the NODE reports for an ordinary registration: `confirmed`. That is db/024's
/// vocabulary, where `unconfirmed` is the identity-pending (John-Doe) state opened only by a
/// registration carrying a `basis` — which the funnel's registration never does. Showing
/// `unconfirmed` here once put a trust state on the header that the next browse of the same
/// chart contradicted (PR #674 review); the mock registers into the same state for that reason.
pub fn header_from_registration(
    id: Uuid,
    raw_name: &str,
    birth_date: Option<&str>,
) -> ChartHeaderView {
    let name = raw_name.trim();
    ChartHeaderView {
        patient_id: id.to_string(),
        name: if name.is_empty() {
            "(no name recorded)".to_string()
        } else {
            name.to_string()
        },
        born: match birth_date.map(str::trim).filter(|d| !d.is_empty()) {
            Some(d) => format!("born {d}"),
            None => "date of birth not recorded".to_string(),
        },
        trust: TrustState::Confirmed.as_str().to_string(),
    }
}

/// The header for a chart opened by `--patient` at launch, where no name was read.
///
/// Says so rather than showing a bare id as if it were a name: the operator runbook and the
/// accessibility pass open charts this way, and a header that looked complete would teach them
/// the wrong thing about what the window knows.
pub fn header_opened_by_id(id: Uuid) -> ChartHeaderView {
    ChartHeaderView {
        patient_id: id.to_string(),
        name: "(opened by chart id at launch — name not read)".to_string(),
        born: "not read".to_string(),
        trust: "not read".to_string(),
    }
}

#[cfg(test)]
#[path = "view_tests.rs"]
pub(crate) mod tests;
