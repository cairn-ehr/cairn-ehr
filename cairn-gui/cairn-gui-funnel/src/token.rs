//! The pairing of a query with the list it produced — the only thing a registration may
//! attest to.
//!
//! # The failure this exists to make impossible
//!
//! A registration is an act that carries the search which preceded it
//! ([ADR-0061](../../../docs/spec/decisions/0061-registration-is-an-act-that-carries-its-search.md)):
//! the chart's birth act permanently records *what was searched for* and *which existing
//! charts were shown to the clerk before they decided none of them fitted*. That record is
//! forensic. It is what a later reviewer reads to answer "should this duplicate have been
//! caught?".
//!
//! So the dangerous bug is not a crash. It is a registration that swears to a search which
//! never ran, or which ran for a different person. `cairn_patient_search::SearchAttestation`
//! already closes half of this one layer down: it can only be *derived from* a displayed
//! list, never constructed independently. This module closes the other half, at the boundary
//! where a user interface could otherwise supply a query and a list that were never together.
//!
//! # How it is closed
//!
//! [`AttestedSearch`] **has no public constructor.** The only way to obtain one is to put a
//! search into a [`TokenStore`] and take it back out by the [`SearchToken`] that search
//! minted. A frontend therefore holds an opaque handle it can only echo; it cannot assemble
//! an attestation, and a bug in it fails to register rather than signing a false one.
//!
//! The design stated this as *"`register` takes only that token"*. Handing the write port an
//! `AttestedSearch` instead is the same guarantee moved into the type system: it holds
//! against a Rust caller too, not merely against JavaScript, and it needs no lookup to be
//! correct.
//!
//! # Why the attested search is the COMMIT-TIME one
//!
//! The browse search at step 1 cannot supply this pair — the clerk types fragments, edits
//! freely, and scrolls — so its query and its displayed set are a moving target. The step-3
//! search runs once, over the finished registration data, which is what makes query/displayed
//! drift structurally impossible: the attested search runs on *whatever the clerk finally
//! typed*, so the two cannot disagree. That is also why the browse list is free to scroll and
//! this one is not ([`crate::prompt`]).
//!
//! # Why a `u64` counter and not a UUID
//!
//! The token never crosses a trust boundary: it goes to the window's own webview and comes
//! back. A monotonic counter is unforgeable enough for that, adds no dependency and no source
//! of randomness, and a window session will not approach JavaScript's 2^53 integer limit —
//! one token per registration search, in a process a clerk restarts daily. If JavaScript ever
//! did round one, [`TokenStore::take`] returns [`TokenError::Mismatched`] and the registration
//! is refused. The failure is closed, which is the property that matters.

use cairn_patient_search::{CandidateList, SearchQuery};
use serde::{Deserialize, Serialize};

/// An opaque handle to one search.
///
/// Crosses to the webview as a plain number and comes back; the webview can only ever echo
/// it. Nothing about the search is recoverable from it — that is the point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchToken(u64);

/// A `(query, displayed list)` pair that a registration may attest to.
///
/// **There is no public constructor**, and that absence is the guarantee — see the module
/// doc. Obtain one only from [`TokenStore::take`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttestedSearch {
    token: SearchToken,
    query: SearchQuery,
    displayed: CandidateList,
}

impl AttestedSearch {
    /// The handle this search was filed under.
    pub fn token(&self) -> SearchToken {
        self.token
    }

    /// What was searched for.
    pub fn query(&self) -> &SearchQuery {
        &self.query
    }

    /// The candidates that were on the screen, in display order. Feeds
    /// `SearchAttestation::from_displayed`, which is the one definition of what a
    /// registration swears to.
    pub fn displayed(&self) -> &CandidateList {
        &self.displayed
    }
}

/// Why a token could not be minted, or could not be redeemed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenError {
    /// Nothing to search on. `db/045` refuses a registration whose attested query is empty —
    /// *"I searched for nothing and found nothing"* is not a search — so refusing it here is
    /// the same cheap pre-check `main.rs` makes before unsealing a key and ticking an HLC.
    EmptyQuery,
    /// Nothing is held. The form was edited, or this token was already redeemed.
    Absent,
    /// Something is held, but it is not the search this token names — a newer search has
    /// superseded it.
    Mismatched,
}

impl std::fmt::Display for TokenError {
    /// The text a clerk sees. Each says what to DO, because a refusal a clerk cannot act on
    /// is a dead end (§9.6 — an in-DB floor refusal is legible on purpose, and a refusal one
    /// layer above it should be no worse).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TokenError::EmptyQuery => write!(
                f,
                "nothing to search on — a registration records the search that preceded it \
                 (§5.8), so type at least a name, a date of birth or an identifier"
            ),
            TokenError::Absent => write!(
                f,
                "the search that licensed this registration is no longer held — it was \
                 superseded or already used. Let the search run again before registering"
            ),
            TokenError::Mismatched => write!(
                f,
                "a newer search has replaced the one this registration was about to attest \
                 to. Answer the current prompt instead"
            ),
        }
    }
}

impl std::error::Error for TokenError {}

/// Custody of the one search a registration may currently attest to.
///
/// Holds **at most one**, because the window has one registration form. A store that held
/// several would need a policy for which one a token-less caller gets, and the only safe
/// policy is "none" — so it holds one and answers by token.
#[derive(Debug, Default)]
pub struct TokenStore {
    held: Option<AttestedSearch>,
    /// Never reused within a process, so a token from a superseded search can be told apart
    /// from the current one rather than silently matching it.
    next: u64,
}

impl TokenStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record the search that just ran, replacing any earlier one, and mint its token.
    ///
    /// Replacing is deliberate: the previous search described a form that no longer exists,
    /// so keeping it would only make it possible to attest to the wrong one.
    ///
    /// **This does not consult [`crate::trigger`]**, and must not start. A form the trigger
    /// calls `Waiting` — a mononymous patient, or one whose date of birth is genuinely
    /// unknown — still gets a token, attesting the search that was actually possible. See
    /// the trigger module's doc for why gating here would be a principle-4 violation.
    pub fn record(
        &mut self,
        query: SearchQuery,
        displayed: CandidateList,
    ) -> Result<SearchToken, TokenError> {
        if query.is_empty() {
            return Err(TokenError::EmptyQuery);
        }
        let token = SearchToken(self.next);
        self.next += 1;
        self.held = Some(AttestedSearch {
            token,
            query,
            displayed,
        });
        Ok(token)
    }

    /// Take the attested search a token names, **removing it**.
    ///
    /// Removing is what stops a double-submit: two clicks on Register must not produce two
    /// charts off one attested search, so the second click finds nothing to attest and is
    /// refused. A registration that then fails puts it back with [`TokenStore::restore`].
    pub fn take(&mut self, token: SearchToken) -> Result<AttestedSearch, TokenError> {
        match self.held.as_ref().map(|held| held.token) {
            None => Err(TokenError::Absent),
            // Compare FIRST, remove only on a match. Taking whatever is held and checking
            // afterwards would discard the current search on a stale token — turning a
            // harmless mis-click into "search again".
            Some(held) if held != token => Err(TokenError::Mismatched),
            Some(_) => Ok(self.held.take().expect("just matched")),
        }
    }

    /// Put a taken search back, keeping its token, after a registration that failed.
    ///
    /// The design's *"Register fails. The form keeps its values."* — a clerk must not be made
    /// to re-search because the database hiccuped. Its token is unchanged, so the form's held
    /// handle stays valid.
    pub fn restore(&mut self, attested: AttestedSearch) {
        self.held = Some(attested);
    }

    /// The form was edited: whatever is held no longer describes it.
    ///
    /// Not an optimisation. A search for `Jon` must not license a registration of `John`, and
    /// discarding is how that stops being possible rather than being merely unlikely.
    pub fn discard(&mut self) {
        self.held = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_patient_search::{Candidate, TrustState};
    use uuid::Uuid;

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

    fn list_of(n: u128) -> CandidateList {
        CandidateList {
            candidates: (1..=n).map(candidate).collect(),
            incomplete: false,
            incomplete_reason: None,
        }
    }

    fn query(name: &str) -> SearchQuery {
        SearchQuery::new(name, Some("1984-03-02"), &[])
    }

    #[test]
    fn a_recorded_search_can_be_taken_back_by_its_token() {
        let mut store = TokenStore::new();
        let token = store
            .record(query("Samantha Michaelowski"), list_of(2))
            .unwrap();
        let attested = store
            .take(token)
            .expect("the search that was just recorded");
        assert_eq!(attested.token(), token);
    }

    #[test]
    fn the_attested_pair_is_exactly_what_was_recorded() {
        // Feeds `SearchAttestation::from_displayed`, which is the ONE definition of what a
        // registration swears to. If this pair can drift from what was recorded, that
        // crate's whole guarantee is defeated one layer up.
        let mut store = TokenStore::new();
        let q = query("Samantha Michaelowski");
        let list = list_of(3);
        let token = store.record(q.clone(), list.clone()).unwrap();
        let attested = store.take(token).unwrap();
        assert_eq!(attested.query(), &q);
        assert_eq!(attested.displayed(), &list);
    }

    #[test]
    fn taking_a_token_twice_fails_the_second_time() {
        // Double-submit protection. Two clicks on Register must not produce two charts off
        // one attested search — which would be a duplicate chart created BY the mechanism
        // that exists to prevent duplicate charts.
        let mut store = TokenStore::new();
        let token = store
            .record(query("Samantha Michaelowski"), list_of(1))
            .unwrap();
        assert!(store.take(token).is_ok());
        assert_eq!(store.take(token), Err(TokenError::Absent));
    }

    #[test]
    fn a_stale_token_is_refused_after_the_form_was_edited() {
        // Editing the form invalidates the search that described the OLD form. Registering
        // on it would attest a search for a different person — the `Jon` -> `John` case.
        let mut store = TokenStore::new();
        let token = store.record(query("Jon Smith"), list_of(1)).unwrap();
        store.discard();
        assert_eq!(store.take(token), Err(TokenError::Absent));
    }

    #[test]
    fn a_token_from_a_superseded_search_is_refused_rather_than_silently_redeemed() {
        // THE DANGEROUS BUG: returning whatever is held regardless of which token was
        // presented. That would let a registration attest search B while the form and the
        // clerk's memory are both about search A.
        let mut store = TokenStore::new();
        let first = store.record(query("Jon Smith"), list_of(1)).unwrap();
        let second = store.record(query("John Smith"), list_of(4)).unwrap();
        assert_ne!(first, second, "a superseding search needs its own token");
        assert_eq!(store.take(first), Err(TokenError::Mismatched));
        // And the current search survives the stale attempt — a mis-click must not cost the
        // clerk the search they are actually looking at.
        assert!(store.take(second).is_ok());
    }

    #[test]
    fn a_restored_search_keeps_its_token_so_a_retry_after_a_failure_works() {
        // The design's "Register fails. The form keeps its values." A clerk must not be
        // made to re-search because the database hiccuped.
        let mut store = TokenStore::new();
        let token = store
            .record(query("Samantha Michaelowski"), list_of(2))
            .unwrap();
        let attested = store.take(token).unwrap();
        store.restore(attested);
        let again = store.take(token).expect("the retry must find its search");
        assert_eq!(again.token(), token);
    }

    #[test]
    fn an_empty_query_is_refused_a_token() {
        // db/045 refuses a term-less attested search. Refusing here is the same cheap
        // pre-check main.rs makes before unsealing a key and ticking an HLC — the refusal
        // arrives before any of that, not from inside a transaction.
        let mut store = TokenStore::new();
        let empty = SearchQuery::new("   ", None, &[]);
        assert!(empty.is_empty(), "fixture must actually be empty");
        assert_eq!(store.record(empty, list_of(0)), Err(TokenError::EmptyQuery));
    }

    #[test]
    fn a_mononymous_form_with_no_birth_date_still_gets_a_token() {
        // THE TRIGGER IS NOT A GATE. `trigger_state` says Waiting for this form (see
        // `trigger::tests::a_mononymous_patient_with_no_birth_date_never_becomes_ready`);
        // `record` mints a token for it anyway. Principle 4 — a patient with one name and
        // an unknown date of birth must still be registrable, attesting the search that was
        // actually possible.
        let mut store = TokenStore::new();
        let q = SearchQuery::new("Amina", None, &[]);
        assert!(!q.is_empty(), "one name IS something to search on");
        assert!(
            store.record(q, list_of(0)).is_ok(),
            "the trigger must never be able to refuse a registration"
        );
    }

    #[test]
    fn an_identifier_alone_gets_a_token_though_no_name_was_typed() {
        // `SearchQuery::is_empty` already says an identifier alone is a real search. A store
        // that required a name would silently make MRN-only registration impossible.
        let mut store = TokenStore::new();
        let q = SearchQuery::new("", None, &[("MRN".into(), "12345".into())]);
        assert!(store.record(q, list_of(0)).is_ok());
    }

    #[test]
    fn a_search_that_found_nobody_is_still_attestable() {
        // The commonest registration by far: the clerk searched, nothing came back, and the
        // chart is new. An empty DISPLAYED list is a real and important attestation — "I
        // looked and there was nobody" — and must not be confused with an empty QUERY.
        let mut store = TokenStore::new();
        let token = store.record(query("Nobody Atall"), list_of(0)).unwrap();
        let attested = store.take(token).unwrap();
        assert!(attested.displayed().candidates.is_empty());
        assert!(!attested.query().is_empty());
    }

    #[test]
    fn taking_from_an_empty_store_is_absent_not_a_panic() {
        let mut store = TokenStore::new();
        // A token that was never minted here. Constructed through `record` on a throwaway
        // store rather than by naming the private field, so the test cannot forge one in a
        // way real code could not.
        let mut other = TokenStore::new();
        let foreign = other.record(query("Some One"), list_of(0)).unwrap();
        assert_eq!(store.take(foreign), Err(TokenError::Absent));
    }

    #[test]
    fn a_token_serializes_as_a_number_javascript_can_carry_exactly() {
        // The one place in this slice where a value leaves Rust's type system and comes
        // back. JSON numbers are f64 in a webview, so a token beyond 2^53 would round and
        // read back as a different token — which fails CLOSED (`Mismatched`), but would
        // still cost a clerk their search. Pinned here so the claim in the module doc is
        // checked rather than asserted.
        let mut store = TokenStore::new();
        let token = store.record(query("Some One"), list_of(0)).unwrap();
        let json = serde_json::to_string(&token).unwrap();
        let n: u64 = serde_json::from_str(&json).unwrap();
        assert!(
            n < (1u64 << 53),
            "a token must stay inside JavaScript's exact integer range: {n}"
        );
        assert_eq!(serde_json::from_str::<SearchToken>(&json).unwrap(), token);
    }

    #[test]
    fn every_refusal_says_what_to_do_about_it() {
        // §9.6 applied one layer above the floor: a refusal a clerk cannot act on is a dead
        // end. Each message names the remedy, not just the fault.
        for err in [
            TokenError::EmptyQuery,
            TokenError::Absent,
            TokenError::Mismatched,
        ] {
            let text = err.to_string();
            assert!(text.len() > 40, "too terse to act on: {text}");
        }
    }
}
