//! What a judgement on a record pair DID, or why it failed, as pure functions: the outcome and
//! failure sentences that Link, Unlink, Confirm and "Different people" share. Split out of
//! `view.rs` (#781), which keeps the comparison's own sentences.
//!
//! The wording IS the safety content (principle 3): an outcome line never calls a judgement
//! that did not take effect "linked", and a failure never offers a live retry over a commit
//! whose outcome is unknown (#713).
use crate::funnel::view::{ErrorView, Retry};
use cairn_gui_data::port::DataError;
use cairn_medication_view::ChartSet;
use cairn_node::chart_link::LinkEffect;
use serde::Serialize;
use uuid::Uuid;

/// What every judgement command hands back (`link_records`, `unlink_records`, `confirm_link`,
/// `record_different_people`): the sentence for the outcome line, and whether the chart
/// should be re-read (it should whenever the record may have changed).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LinkReportView {
    pub sentence: String,
    /// Re-read the chart. For Link and Unlink, `false` means the judgement did NOT change the
    /// record — only [`LinkEffect::Outranked`] — and the webview relies on exactly that: `link.js`
    /// and `unlink.js` keep the panel open and show the sentence there once, instead of closing it
    /// and re-reading. A new `false` case must be worded for that panel.
    ///
    /// "Different people" (R5a, `duplicates::verdict::different_people_report`) is the second
    /// producer: it is `true` whenever ANY pair was recorded — even an all-`Outranked` one —
    /// because the banner must be re-read whatever the record did.
    pub reload: bool,
}

/// A refusal the clinician cannot retry as-is.
pub fn refused(text: impl Into<String>) -> ErrorView {
    ErrorView {
        text: text.into(),
        retry: Retry::Never,
    }
}

/// A refusal because the screen is behind the record (a chart joined, a pair was judged by a
/// colleague, a link went). `Retry::AfterReread`: the window re-reads the WHOLE chart and takes
/// the act's button away, because pressing it again over the same screen would be refused
/// again. Distinct from [`refused`] (`Retry::Never`), which is for a refusal a re-read cannot
/// mend (a wrong chart id, fixture mode).
pub fn stale(text: impl Into<String>) -> ErrorView {
    ErrorView {
        text: text.into(),
        retry: Retry::AfterReread,
    }
}

/// Link pressed while the signing key is locked. NOT a verdict: nothing about the comparison
/// was wrong, and once the clinician unlocks the key the identical Link succeeds — so it keeps
/// the button (`Retry::Now`). As `Retry::Never` the webview took the comparison away and forced
/// a second Compare, an act the paper workflow does not have (PR #707 review).
pub fn key_locked() -> ErrorView {
    key_locked_for("Link — same person")
}

/// [`key_locked`] for any judgement button: the message must name the button the clinician
/// actually pressed — on a safety panel, naming the opposite act ("press Link" after an
/// Unlink click) is a wrong instruction. Still `Retry::Now`.
pub fn key_locked_for(button: &str) -> ErrorView {
    ErrorView {
        text: format!("your signing key is locked — unlock it, then press \"{button}\" again"),
        retry: Retry::Now,
    }
}

/// The charts `now` combines that were in neither compared set — `compared` is the union of
/// the two sets the panel showed. Empty in the ordinary case.
fn uncompared(now: &ChartSet, compared: &ChartSet) -> Vec<String> {
    now.members()
        .iter()
        .filter(|c| !compared.contains(c))
        .map(Uuid::to_string)
        .collect()
}

/// What the link did, as the outcome line says it. Never "linked" for a link that did not
/// take effect (R2a: recorded is not took effect).
///
/// `charts` is what the record reads as now (the node's read inside the judgement's own
/// transaction); `compared` is both sets the panel showed. If the record now also includes a
/// chart the comparison never showed — a third chart already linked to one side by the time the
/// judgement landed — the sentence names it, so it is reviewed rather than assumed (final review
/// O1): the outcome tells the truth about what the link joined.
pub fn link_report(effect: LinkEffect, charts: &ChartSet, compared: &ChartSet) -> LinkReportView {
    let mut view = effect_report(effect, charts);
    let extra = uncompared(charts, compared);
    if !extra.is_empty() {
        view.sentence = format!(
            "{} The record now also includes chart(s) {} that were not in the comparison — \
             review them.",
            view.sentence,
            extra.join(", ")
        );
    }
    view
}

/// A "same person" judgement that was recorded but lost the overlay to a later "different
/// people" on the pair. Shared by the link's outcome line and the confirm's (`confirm_view.rs`):
/// a confirm IS a link judgement, so it lands, and is worded, exactly as one.
pub(super) const LINK_OUTRANKED: &str = "Recorded, but NOT in effect: a later judgement on this \
     pair says these are different people. The two judgements disagree — settle it with the \
     person who made the other one. Linking again would normally record a newer judgement that \
     overrules theirs — it would not settle the disagreement.";

/// A "same person" judgement that took effect yet left the record unchanged. R2a never returns
/// this for a link; worded honestly in case it ever does — and true wherever it lands (an answer
/// arriving with another chart open re-reads nothing). Shared with the confirm, as above.
pub(super) const LINK_STILL_JOINED: &str = "Recorded, but the record did not change the way a \
     link should — check what the chart now combines.";

/// The outcome sentence for each [`LinkEffect`], before [`link_report`] adds anything about
/// charts the comparison did not show.
fn effect_report(effect: LinkEffect, charts: &ChartSet) -> LinkReportView {
    match effect {
        LinkEffect::TookEffect => LinkReportView {
            sentence: format!(
                "Linked — this record now combines {} charts.",
                charts.members().len()
            ),
            reload: true,
        },
        LinkEffect::Outranked => LinkReportView {
            sentence: LINK_OUTRANKED.into(),
            reload: false,
        },
        // See LINK_STILL_JOINED: R2a never returns this for a link.
        LinkEffect::StillJoined => LinkReportView {
            sentence: LINK_STILL_JOINED.into(),
            reload: true,
        },
    }
}

/// A failed link, worded by the SAME classification the funnel uses (`data_error_from`: a
/// `P0001` floor refusal or a marked `cairn-node` verdict is a verdict; anything else an
/// outage) and the funnel's `Retry` vocabulary (#702). An outage is "not confirmed", never
/// "nothing changed", and keeps `Retry::Now`: it wrote nothing. The one failure that MAY have
/// written — a connection lost during the commit — is marked by the node and never reaches that
/// arm; see [`judgement_error_view`] (#713).
pub fn link_error_view(e: &anyhow::Error) -> ErrorView {
    judgement_error_view("link", e)
}

/// A failed judgement worded for its act — the one entry point every judgement surface (Link,
/// Unlink, Confirm, "Different people") goes through, so the commit-unknown case below cannot be
/// handled in one and missed in another.
///
/// It asks ONE question before the shared classification: was the failure an interrupted
/// COMMIT (marked by `cairn_node::db_diagnosis::commit_outcome_unknown`, read back here by
/// `commit_outcome_unknown_event`)? That judgement may have been
/// recorded, and `data_error_from` would otherwise read it as an outage and keep a live retry —
/// the #713 hazard: a judgement that landed as Outranked leaves the record looking unchanged,
/// so a second press records a second judgement stamped above the colleague's it lost to. It
/// gets [`Retry::AfterReread`] instead: the webview takes the button away (from any open
/// comparison of the same pair) and re-reads the chart when the judgement touched it. The
/// sentence says what a re-read cannot: an Outranked landing leaves the chart looking unchanged.
pub fn judgement_error_view(act: &str, e: &anyhow::Error) -> ErrorView {
    match cairn_node::db_diagnosis::commit_outcome_unknown_event(e) {
        Some(event) => ErrorView {
            text: format!(
                "Whether the {act} was recorded is NOT known — the connection was lost while \
                 event {event} was being saved. If it was recorded but a later judgement outranks it, \
                 the chart will look unchanged after it is read again."
            ),
            retry: Retry::AfterReread,
        },
        None => judgement_error_from(act, cairn_gui_live::error::data_error_from(e)),
    }
}

/// The wording half of [`judgement_error_view`] for every failure that is NOT an interrupted
/// commit, worded for its act (`"link"` / `"unlink"` / `"\"different people\" judgement"`) by the
/// classification the funnel uses. Split out so every arm is unit-tested with no database: the
/// classification (`data_error_from`) is `cairn-gui-live`'s and tested there, and the verdicts
/// `cairn-node` raises are `pub(crate)` to it, so this crate cannot build them.
///
/// `NotProvisioned` says "yet", not "until an operator acts": one of its causes (a chart this
/// node does not hold yet) resolves by sync, with no operator involved (final review M4).
pub fn judgement_error_from(act: &str, error: DataError) -> ErrorView {
    match error {
        DataError::Refused(t) => ErrorView {
            text: format!("The {act} was refused: {t}"),
            retry: Retry::Never,
        },
        DataError::NotProvisioned(t) => ErrorView {
            text: format!("This node cannot record the {act} yet: {t}"),
            retry: Retry::AfterOperator,
        },
        // The record changed under the clinician: nothing was recorded, and the window re-reads
        // the whole chart rather than offering a live retry over a picture that is now wrong.
        DataError::Stale(t) => ErrorView {
            text: format!("The {act} was not recorded: {t}"),
            retry: Retry::AfterReread,
        },
        DataError::Unavailable(t) => ErrorView {
            text: format!("The {act} was not confirmed: {t}"),
            retry: Retry::Now,
        },
        DataError::NotFound => refused(format!("The {act} was not recorded.")),
    }
}

#[cfg(test)]
#[path = "judgement_view_tests.rs"]
mod tests;
