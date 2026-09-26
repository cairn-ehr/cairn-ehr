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
//! it. (The design assumed a full name plus a date of birth returns few candidates; measured
//! 2026-09-23 it returns ~100, because `db/046` is a disjunction — see `PROMPT_CAP`'s doc.)
//!
//! # Two partialities, two fields
//!
//! [ADR-0060](../../../docs/spec/decisions/0060-partial-validity-a-defect-on-one-line-never-invalidates-another.md)
//! decision 2: partial completion is **reported, never implied**. There are two genuinely
//! different partialities in play and they must not be collapsed:
//!
//! - the *node* could not read every chart the search matched — the SEARCH was partial. This
//!   is `incomplete` (+ its reason), and it is SIGNED: it is what makes "no match" untrustworthy.
//! - this prompt did not show every candidate the node returned — the DISPLAY was cut. This is
//!   [`PromptList::withheld`], shown on screen as a count and never signed.
//!
//! Until [ADR-0075](../../../docs/spec/decisions/0075-the-step-3-prompt-is-a-nudge-not-a-completeness-claim.md)
//! (2026-09-26, #671) the second was OR-ed into the first, which set the signed flag on 92% of
//! registrations and so made it mean nothing. The prompt is a best-effort nudge; being cut is
//! its normal state, not a defect of the search. Folding `withheld` back into `incomplete`
//! reinstates exactly that.

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
/// principle 4 warns about.
///
/// Measured 2026-09-23: the prompt routinely truncates (92% of registrations over 50,000 real
/// names), and ADR-0075 decided that is its normal state — the prompt is a nudge showing the
/// closest few (the ranking decides which), not a completeness claim; duplicates are repaired
/// by `link`. So five is a layout choice now, not a completeness boundary. **Do not raise it
/// to make a number look better:** a longer list is exactly what the person at the desk will
/// not read.
pub const PROMPT_CAP: usize = 5;

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
/// `PromptList`, and `record` accepts nothing else; the private fields are what make that
/// stick. This is the same trick `SearchAttestation::from_displayed` plays one layer down,
/// applied one layer up.
///
/// It bounds only what gets *signed*. The matching hazard in the node —
/// `cairn_search_candidates` returning thousands of candidates for a common surname in the
/// first place — is [#357](https://github.com/cairn-ehr/cairn-ehr/issues/357), and this type
/// does not substitute for it: a prompt that truncates honestly is still a prompt that could
/// not show the clerk the chart they needed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptList {
    list: CandidateList,
    /// How many candidates the node returned that this prompt did not show. Shown on screen
    /// (ADR-0075 decision 4), never signed — see the module doc.
    withheld: usize,
}

impl PromptList {
    /// The bounded list, for the caller that has to render or sign it.
    ///
    /// Read-only on purpose: handing out `&mut` would let a caller extend the list after the
    /// bounding that made it truthful.
    pub fn as_list(&self) -> &CandidateList {
        &self.list
    }

    /// How many further candidates matched but are not shown — for the on-screen line
    /// ("the 5 closest of 103"), never for the signature. Read-only, like [`Self::as_list`].
    pub fn withheld(&self) -> usize {
        self.withheld
    }

    /// The three facts the on-screen announcement is built from, as one named value.
    ///
    /// Why not three getters passed to `prompt_summary`: two adjacent `usize` arguments compile
    /// just as well swapped, and the swap reads "98 might be this person — the 98 closest of
    /// 103" (review of #678). Named fields make a swap visible at the call site.
    pub fn counts(&self) -> PromptCounts {
        PromptCounts {
            shown: self.list.candidates.len(),
            withheld: self.withheld,
            incomplete: self.list.incomplete,
        }
    }
}

/// What the step-3 announcement says, read off a [`PromptList`] by [`PromptList::counts`].
///
/// Plain data with public fields, so a view test can state any combination — including shapes
/// today's cap never produces (showed nobody because everything was cut) that the wording must
/// still get right. Production code takes it from `counts()`, where it is consistent by
/// construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PromptCounts {
    /// Rows on screen — the ones a registration signs as displayed.
    pub shown: usize,
    /// Further candidates that matched but are not shown. Said, never signed (ADR-0075).
    pub withheld: usize,
    /// The SEARCH was partial (the node could not read a chart it matched) — ADR-0061's meaning.
    pub incomplete: bool,
}

impl PromptCounts {
    /// Everything the node returned: the "M" of "the N closest of M". Owned here, not
    /// recomputed by each caller.
    pub fn total(&self) -> usize {
        self.shown + self.withheld
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
/// zero cap is useless but still honest — `withheld` says it showed nobody, which is a
/// different statement from the search having matched nobody.
///
/// Order is preserved exactly. `SearchAttestation::from_displayed` reads the candidate vector
/// in order, so a reorder here would silently change what gets signed.
fn bound_to(list: &CandidateList, cap: usize) -> PromptList {
    PromptList {
        list: CandidateList {
            candidates: list.candidates.iter().take(cap).cloned().collect(),
            // ONLY the node's own partiality (ADR-0075 decision 3, restoring ADR-0061's
            // meaning): the search could not read some candidate. Being cut is `withheld`.
            // Copied, never recomputed, so a list the node called partial can never be
            // laundered into a complete one here.
            incomplete: list.incomplete,
            // `node_reason` rather than `list.incomplete_reason` directly: a node that set the
            // flag with no prose still reaches the clerk as a sentence. And ONLY when the flag
            // is set: prose beside a list that says it is complete would contradict it, and
            // the flag is what gets signed, so the flag wins (review of #678).
            incomplete_reason: if list.incomplete {
                node_reason(list).map(str::to_string)
            } else {
                None
            },
        },
        withheld: list.candidates.len().saturating_sub(cap),
    }
}

/// What the node said about its own partiality, never silently nothing.
///
/// `CandidateList`'s doc says `incomplete_reason` is `Some` whenever `incomplete` — but that
/// is a comment on a struct with public fields, not a type, and this is the one function whose
/// stated job is keeping the node's partiality intact. A bare flag becomes a sentence rather
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

    /// Review of #678: the "N closest of M" line needs shown, withheld and the search's
    /// partiality together. Handing them out as three bare values let a caller swap the two
    /// counts and still compile, so they travel as one named value read off the prompt.
    #[test]
    fn the_counts_are_read_off_the_prompt_and_total_what_the_node_returned() {
        let counts = bound_to(&list_with(8, true, Some("x")), 5).counts();
        assert_eq!(
            counts,
            PromptCounts {
                shown: 5,
                withheld: 3,
                incomplete: true
            }
        );
        assert_eq!(counts.total(), 8);
    }

    /// Review of #678: a node list claiming a reason for a partiality it does NOT claim is a
    /// contradiction the public fields allow. Passing the prose through would put a "why it is
    /// not complete" sentence beside a list that says it is. The flag wins: no flag, no reason.
    #[test]
    fn a_reason_without_the_flag_is_not_passed_through() {
        let bounded = bound_for_prompt(&list_with(3, false, Some("stray prose")));
        assert!(!bounded.as_list().incomplete);
        assert_eq!(bounded.as_list().incomplete_reason, None);
    }

    #[test]
    fn a_list_exactly_the_size_of_the_cap_withholds_nothing() {
        // The off-by-one that would report a full prompt as cut.
        let bounded = bound_for_prompt(&list_of(PROMPT_CAP as u128, None));
        assert_eq!(bounded.withheld(), 0);
        assert!(!bounded.as_list().incomplete);
        assert_eq!(bounded.as_list().candidates.len(), PROMPT_CAP);
    }

    #[test]
    fn a_list_one_longer_than_the_cap_withholds_exactly_one() {
        // THE OTHER SIDE OF THAT BOUNDARY: a six-candidate result shows five and must say it
        // left ONE out — `saturating_sub` off by one would hide exactly that one.
        let bounded = bound_for_prompt(&list_of(PROMPT_CAP as u128 + 1, None));
        assert_eq!(bounded.as_list().candidates.len(), PROMPT_CAP);
        assert_eq!(bounded.withheld(), 1);
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

    /// ADR-0075 decision 3: cutting the list to the cap is NOT an incompleteness of the
    /// search. It is counted (for the on-screen line) and never signed. Before ADR-0075 this
    /// set `incomplete` on 92% of registrations, so the signed flag said nothing.
    #[test]
    fn truncation_is_counted_not_signed_as_incomplete() {
        let bounded = bound_to(&list_of(8, None), 5);
        assert!(
            !bounded.as_list().incomplete,
            "truncation is the prompt's normal state"
        );
        assert_eq!(bounded.as_list().incomplete_reason, None);
        assert_eq!(bounded.withheld(), 3);
    }

    /// THE ONE THAT MATTERS, restated for two fields: the node could not READ two charts the
    /// search matched, and this prompt could not SHOW three it returned. The first is the
    /// serious one — it is what makes a zero untrustworthy — so it must survive truncation
    /// untouched: same flag, same words, nothing appended.
    #[test]
    fn a_partial_search_that_also_truncates_keeps_the_nodes_word() {
        let node_said = "2 charts could not be read";
        let bounded = bound_to(&list_of(8, Some(node_said)), 5);
        assert!(bounded.as_list().incomplete);
        assert_eq!(
            bounded.as_list().incomplete_reason.as_deref(),
            Some(node_said)
        );
        assert_eq!(bounded.withheld(), 3);
    }

    #[test]
    fn a_bare_partiality_flag_from_the_node_keeps_its_own_attribution() {
        // `incomplete: true` with no prose is a legal shape; it must still reach the clerk as
        // a sentence attributed to the node, cut or not.
        let bounded = bound_to(&list_with(8, true, None), 5);
        assert!(bounded.as_list().incomplete);
        let reason = bounded
            .as_list()
            .incomplete_reason
            .clone()
            .expect("a bare flag must still produce a sentence");
        assert!(
            reason.contains("not exhaustive"),
            "the node's partiality must still be attributed: {reason}"
        );
    }

    #[test]
    fn a_partial_search_that_fits_inside_the_cap_stays_partial() {
        // `incomplete` is the node's to set; bounding must never launder it away.
        let bounded = bound_for_prompt(&list_of(2, Some("1 chart could not be read")));
        assert!(bounded.as_list().incomplete);
        assert_eq!(
            bounded.as_list().incomplete_reason.as_deref(),
            Some("1 chart could not be read"),
            "nothing to add, so nothing should be added"
        );
        assert_eq!(bounded.withheld(), 0);
    }

    #[test]
    fn a_cap_of_zero_shows_nobody_and_counts_everyone_withheld() {
        // Kept total rather than panicking. It must never look like "found nothing": the
        // count is what separates a search that matched nobody from a prompt that showed
        // nobody.
        let bounded = bound_to(&list_of(4, None), 0);
        assert!(bounded.as_list().candidates.is_empty());
        assert_eq!(bounded.withheld(), 4);
        assert!(!bounded.as_list().incomplete);
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
        assert_eq!(bounded.withheld(), 0);
    }
}
