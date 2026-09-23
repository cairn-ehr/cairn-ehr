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
//! *states* the discipline one layer down — `from_displayed` is the one definition of what a
//! registration attests to — but it does not enforce it: its three fields are `pub` and it
//! derives `Deserialize`, so a determined caller can assemble one field by field
//! ([#355](https://github.com/cairn-ehr/cairn-ehr/issues/355) tracks making it
//! constructor-only; this module deliberately does not rely on it). The enforcement lives
//! here, at the
//! boundary where a user interface would otherwise supply a query and a list that were never
//! together, and it is enforcement rather than convention because [`AttestedSearch`]'s fields
//! are private and it has no public constructor.
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
//! search is different: it runs unasked over the completed registration data, and the pair a
//! registration attests to is the one from the run that *preceded the commit*. That is what
//! makes query/displayed drift structurally impossible — the attested search ran on whatever
//! the clerk finally typed, so the two cannot disagree — and it is why the browse list is
//! free to scroll and this one is not ([`crate::prompt`]).
//!
//! **Not "runs once".** It re-runs in the background as the clerk edits, which is precisely
//! why [`TokenStore::record`] replaces what it holds, why [`TokenStore::discard`] exists, and
//! why [`TokenStore::restore`] has to ask which generation it is putting a search back onto.
//!
//! # Why a `u64` counter and not a UUID
//!
//! The token goes to the window's own webview and comes back, and nothing about the search is
//! recoverable from it. A monotonic counter adds no dependency and no source of randomness,
//! and a session will not approach JavaScript's 2^53 integer limit — one token per
//! registration search, in a process a clerk restarts daily. If JavaScript ever did round one,
//! [`TokenStore::take`] returns [`TokenError::Mismatched`] and the registration is refused.
//! The failure is closed, which is the property that matters.
//!
//! **What makes this safe is a precondition, not an absence of a trust boundary.** Nothing
//! here depends on a token being unguessable — the guess space is tiny — so the property being
//! relied on is that each store is reached only by the window it belongs to, and that no two
//! stores mint the same token. The second half is why the counter is process-global (see
//! `NEXT_TOKEN`); what a guessed token buys is bounded anyway: redeeming a search that
//! really did run, in this process, for this form.

use crate::prompt::PromptList;
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
///
/// **Deliberately NOT `Clone`.** A caller who could keep a copy past a
/// [`TokenStore::take`] would have an attestation the store no longer knows about, and could
/// register with it twice — defeating the single-use property that stops two clicks on
/// Register producing two charts. If a future caller seems to need a clone, it almost
/// certainly wants a second `record` instead.
///
/// Because a borrow is as good as a copy for that purpose,
/// [`crate::port`-shaped](TokenStore::take) write ports take this **by value**: see
/// `cairn_gui_data::port::PatientRegistration::register`, which consumes it and hands it
/// back inside its error so the only way to retry is through [`TokenStore::restore`].
#[derive(PartialEq, Eq)]
pub struct AttestedSearch {
    token: SearchToken,
    /// The store's generation at the moment this was taken. [`TokenStore::restore`] puts it
    /// back only if the store is still on that generation — which is what distinguishes
    /// "nothing is held because I just took this" from "nothing is held because the clerk
    /// edited the form". See `restore`'s doc for the failure that distinction prevents.
    generation: u64,
    query: SearchQuery,
    displayed: PromptList,
}

/// Hand-written and **redacting on purpose**.
///
/// A derived `Debug` prints the query — the patient's name, date of birth and identifiers —
/// and every displayed candidate's name, age and UUID. A single `log::debug!("{state:?}")`
/// or a panicking `unwrap` in a command handler would then write a candidate list into a log
/// file, which is a disclosure ADR-0006/§11.8 does not permit of clinical content, and it is
/// a live `rust/cleartext-logging` sink shape besides.
///
/// The token, the generation and the candidate *count* are what custody bugs are debugged
/// with; the content is reachable only through the accessors, deliberately.
impl std::fmt::Debug for AttestedSearch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AttestedSearch")
            .field("token", &self.token)
            .field("generation", &self.generation)
            .field("query", &"<redacted>")
            .field(
                "displayed",
                &format_args!(
                    "<{} candidate(s), redacted>",
                    self.displayed.as_list().candidates.len()
                ),
            )
            .finish()
    }
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
    ///
    /// A `&CandidateList` rather than a `&PromptList` because that is what
    /// `from_displayed` takes, and because by this point the bounding has already happened:
    /// the only way this list got here was through [`crate::prompt::bound_for_prompt`].
    pub fn displayed(&self) -> &CandidateList {
        self.displayed.as_list()
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
    /// A registration taken from this store has not settled yet.
    ///
    /// Two clicks on Register must not produce two charts, and removing the held search is
    /// not sufficient on its own: the step-3 search runs in the background as the clerk
    /// edits, so a re-search can land between the clicks and leave the second one a fresh,
    /// redeemable token to spend. This refusal is what makes "at most one registration in
    /// flight" a property of the store rather than a hope about the button's disabled state.
    RegistrationInFlight,
}

impl std::fmt::Display for TokenError {
    /// The text a clerk sees. Each says what to DO, because a refusal a clerk cannot act on
    /// is a dead end (§9.8 — a floor refusal fails closed *legibly* on purpose, and a refusal
    /// one layer above it should be no worse).
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
            TokenError::RegistrationInFlight => write!(
                f,
                "this registration is already being saved — wait for it to finish rather \
                 than registering again, or the same patient gets two charts"
            ),
        }
    }
}

impl std::error::Error for TokenError {}

/// The source of every [`SearchToken`] in this process.
///
/// **Process-global, not per-store, and that is load-bearing.** A counter living in the
/// store would restart at zero for every `TokenStore`, so two windows would both mint token
/// `0`; an attestation taken from one store could then be installed into the other by
/// [`TokenStore::restore`] and redeemed by *its* form's own token `0` — registering patient
/// B while attesting the search that was run for patient A. With one counter per process
/// that cross-store confusion cannot be expressed: a foreign token simply never matches, so
/// it fails closed as [`TokenError::Mismatched`].
///
/// `Relaxed` is sufficient: nothing orders other memory against this, and the only property
/// required is that no two `fetch_add`s return the same value.
static NEXT_TOKEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// What [`TokenStore::restore`] did with the attestation it was handed.
///
/// `#[must_use]` because the two outcomes need *different words on screen* — "your
/// registration failed, press Register to try again" versus "a newer search has replaced
/// yours, answer the prompt on screen" — and a caller that ignores the distinction will
/// tell the clerk the wrong one. `restore` consumes the attestation either way: handing it
/// back on rejection would re-open the double-registration hole that not being `Clone`
/// exists to close.
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Restored {
    /// Put back under its original token. The form's handle is valid and a retry will work.
    Kept,
    /// Dropped, because the form it described no longer exists — a newer search landed, or
    /// the clerk edited the form while this registration was in flight.
    SupersededAndDropped,
}

/// Custody of the one search a registration may currently attest to.
///
/// Holds **at most one**, because the window has one registration form. A store that held
/// several would need a policy for which one a token-less caller gets, and the only safe
/// policy is "none" — so it holds one and answers by token.
///
/// # The two counters, and why `Option::is_none()` was not enough
///
/// `held.is_none()` is true in two states this store must tell apart: *"I just took the
/// search for a registration in flight"* and *"the clerk invalidated it by editing the
/// form"*. Restoring is right in the first and catastrophic in the second — it is the
/// `Jon` → `John` resurrection [`TokenStore::discard`] promises to make impossible. So
/// invalidation is counted, not inferred, and a taken attestation remembers the generation
/// it belongs to.
#[derive(Debug, Default)]
pub struct TokenStore {
    held: Option<AttestedSearch>,
    /// Bumped by **every** act that invalidates what was held — a new `record`, a refused
    /// empty-query `record`, `discard`, and a `commit` (a registration that succeeded consumed
    /// the form). A taken attestation may only be restored while the store is still on the
    /// generation it was taken from.
    generation: u64,
    /// Set by [`TokenStore::take`], cleared by [`TokenStore::restore`] or
    /// [`TokenStore::commit`] — and so by [`TokenStore::settle`], which is the end callers
    /// should actually reach for and which delegates to exactly those two.
    /// See [`TokenError::RegistrationInFlight`].
    in_flight: bool,
}

impl TokenStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drop whatever is held and move to a new generation.
    ///
    /// The single place invalidation happens, so that no path can forget to count it — the
    /// bug that let a refused `record` leave a stale search redeemable.
    fn invalidate(&mut self) {
        self.held = None;
        self.generation += 1;
    }

    /// Record the search that just ran, replacing any earlier one, and mint its token.
    ///
    /// Takes a [`PromptList`], not a raw `CandidateList`: the list a registration attests to
    /// must be the one that was actually on screen, so the bounding is not something a caller
    /// can skip. See [`crate::prompt::PromptList`].
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
        displayed: PromptList,
    ) -> Result<SearchToken, TokenError> {
        // Invalidate FIRST, before the empty-query refusal can return. Whatever was held
        // describes a form that no longer exists the moment a new search runs over it, and
        // that is just as true when the new form turns out to be unsearchable: a clerk who
        // cleared the form has abandoned the search, so leaving it redeemable would let the
        // NEXT patient's chart attest the PREVIOUS patient's search. An unsearchable form
        // is an invalidation, not a no-op.
        self.invalidate();
        if query.is_empty() {
            return Err(TokenError::EmptyQuery);
        }
        let token = SearchToken(NEXT_TOKEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
        self.held = Some(AttestedSearch {
            token,
            generation: self.generation,
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
        // Refuse before looking at what is held. A registration already in flight means the
        // clerk's second click must be refused even though a background re-search may have
        // left a perfectly valid NEWER token for it to spend — which is precisely how two
        // clicks produced two charts.
        if self.in_flight {
            return Err(TokenError::RegistrationInFlight);
        }
        match self.held.as_ref().map(|held| held.token) {
            None => Err(TokenError::Absent),
            // Compare FIRST, remove only on a match. Taking whatever is held and checking
            // afterwards would discard the current search on a stale token — turning a
            // harmless mis-click into "search again".
            Some(held) if held != token => Err(TokenError::Mismatched),
            Some(_) => {
                let attested = self.held.take().expect("just matched");
                self.in_flight = true;
                Ok(attested)
            }
        }
    }

    /// Put a taken search back, keeping its token, after a registration that failed.
    ///
    /// The design's *"Register fails. The form keeps its values."* — a clerk must not be made
    /// to re-search because the database hiccuped. Its token is unchanged, so the form's held
    /// handle stays valid.
    ///
    /// It settles the in-flight registration either way, so the clerk's next click is not
    /// refused as [`TokenError::RegistrationInFlight`] forever.
    ///
    /// **Restores only onto the generation it was taken from**, and that guard is not
    /// cosmetic — there are two distinct ways the form can move on underneath a registration
    /// in flight, and both must reject the restore:
    ///
    /// - **A newer search landed.** The step-3 search runs in the background as the clerk
    ///   edits, so a re-search can complete mid-flight. Putting the older one back OVER it
    ///   would resurrect a search describing a form that no longer exists, and the form's own
    ///   (newer) token would then be refused as `Mismatched` — telling the clerk a newer
    ///   search had replaced theirs when in fact an older one had.
    /// - **The clerk edited the form.** [`TokenStore::discard`] leaves nothing held, so a
    ///   guard that asked only `held.is_none()` would cheerfully restore — resurrecting the
    ///   very search `discard` exists to destroy. That is the `Jon` → `John` case: register,
    ///   correct the typo mid-flight, the write fails, and the chart for `John` is born
    ///   permanently attesting a search for `Jon`.
    ///
    /// Fail-closed in both; the generation check is what also makes it truthful.
    pub fn restore(&mut self, attested: AttestedSearch) -> Restored {
        self.in_flight = false;
        if self.held.is_some() || attested.generation != self.generation {
            return Restored::SupersededAndDropped;
        }
        self.held = Some(attested);
        Restored::Kept
    }

    /// The registration succeeded: settle it, so the next one is not refused.
    ///
    /// **A caller that forgets this leaves the store refusing every later `take`.** That is
    /// deliberate — it fails closed, costing a window reload rather than minting a duplicate
    /// chart — but it does mean [`TokenStore::commit`] and [`TokenStore::restore`] are the
    /// two mandatory ends of every [`TokenStore::take`]. There is no third.
    ///
    /// # A success INVALIDATES, and that is the load-bearing half
    ///
    /// Clearing `in_flight` alone was not enough, and the gap was a duplicate chart. `record`
    /// has no `in_flight` guard — deliberately, because the step-3 search re-runs in the
    /// background as the clerk types, so a fresh token can be minted *while* a registration is
    /// in flight. Clearing only the latch left that token redeemable after the write had
    /// already succeeded, so a second click minted a second chart for the patient just
    /// registered — the exact harm [`TokenError::RegistrationInFlight`] claims to make a
    /// property of the store rather than a hope about a disabled button.
    ///
    /// So a success counts a generation: **the form has been consumed, and nothing recorded
    /// against a consumed form may be redeemed.** A window that wants to register again
    /// searches again — the same gesture the paper counterpart forces, and the same rule
    /// [`TokenStore::discard`] already applied to an edit. Pinned by
    /// `a_success_consumes_the_form_so_a_mid_flight_search_cannot_mint_a_second_chart`.
    /// (PR #661 review, found independently by three reviewers.)
    ///
    /// **The trade, stated rather than left to be discovered.** Neither this method nor
    /// [`TokenStore::settle`]'s `Ok` arm can prove a [`TokenStore::take`] was outstanding — the
    /// success arm carries no attestation, because a successful port call consumed it. So a
    /// caller that commits without having taken now destroys a held search as well as clearing a
    /// latch that was never set. That is misuse either way, and the important part is the
    /// DIRECTION it fails in: the next `take` answers [`TokenError::Absent`], whose remedy is
    /// *"let the search run again"*. It cannot mint a chart. Losing a search costs a clerk one
    /// gesture; the behaviour this replaced cost a patient a duplicate chart.
    pub fn commit(&mut self) {
        self.in_flight = false;
        self.invalidate();
    }

    /// Settle a registration's outcome: **the end of a [`TokenStore::take`] a caller reaches by
    /// writing the SHORT thing, rather than the one they reach by remembering to.**
    ///
    /// # The trap this closes
    ///
    /// [`TokenStore::take`] has exactly two valid ends — [`TokenStore::commit`] and
    /// [`TokenStore::restore`] — and the port hands the attested search back *inside* its error
    /// precisely so that `restore` has the value it needs. That made the natural Rust idiom a
    /// silent trap:
    ///
    /// ```ignore
    /// let id = live.register(attested, name).await.map_err(|(e, _)| e)?;  // ← latches the store
    /// ```
    ///
    /// It compiles with no warning. It discards the attestation during *destructuring*, so
    /// `#[must_use]` cannot catch it — that lint fires on an unused expression result, not on a
    /// field dropped in a pattern, and `Result` is `#[must_use]` already. And it leaves
    /// `in_flight` set, so every later `take` returns [`TokenError::RegistrationInFlight`].
    ///
    /// The clerk cannot get out of that state either. [`TokenStore::discard`] — what editing
    /// the form calls — bumps the generation and **deliberately does not clear `in_flight`**,
    /// because clearing it there is how two clicks once produced two charts. So the recovery
    /// gesture does not recover, and nothing short of rebuilding the store does.
    ///
    /// Routing the outcome through here makes the short path the correct one: the ergonomic
    /// call is `store.settle(port_result)`, and `map_err(|(e, _)| e)` becomes the *longer*
    /// thing to write. See [#659](https://github.com/cairn-ehr/cairn-ehr/issues/659).
    ///
    /// # Why the `Restored` is returned rather than swallowed
    ///
    /// The two failure outcomes need different words on screen. [`Restored::Kept`] means the
    /// form's token is still redeemable and the clerk may simply press Register again;
    /// [`Restored::SupersededAndDropped`] means a newer search landed or the clerk edited, so
    /// there is nothing to retry *with* and the window must wait for the next search before it
    /// offers Register at all. Collapsing them would leave a live Register button sitting over
    /// a search that no longer exists.
    ///
    /// # Generic on purpose
    ///
    /// `T` and `E`, not `Uuid` and `DataError`. `DataError` lives in `cairn-gui-data`, which
    /// already depends on this crate, so naming it here would invert that edge into a cycle and
    /// the tree would not build. It is also the honest signature: a token store has no business
    /// knowing what a failure *is*, only that one happened.
    pub fn settle<T, E>(
        &mut self,
        outcome: Result<T, (E, AttestedSearch)>,
    ) -> Result<T, (E, Restored)> {
        match outcome {
            Ok(value) => {
                self.commit();
                Ok(value)
            }
            // `restore` settles the in-flight registration whether or not it keeps the
            // attestation, which is what makes this arm total: there is no path through
            // `settle` that leaves the store latched.
            Err((error, attested)) => Err((error, self.restore(attested))),
        }
    }

    /// The form was edited: whatever is held no longer describes it.
    ///
    /// Not an optimisation. A search for `Jon` must not license a registration of `John`, and
    /// discarding is how that stops being possible rather than being merely unlikely — which
    /// is why it counts a new generation rather than merely clearing the slot, so that a
    /// registration failing mid-flight cannot put the discarded search back.
    pub fn discard(&mut self) {
        self.invalidate();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt::bound_for_prompt;
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

    /// A displayed list, bounded the only way `record` will accept.
    ///
    /// Note `n` may exceed `PROMPT_CAP`: the bounding is what keeps the attested list
    /// truthful, and these tests are about custody rather than about the bounding.
    fn list_of(n: u128) -> PromptList {
        bound_for_prompt(&CandidateList {
            candidates: (1..=n).map(candidate).collect(),
            incomplete: false,
            incomplete_reason: None,
        })
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
        assert_eq!(attested.displayed(), list.as_list());
    }

    #[test]
    fn taking_a_token_twice_fails_the_second_time() {
        // Double-submit protection. Two clicks on Register must not produce two charts off
        // one attested search — which would be a duplicate chart created BY the mechanism
        // that exists to prevent duplicate charts.
        //
        // While the first registration is still in flight the refusal is
        // `RegistrationInFlight`, which is the more useful of the two truths: it tells the
        // clerk to WAIT rather than to search again.
        let mut store = TokenStore::new();
        let token = store
            .record(query("Samantha Michaelowski"), list_of(1))
            .unwrap();
        assert!(store.take(token).is_ok());
        assert_eq!(store.take(token), Err(TokenError::RegistrationInFlight));
        // Once it has settled, the token is simply spent.
        store.commit();
        assert_eq!(store.take(token), Err(TokenError::Absent));
    }

    #[test]
    fn a_second_registration_cannot_be_taken_while_the_first_is_in_flight() {
        // THE DOUBLE-SUBMIT HOLE THAT REMOVING THE HELD SEARCH DOES NOT CLOSE. The step-3
        // search runs in the background as the clerk edits, so the clerk's last keystroke
        // can land a re-search BETWEEN the two clicks — handing click 2 a fresh, perfectly
        // valid token to spend while click 1's write is still in flight. Two charts, minted
        // by the machinery whose whole job is preventing duplicate charts.
        //
        // The debounce window and the between-clicks window are the same order of
        // magnitude, so this is not a theoretical race.
        let mut store = TokenStore::new();
        let first = store.record(query("Jon Smith"), list_of(1)).unwrap();
        let _in_flight = store.take(first).unwrap();
        let second = store.record(query("Jon Smith"), list_of(1)).unwrap();
        assert_eq!(
            store.take(second),
            Err(TokenError::RegistrationInFlight),
            "at most one registration may be in flight, whatever token is presented"
        );
    }

    #[test]
    fn a_settled_registration_frees_the_store_for_the_next_patient() {
        // The other half of the in-flight guard: it must not be a one-shot latch. A clerk
        // registers all day, and a store that refused every take after the first would be
        // fail-closed but useless.
        let mut store = TokenStore::new();
        let first = store.record(query("Jon Smith"), list_of(1)).unwrap();
        store.take(first).unwrap();
        store.commit();
        let second = store.record(query("Amina Hassan"), list_of(0)).unwrap();
        assert!(
            store.take(second).is_ok(),
            "a settled registration must not block the next one"
        );
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
        assert_eq!(store.restore(attested), Restored::Kept);
        let again = store.take(token).expect("the retry must find its search");
        assert_eq!(again.token(), token);
    }

    #[test]
    fn restoring_after_a_newer_search_landed_does_not_resurrect_the_older_one() {
        // The step-3 search runs in the background as the clerk edits, so a re-search can
        // complete while a registration is still in flight. If `restore` clobbered it, the
        // form's own newer token would be refused and the clerk told a NEWER search had
        // replaced theirs — when an older one had. Fail-closed either way; this is the half
        // that makes it truthful.
        let mut store = TokenStore::new();
        let first = store.record(query("Jon Smith"), list_of(1)).unwrap();
        let taken = store.take(first).unwrap();
        // …the background search lands while the registration is in flight…
        let second = store.record(query("John Smith"), list_of(2)).unwrap();
        // …and the registration then fails, putting its search back.
        assert_eq!(store.restore(taken), Restored::SupersededAndDropped);

        assert_eq!(
            store.take(first),
            Err(TokenError::Mismatched),
            "the stale search must not have been resurrected"
        );
        assert!(
            store.take(second).is_ok(),
            "the search the form is actually holding must survive"
        );
    }

    #[test]
    fn a_discarded_search_cannot_be_resurrected_by_a_failed_registration() {
        // THE `Jon` -> `John` HOLE, through the `discard` door. `discard`'s own doc says a
        // search for `Jon` must not license a registration of `John` — but a guard that
        // only asks `held.is_none()` cannot tell "nothing held because I just took it"
        // from "nothing held because the clerk edited the form".
        //
        // Reachable: click Register (take), correct the typo while the write is in flight
        // (discard), the write fails (restore), click Register again. Without the
        // generation check the store hands back the `Jon` search and the chart for `John`
        // is born permanently attesting it.
        let mut store = TokenStore::new();
        let token = store.record(query("Jon Smith"), list_of(1)).unwrap();
        let attested = store.take(token).unwrap();
        store.discard();
        assert_eq!(
            store.restore(attested),
            Restored::SupersededAndDropped,
            "a discarded search must be dropped, not put back"
        );
        assert_eq!(
            store.take(token),
            Err(TokenError::Absent),
            "a search the clerk invalidated by editing must stay invalidated"
        );
    }

    #[test]
    fn clearing_the_form_invalidates_the_search_that_was_held() {
        // `record`'s doc promises it replaces "any earlier one", because "keeping it would
        // only make it possible to attest to the wrong one". The empty-query refusal must
        // honour that promise rather than returning before `held` is touched: a clerk who
        // clears the form has abandoned the search, and leaving it redeemable lets the next
        // patient's chart attest the previous patient's search.
        let mut store = TokenStore::new();
        let token = store.record(query("Jon Smith"), list_of(1)).unwrap();
        assert_eq!(
            store.record(SearchQuery::new("   ", None, &[]), list_of(0)),
            Err(TokenError::EmptyQuery)
        );
        assert_eq!(
            store.take(token),
            Err(TokenError::Absent),
            "an unsearchable form is an invalidation, not a no-op"
        );
    }

    #[test]
    fn a_token_is_never_reissued_after_a_discard() {
        // The `next` counter's doc claims a token is "never reused". Non-reuse across a
        // `take` was pinned; across a `discard` it was not, so a counter reset there would
        // let token 0 from a discarded `Jon` search match token 0 of a later `John` search
        // and be silently redeemed.
        let mut store = TokenStore::new();
        let first = store.record(query("Jon Smith"), list_of(1)).unwrap();
        store.discard();
        let second = store.record(query("John Smith"), list_of(1)).unwrap();
        assert_ne!(first, second, "a discarded token must not be reissued");
        assert_eq!(store.take(first), Err(TokenError::Mismatched));
    }

    #[test]
    fn two_attested_searches_can_never_be_in_flight_at_once() {
        // The stronger form of "which of two restores wins?": with the in-flight guard the
        // question cannot arise, because a second attestation can never be taken while the
        // first is outstanding. Pinned so nobody removes the guard and reintroduces a race
        // whose resolution would then depend on restore ORDER — something no caller
        // controls.
        let mut store = TokenStore::new();
        let first = store.record(query("Jon Smith"), list_of(1)).unwrap();
        let _older = store.take(first).unwrap();
        let second = store.record(query("John Smith"), list_of(2)).unwrap();
        assert_eq!(store.take(second), Err(TokenError::RegistrationInFlight));
    }

    #[test]
    fn tokens_are_unique_across_stores_in_one_process() {
        // The `next` doc says "never reused within a PROCESS". A per-store counter makes
        // that false: two windows both mint token 0, and an attestation taken from one
        // store then installs into the other and is redeemed by its form's own token 0 —
        // registering patient B while attesting the search for patient A.
        let mut a = TokenStore::new();
        let mut b = TokenStore::new();
        let from_a = a.record(query("Jon Smith"), list_of(1)).unwrap();
        let from_b = b.record(query("John Smith"), list_of(1)).unwrap();
        assert_ne!(from_a, from_b, "two stores must not mint the same token");
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
        // back, so what this genuinely pins is the ROUND TRIP: a `SearchToken` serialises as
        // a bare JSON number and reads back as the same token.
        //
        // The `< 2^53` assertion is a sanity check on the shape, NOT evidence about session
        // lifetime — this token comes from a fresh counter, so it is asserting that a small
        // number is small. The lifetime claim in the module doc rests on the arithmetic
        // there (one token per registration search, u64), not on this test.
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
    fn every_refusal_names_its_own_remedy_and_no_two_read_alike() {
        // §9.8 applied one layer above the floor: a refusal a clerk cannot act on is a dead
        // end. This used to assert only `text.len() > 40`, which is a proxy for verbosity
        // rather than for naming a remedy — collapsing all four arms into one generic
        // "something went wrong, try again later" would have passed it, and a clerk hitting
        // `Mismatched` would have been told to retype a name.
        let cases = [
            (TokenError::EmptyQuery, "type at least"),
            (TokenError::Absent, "run again"),
            (TokenError::Mismatched, "current prompt"),
            (TokenError::RegistrationInFlight, "wait"),
        ];
        for (err, remedy) in cases {
            let text = err.to_string();
            assert!(
                text.contains(remedy),
                "{err:?} must name its remedy ({remedy:?}): {text}"
            );
        }
        // And they must be four different sentences, not one message wearing four names.
        let texts: std::collections::BTreeSet<String> =
            cases.iter().map(|(e, _)| e.to_string()).collect();
        assert_eq!(texts.len(), cases.len(), "two refusals read alike");
    }

    #[test]
    fn the_attested_search_type_keeps_the_shape_its_guarantee_depends_on() {
        // A STRUCTURAL GUARD, in the house style of `nothing_in_the_search_path_can_narrow_on_sex`
        // and `crypto_sink_names_are_genuine.rs`. The module doc's central claim is that
        // `AttestedSearch` has no public constructor and is not `Clone` — and nothing enforced
        // it: adding `#[derive(Clone)]` or a `pub fn new` compiles and breaks no test, while
        // silently re-opening "one attested search creates two charts".
        let src = include_str!("token.rs");
        let decl = src
            .split_once("pub struct AttestedSearch {")
            .expect("the struct must still be here")
            .0;
        let derives = decl
            .rsplit_once("#[derive(")
            .expect("a derive list above the struct")
            .1;
        let derives = &derives[..derives.find(')').expect("a closed derive list")];
        for forbidden in ["Clone", "Copy", "Deserialize", "Default"] {
            assert!(
                !derives.contains(forbidden),
                "AttestedSearch must not derive {forbidden} — it would let a caller keep or \
                 forge an attestation the store no longer knows about. Derives: {derives}"
            );
        }
        // No public constructor: every `pub fn` in the `impl AttestedSearch` block must be an
        // accessor, never something handing back a `Self`.
        let imp = src
            .split_once("impl AttestedSearch {")
            .expect("the impl block must still be here")
            .1;
        let imp = &imp[..imp.find("\n}").expect("a closed impl block")];
        for constructor in ["-> Self", "-> AttestedSearch"] {
            assert!(
                !imp.contains(constructor),
                "AttestedSearch must have no public constructor, found `{constructor}`: {imp}"
            );
        }
    }

    // --- #659: settle is the only end of a `take` a caller can reach by accident ---

    /// A success settles the store, so the NEXT registration is not refused.
    ///
    /// Without `settle`, the ergonomic `map_err(|(e, _)| e)?` leaves `in_flight` set and every
    /// later `take` returns `RegistrationInFlight` — for the rest of the window's life.
    #[test]
    fn settling_a_success_leaves_the_store_ready_for_the_next_registration() {
        let mut store = TokenStore::new();
        let first = store.record(query("Aabria Iyengar"), list_of(1)).unwrap();
        let attested = store.take(first).expect("the only token");
        // The port consumed the attestation and answered `Ok` — which is exactly why the
        // success arm has nothing to hand back, and exactly why forgetting to `commit` is so
        // easy to do.
        drop(attested);

        let settled: Result<u8, (&str, Restored)> =
            store.settle(Ok::<u8, (&str, AttestedSearch)>(7));
        assert_eq!(settled.ok(), Some(7));

        let second = store.record(query("Bilal Osei"), list_of(1)).unwrap();
        assert!(
            store.take(second).is_ok(),
            "a settled success must not latch the store: every later registration is refused \
             forever otherwise"
        );
    }

    /// A registration that SUCCEEDED consumes the form, so a search that landed mid-flight
    /// must not stay redeemable — otherwise one patient gets two charts.
    ///
    /// The walk, all through the public API and all of it ordinary:
    ///
    /// 1. the clerk clicks Register — `take` removes the held search and latches `in_flight`;
    /// 2. the debounced step-3 search from their LAST keystroke lands while the write is in
    ///    flight and calls `record`, which is legitimate and is why `record` has no
    ///    `in_flight` guard — it is the same interleaving
    ///    `restoring_after_a_newer_search_landed_does_not_resurrect_the_older_one` relies on;
    ///    the store now holds a fresh, redeemable token;
    /// 3. the write succeeds and the caller settles it.
    ///
    /// If step 3 only cleared the latch, a second click — a double-click, a doubled Enter, a
    /// success handler slow enough for the clerk to press again — would `take` the token from
    /// step 2 and mint a SECOND chart for the patient just registered. That is precisely the
    /// harm [`TokenError::RegistrationInFlight`]'s doc claims is a property "of the store
    /// rather than a hope about the button's disabled state"; before this test it was back to
    /// being a hope the moment the registration succeeded.
    ///
    /// So a success `invalidate`s: the form has been consumed, and nothing recorded against a
    /// consumed form may be redeemed. A window that wants to register again starts by
    /// searching again, which is the same gesture the paper counterpart forces (PR #661
    /// review, converged on independently by three reviewers).
    #[test]
    fn a_success_consumes_the_form_so_a_mid_flight_search_cannot_mint_a_second_chart() {
        let mut store = TokenStore::new();
        let first = store.record(query("Aabria Iyengar"), list_of(1)).unwrap();
        let attested = store.take(first).expect("the only token");

        // The background re-search lands WHILE the registration is in flight.
        let mid_flight = store
            .record(query("Aabria Iyengar"), list_of(1))
            .expect("a background search may land mid-flight; `record` has no in_flight guard");

        // The port consumed the attestation and answered `Ok`.
        drop(attested);
        let settled: Result<u8, (&str, Restored)> =
            store.settle(Ok::<u8, (&str, AttestedSearch)>(7));
        assert_eq!(settled.ok(), Some(7));

        assert!(
            matches!(store.take(mid_flight), Err(TokenError::Absent)),
            "a token recorded while the registration was in flight must NOT survive that \
             registration succeeding: redeeming it mints a second chart for the patient who \
             was just registered"
        );
    }

    /// A failure settles it too, AND puts the attested search back for the retry.
    ///
    /// The design's *"Register fails. The form keeps its values."* — a clerk must not be made
    /// to re-search because the database hiccuped.
    #[test]
    fn settling_a_failure_restores_the_search_and_keeps_its_token() {
        let mut store = TokenStore::new();
        let token = store.record(query("Chidi Anagonye"), list_of(2)).unwrap();
        let attested = store.take(token).expect("the only token");

        let settled: Result<u8, (&str, Restored)> =
            store.settle(Err(("the node was unreachable", attested)));
        let Err((message, restored)) = settled else {
            panic!("a failed registration must settle as a failure");
        };
        assert_eq!(message, "the node was unreachable");
        assert_eq!(restored, Restored::Kept);
        assert!(
            store.take(token).is_ok(),
            "the SAME token must still be redeemable, or the form's held handle is a lie"
        );
    }

    /// THE BUG #659 IS ACTUALLY ABOUT: the clerk edits while the registration is in flight.
    ///
    /// `discard` bumps the generation and deliberately leaves `in_flight` set, so the restore
    /// is correctly refused as superseded — but the store must still be SETTLED, or editing
    /// the form (the clerk's own recovery gesture) latches it shut and nothing short of
    /// rebuilding the `TokenStore` recovers.
    #[test]
    fn settling_a_failure_the_clerk_has_already_edited_past_still_unlatches_the_store() {
        let mut store = TokenStore::new();
        let token = store.record(query("Jon Mistyped"), list_of(1)).unwrap();
        let attested = store.take(token).expect("the only token");

        store.discard(); // the clerk corrects the spelling while the write is in flight

        let settled: Result<u8, (&str, Restored)> = store.settle(Err(("refused", attested)));
        let Err((_, restored)) = settled else {
            panic!("a failed registration must settle as a failure");
        };
        assert_eq!(
            restored,
            Restored::SupersededAndDropped,
            "the pre-edit search must NOT come back — that is how a chart is born attesting a \
             search for a different spelling of the name"
        );

        let fresh = store.record(query("John Corrected"), list_of(1)).unwrap();
        assert!(
            store.take(fresh).is_ok(),
            "the store must be usable again: editing the form is the clerk's recovery gesture, \
             and a gesture that latches the store breaks the very thing it is meant to fix"
        );
    }
}
