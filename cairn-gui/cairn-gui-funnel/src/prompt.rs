//! The bounded prompt — how many candidates a registration may swear it displayed.
//!
//! # Why this is bounded at all, when the browse list is not
//!
//! The funnel has two searches and only one of them is attested. The browse search at step 1
//! is a moving target — the clerk types fragments, edits freely, and scrolls a long list — so
//! it carries no signed claim and is free to scroll. The step-3 search is the opposite: it
//! runs automatically over the completed registration data, and the pair of
//! (query, displayed list) from the run that preceded the commit is what
//! `cairn_patient_search::SearchAttestation` records permanently into the chart's birth act.
//! (It re-runs as the clerk edits — see [`crate::token`] — but only one run is ever attested.)
//!
//! So this list is a claim about what a human saw. `displayed` means *"the candidate ids that
//! were on the screen"*, and it has to be literally true. Signing that forty were displayed
//! when three were visible is a precise untruth (principle 4) — and it is exactly the claim
//! someone would later use to argue the clerk should have seen the duplicate. Bounding the
//! list is how that sentence stays true.
//!
//! # Why a constant and not a measurement
//!
//! Viewport tracking was considered and rejected in the design: a signed clinical record
//! should not assert "this row was on screen", and no test can pin it. A named constant is
//! the honest alternative — a number a reviewer can argue with, and a prompt laid out to fit
//! it. This is cheap here in a way it would not have been for the browse list, because a
//! search over a full name plus a date of birth returns few candidates by construction.
//!
//! # What "incomplete" must never become
//!
//! [ADR-0060](../../../docs/spec/decisions/0060-partial-validity-a-defect-on-one-line-never-invalidates-another.md)
//! decision 2: partial completion is **reported, never implied**. There are two genuinely
//! different partialities in play and they must not be collapsed:
//!
//! - the *node* could not read every chart the search matched — the search itself was partial;
//! - this prompt could not show every candidate the node returned — the display was partial.
//!
//! The second is milder and commoner. Letting it overwrite the first would silently delete a
//! warning the clerk needs, so [`bound_for_prompt`] keeps both.

use cairn_patient_search::CandidateList;

/// The most candidates the step-3 prompt shows.
///
/// A **clinical** decision — how many existing charts a clerk is shown before being allowed
/// to create another — not a layout detail, so it is named and pinned by a test rather than
/// buried in a slice expression.
///
/// Five is a **guess** at what fits without scrolling together with the question and its two
/// answers — said plainly because there is no pinned minimum window size in the repo yet to
/// derive it from, and dressing a guess in a precise-sounding justification is the shape
/// principle 4 warns about. What makes the guess safe is that being wrong is *reported*: a
/// truncating prompt says so, in the sentence below.
///
/// If it turns out the prompt is *routinely* truncating, the cap is wrong and the design needs
/// revisiting — quietly signing partial lists is the failure this whole module exists to
/// prevent, not a state to get used to. Slice 2b owes a truncation-frequency report.
pub const PROMPT_CAP: usize = 5;

/// How the prompt admits to candidates it did not show.
///
/// Its own function so the sentence is testable on its own, rather than being built inline
/// inside a `match` where no test can reach it.
fn withheld_reason(withheld: usize) -> String {
    format!(
        "{withheld} further candidate(s) matched this search and are NOT shown here, so this \
         list is not the whole answer"
    )
}

/// Join whatever the node said about its own partiality to whatever this prompt has to add.
///
/// Neither ever replaces the other — see the module doc. `" · "` rather than a newline
/// because this reaches a single status line on screen.
fn combine_reasons(from_node: Option<&str>, from_truncation: Option<String>) -> Option<String> {
    match (from_node, from_truncation) {
        (None, None) => None,
        (Some(node), None) => Some(node.to_string()),
        (None, Some(cut)) => Some(cut),
        (Some(node), Some(cut)) => Some(format!("{node} · {cut}")),
    }
}

/// A candidate list that has been bounded to what a prompt can truthfully claim it showed.
///
/// # Why this is a type and not just a `CandidateList`
///
/// `TokenStore::record` freezes its list into the pair a registration attests to, and the
/// rule above says `displayed` *"has to be literally true"*. But a bounded list and a raw
/// node list are the same shape, so nothing stopped a caller passing the node's forty
/// candidates straight into `record` and signing that forty were displayed when five were on
/// screen — the precise untruth this whole module is written to forbid, reachable by the
/// shortest path anyone would write.
///
/// So the bounding is in the type. [`bound_for_prompt`] is the only way to obtain a
/// `PromptList`, and `record` accepts nothing else; the private field is what makes that
/// stick. This is the same trick `SearchAttestation::from_displayed` plays one layer down,
/// applied one layer up.
///
/// It bounds only what gets *signed*. The matching hazard in the node —
/// `cairn_search_candidates` returning thousands of candidates for a common surname in the
/// first place — is [#357](https://github.com/cairn-ehr/cairn-ehr/issues/357), and this type
/// does not substitute for it: a prompt that truncates honestly is still a prompt that could
/// not show the clerk the chart they needed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptList(CandidateList);

impl PromptList {
    /// The bounded list, for the caller that has to render or sign it.
    ///
    /// Read-only on purpose: handing out `&mut` would let a caller extend the list after the
    /// bounding that made it truthful.
    pub fn as_list(&self) -> &CandidateList {
        &self.0
    }
}

/// Bound a node-returned list to what the step-3 prompt can truthfully claim it displayed.
///
/// Always bounds at [`PROMPT_CAP`], because the cap is a property of the prompt rather than
/// of the call site: a caller that could choose its own bound could choose `usize::MAX` and
/// be back where it started. The cap is exercised at other values through the private
/// `bound_to`, which the tests below use to reach the edges.
pub fn bound_for_prompt(list: &CandidateList) -> PromptList {
    bound_to(list, PROMPT_CAP)
}

/// The bounding itself, at an arbitrary cap.
///
/// Private so [`PROMPT_CAP`] stays the only bound reachable from outside, but a free function
/// with an explicit cap so the off-by-one, the exactly-at-the-cap and the zero cases are each
/// testable without building a prompt.
///
/// Total by construction: any `cap`, including zero, yields a list rather than a panic. A
/// zero cap is useless but still honest — it reports that it showed nobody, which is a
/// different statement from the search having matched nobody.
///
/// Order is preserved exactly. `SearchAttestation::from_displayed` reads the candidate vector
/// in order, so a reorder here would silently change what gets signed.
fn bound_to(list: &CandidateList, cap: usize) -> PromptList {
    let withheld = list.candidates.len().saturating_sub(cap);
    PromptList(CandidateList {
        candidates: list.candidates.iter().take(cap).cloned().collect(),
        // `incomplete` is the OR of the two partialities. It can only ever be turned ON here:
        // a list the node already called partial must never be laundered into a complete one
        // by fitting inside the cap.
        //
        // `node_reason` rather than `list.incomplete_reason` directly: a node that set the
        // flag with no prose would otherwise hand the clerk ONLY the truncation sentence, so
        // the milder cause would read as the whole story. The flag survives either way; this
        // keeps the attribution too.
        incomplete: list.incomplete || withheld > 0,
        incomplete_reason: combine_reasons(
            node_reason(list),
            (withheld > 0).then(|| withheld_reason(withheld)),
        ),
    })
}

/// What the node said about its own partiality, never silently nothing.
///
/// `CandidateList`'s doc says `incomplete_reason` is `Some` whenever `incomplete` — but that
/// is a comment on a struct with public fields, not a type, and this is the one function whose
/// stated job is keeping the two partialities distinct. A bare flag becomes a sentence rather
/// than being dropped. Public because every list a clerk reads needs it — the unbounded browse
/// list as much as the prompt (PR #674 review found browse dropping a bare flag).
pub fn node_reason(list: &CandidateList) -> Option<&str> {
    match (list.incomplete, list.incomplete_reason.as_deref()) {
        (true, None) => Some("the node reported this search was not exhaustive but gave no reason"),
        (_, reason) => reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_patient_search::{Candidate, TrustState};
    use uuid::Uuid;

    /// A candidate distinguishable by its id, which is the only field bounding touches.
    fn candidate(n: u128) -> Candidate {
        Candidate {
            patient_id: Uuid::from_u128(n),
            display_name: format!("Patient {n}"),
            age: None,
            trust: TrustState::Confirmed,
            last_activity: None,
            locale: None,
            photo_ref: None,
        }
    }

    /// The common case: a node list whose partiality, if any, came with prose.
    fn list_of(n: u128, incomplete_reason: Option<&str>) -> CandidateList {
        list_with(n, incomplete_reason.is_some(), incomplete_reason)
    }

    /// `incomplete` and its reason set INDEPENDENTLY.
    ///
    /// `list_of` derives the flag from the prose, which is the shape a well-behaved node
    /// produces — and that made `incomplete: true` with no reason unreachable in every test,
    /// hiding whether the bounding preserved a bare flag's attribution. It is a legal shape
    /// (`CandidateList`'s fields are public), so it needs a builder that can express it.
    fn list_with(n: u128, incomplete: bool, incomplete_reason: Option<&str>) -> CandidateList {
        CandidateList {
            candidates: (1..=n).map(candidate).collect(),
            incomplete,
            incomplete_reason: incomplete_reason.map(str::to_string),
        }
    }

    fn ids(list: &CandidateList) -> Vec<Uuid> {
        list.candidates.iter().map(|c| c.patient_id).collect()
    }

    #[test]
    fn the_prompt_cap_is_five() {
        // Pinned so a change is visible in a diff and has to be argued for.
        assert_eq!(PROMPT_CAP, 5);
    }

    #[test]
    fn a_list_that_fits_comes_back_untouched() {
        // Bounding must be a no-op when there is nothing to bound, or every honest prompt
        // starts claiming a partiality it does not have — and a warning that fires on every
        // screen is a warning nobody reads.
        let list = list_of(3, None);
        assert_eq!(bound_for_prompt(&list).as_list(), &list);
    }

    #[test]
    fn a_list_exactly_the_size_of_the_cap_is_not_truncated() {
        // The off-by-one that would mark a full-but-complete prompt as partial.
        let list = list_of(PROMPT_CAP as u128, None);
        let bounded = bound_for_prompt(&list);
        assert!(
            !bounded.as_list().incomplete,
            "{:?}",
            bounded.as_list().incomplete_reason
        );
        assert_eq!(bounded.as_list().candidates.len(), PROMPT_CAP);
    }

    #[test]
    fn a_list_one_longer_than_the_cap_is_reported_as_truncated() {
        // THE OTHER SIDE OF THAT BOUNDARY, and the one that was missing: every other test
        // withholds 0, 3, 4 or 5 candidates, so `withheld > 0` could have been `withheld > 1`
        // and stayed green. Under that mutation a six-candidate result shows five, hides ONE,
        // and calls itself complete — and the one hidden candidate is exactly the duplicate
        // the funnel exists to surface.
        let bounded = bound_for_prompt(&list_of(PROMPT_CAP as u128 + 1, None));
        assert_eq!(bounded.as_list().candidates.len(), PROMPT_CAP);
        assert!(bounded.as_list().incomplete, "hiding one is still hiding");
        let reason = bounded
            .as_list()
            .incomplete_reason
            .clone()
            .expect("a reason, not a bare flag");
        assert!(reason.contains('1'), "must name the count: {reason}");
    }

    #[test]
    fn a_longer_list_keeps_the_first_cap_candidates_in_display_order() {
        // Order is the attestation's order — `SearchAttestation::from_displayed` reads this
        // vector in sequence — so a reorder here silently changes what gets signed.
        let bounded = bound_to(&list_of(8, None), 3);
        assert_eq!(
            ids(bounded.as_list()),
            vec![Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3)]
        );
    }

    #[test]
    fn truncating_marks_the_list_incomplete_and_says_how_many_were_withheld() {
        // ADR-0060 decision 2: reported, never implied. A bare `incomplete: true` is a flag
        // a clerk cannot act on.
        let bounded = bound_to(&list_of(8, None), 5);
        assert!(bounded.as_list().incomplete);
        let reason = bounded
            .as_list()
            .incomplete_reason
            .clone()
            .expect("a reason, not a bare flag");
        assert!(reason.contains('3'), "must name the count: {reason}");
    }

    #[test]
    fn the_nodes_own_reason_is_never_overwritten_by_the_truncation_reason() {
        // THE ONE THAT MATTERS. These are two different partialities: the node could not
        // READ two charts the search matched, and this prompt could not SHOW three it
        // returned. The first is the more serious and the less obvious, and replacing it
        // with the second deletes a warning the clerk needs in order to distrust a zero.
        let node_said = "2 charts could not be read";
        let bounded = bound_to(&list_of(8, Some(node_said)), 5);
        let reason = bounded
            .as_list()
            .incomplete_reason
            .clone()
            .expect("a reason");
        assert!(reason.contains(node_said), "node's reason lost: {reason}");
        assert!(reason.contains('3'), "truncation not reported: {reason}");
    }

    #[test]
    fn a_bare_partiality_flag_from_the_node_keeps_its_own_attribution() {
        // `incomplete: true` with no prose is a legal shape, and a truncating prompt used to
        // hand the clerk ONLY the truncation sentence for it — so display truncation read as
        // the whole story while the node had in fact also failed to read charts. The flag
        // survived; the attribution did not. That is the same collapse this module forbids,
        // arriving by a different door.
        let bounded = bound_to(&list_with(8, true, None), 5);
        let reason = bounded
            .as_list()
            .incomplete_reason
            .clone()
            .expect("a bare flag must still produce a sentence");
        assert!(
            reason.contains("not exhaustive"),
            "the node's partiality must still be attributed: {reason}"
        );
        assert!(reason.contains('3'), "truncation not reported: {reason}");
    }

    #[test]
    fn a_partial_search_that_fits_inside_the_cap_stays_partial() {
        // The other direction of the same rule: `incomplete` may only ever be turned ON
        // here. A short list the node called partial must not be laundered into a complete
        // one just because it fitted.
        let bounded = bound_for_prompt(&list_of(2, Some("1 chart could not be read")));
        assert!(bounded.as_list().incomplete);
        assert_eq!(
            bounded.as_list().incomplete_reason.as_deref(),
            Some("1 chart could not be read"),
            "nothing to add, so nothing should be added"
        );
    }

    #[test]
    fn a_cap_of_zero_yields_an_empty_list_that_admits_it_is_empty_by_truncation() {
        // Kept total rather than panicking, but it must never look like "found nothing":
        // the reason is what separates a search that matched nobody from a prompt that
        // showed nobody. Both display as an empty list; only one of them is an answer.
        let bounded = bound_to(&list_of(4, None), 0);
        assert!(bounded.as_list().candidates.is_empty());
        assert!(bounded.as_list().incomplete);
        let reason = bounded
            .as_list()
            .incomplete_reason
            .clone()
            .expect("a reason");
        assert!(reason.contains('4'), "must name what it hid: {reason}");
    }

    #[test]
    fn an_empty_search_result_is_complete_not_truncated() {
        // A genuine zero — nobody matched — is an exhaustive, true answer. Marking it
        // partial would teach a clerk to distrust the one result the funnel most needs them
        // to trust before they create a chart.
        let bounded = bound_for_prompt(&list_of(0, None));
        assert!(bounded.as_list().candidates.is_empty());
        assert!(!bounded.as_list().incomplete);
        assert_eq!(bounded.as_list().incomplete_reason, None);
    }

    #[test]
    fn the_withheld_sentence_names_the_count_and_says_the_list_is_not_the_whole_answer() {
        // Tested directly because it is the sentence a clerk actually reads, and a reason
        // that says "incomplete" without saying what is missing is not a report.
        let reason = withheld_reason(7);
        assert!(reason.contains('7'), "{reason}");
        assert!(reason.contains("NOT shown"), "{reason}");
    }

    #[test]
    fn combining_reasons_never_drops_one_of_them() {
        assert_eq!(combine_reasons(None, None), None);
        assert_eq!(combine_reasons(Some("a"), None).as_deref(), Some("a"));
        assert_eq!(
            combine_reasons(None, Some("b".into())).as_deref(),
            Some("b")
        );
        assert_eq!(
            combine_reasons(Some("a"), Some("b".into())).as_deref(),
            Some("a · b")
        );
    }
}
