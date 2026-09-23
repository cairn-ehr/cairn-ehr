//! The ports the UI reaches the record through. Real impl → the node (today by direct
//! query, later its native-API client); mock impl → fixtures.
//!
//! Three traits, and the split between them is the contract:
//!
//! - [`ClinicalData`] — one patient's chart. **Read-only, and stays so.**
//! - [`PatientSearch`] — the §5.8 candidate search. Read-only and unsigned.
//! - [`PatientRegistration`] — the funnel's ONE write, which creates a chart and
//!   permanently records the search that licensed it (ADR-0061).
//!
//! The UI still signs nothing itself (§9.6): `register` hands an already-paired
//! `AttestedSearch` to an implementation that owns the key, and the in-DB floor is the real
//! enforcement either way (principle 12).
use cairn_gui_funnel::AttestedSearch;
use cairn_gui_tab::PatientRef;
use cairn_patient_search::{CandidateList, SearchQuery};

#[derive(Debug, Clone)]
pub struct Demographics {
    pub patient: PatientRef,
    pub sex: String,
    pub birth_date: String,
    /// (system, value), e.g. ("MRN", "12345").
    pub identifiers: Vec<(String, String)>,
}

/// A one-line cross-reference summary — the payload behind a "see X-ray report"
/// link that opens the target in the other pane (spec §5).
#[derive(Debug, Clone)]
pub struct NoteRef {
    pub id: String,
    pub one_line: String,
}

/// Why a port could not answer.
///
/// The three variants are three different clinical facts, and the split that matters is
/// [`DataError::Refused`] against [`DataError::Unavailable`] — the distinction
/// [#648](https://github.com/cairn-ehr/cairn-ehr/issues/648) asked for, landed here in slice
/// 2b now that a live implementation exists to produce it.
///
/// - `NotFound` — no such chart. A true, exhaustive answer.
/// - `Unavailable` — **nothing was decided.** A dropped connection, a lock timeout, a full
///   disk. The very same call may well succeed on a retry, so the window offers one.
/// - `Refused` — **something decided against this call and will decide the same way every
///   time.** Two layers can decide, and this variant covers both (#651): the **in-DB floor** (a
///   term-less attested query at `db/045`; a chart whose first event is not its registration at
///   `db/005` step 8b — #345 / ADR-0061), and a **`cairn-node` pre-flight check that refuses in
///   Rust before any statement reaches Postgres** — a malformed date of birth, which the floor
///   never sees. A retry cannot succeed, and offering one is a precise untruth on a
///   wrong-chart-prevention surface (principle 4). The payload is whichever layer decided: the
///   floor's own message, or the orchestrator's own sentence. Both are legible on purpose, and
///   the text is the one thing that tells the clerk what to change.
/// - `NotProvisioned` — **this node decided against the call, and the form was never the
///   problem.** Also a verdict, not an accident: retrying the identical call is pointless, so
///   this is emphatically not an `Unavailable`. But it is pointless *until an operator runs the
///   command the payload names*, at which point the same call succeeds — so a window must
///   withhold the retry-now that `Unavailable` earns, show the remedy, and keep a way to try
///   again once it is done. Rendering it as `Refused` strands a clerk whose form was correct;
///   rendering it as `Unavailable` hides the remedy behind a retry that will keep failing.
///   Carried by `cairn_node::db_diagnosis::RefusalScope::NodeState` (PR #661 review).
///
/// # What a refusal does NOT change: the attestation still goes back
///
/// An earlier draft of this doc reasoned that the variant would also decide whether the caller
/// calls `TokenStore::restore` — restore after an outage, pointless after a refusal.
/// **Building it showed that to be wrong, and the reason is the token store's shape.**
/// `restore` and `commit` are the two mandatory ends of every `take`; a caller that does
/// neither latches the store closed and costs a window reload. After a refusal `commit` would
/// be a lie (nothing was created), so `restore` is the only truthful end — and it is also the
/// right one, because the clerk's next act is to EDIT the form, and editing calls `discard`,
/// which destroys the doomed attestation on a new generation. A clerk who instead clicks
/// Register again meets the same legible refusal, which is honest. So both arms restore; only
/// the sentence on screen differs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataError {
    NotFound,
    Unavailable(String),
    /// A deterministic verdict — from the in-DB floor, or from a `cairn-node` pre-flight check
    /// that refuses before Postgres sees the call. See the enum doc: never retried, always
    /// legible, and the payload is whichever layer decided.
    Refused(String),
    /// A verdict about this NODE's provisioning state, not about the call. See the enum doc:
    /// never retried *as is*, but the payload names a command that makes the same call succeed.
    NotProvisioned(String),
}

pub trait ClinicalData {
    fn demographics(&self, patient_uuid: &str) -> Result<Demographics, DataError>;
    fn note_refs(&self, patient_uuid: &str) -> Result<Vec<NoteRef>, DataError>;

    /// One patient's medication chart — current drugs, ceased ones, each carrying the
    /// signature state of its member threads, AND what the node knows it cannot display.
    ///
    /// Returns the whole `PatientMedicationList` rather than just its rows on purpose.
    /// ADR-0060 decision 2 requires partial completion to be *reported, never implied*, and
    /// a port that hands over only rows makes obeying that impossible: the renderer cannot
    /// warn about a drug it was never given. The real implementation is
    /// `cairn_node::medication::read::list_patient_medications`; this port exists so the
    /// window can also run against fixtures with no database.
    fn medications(
        &self,
        patient_uuid: &str,
    ) -> Result<cairn_medication_view::PatientMedicationList, DataError>;
}

// ---------------------------------------------------------------------------------------
// The funnel's two ports (§5.3/§5.8 search-before-create).
//
// TWO TRAITS AND NOT ONE, deliberately: the write surface is then exactly one method wide,
// and `--mock` can exercise the whole browse workflow with no signing in it at all. A single
// trait carrying both would make "this window can search but must never write" unstateable.
//
// WHY `impl Future ... + Send` AND NOT `async fn`. A bare `async fn` in a public trait raises
// the `async_fn_in_trait` lint, which CI turns into an error, and the future has to be `Send`
// to be awaited inside a Tauri command. Spelling the bound is the fix that costs no
// `async-trait` dependency.
//
// The consequence, stated so 2b does not rediscover it: these traits are **not
// dyn-compatible**. The window dispatches mock-vs-live with the same `is_mock()` branch
// `commands.rs` already uses, not with a trait object. That is not a workaround — the window
// knows which mode it launched in, and a boolean that can disagree with reality is exactly
// how a "mock" window ends up writing to a real database (the reasoning `AppState::is_mock`
// already carries).
// ---------------------------------------------------------------------------------------

/// §5.8 candidate search — step 1 (browse) and step 3 (the registration search) both call it.
///
/// Read-only and unsigned. The candidate list it returns is advisory in the strict sense:
/// nothing it says can refuse a registration, only inform one.
pub trait PatientSearch {
    /// Run the search.
    ///
    /// `today` is the caller's clock as an ISO `YYYY-MM-DD` string, exactly as
    /// `cairn_node::patient::search::search_patients` takes it, and for the same reason: the
    /// age arithmetic shown beside a patient's name stays pure and the edge owns the clock.
    ///
    /// **The caller's value is the one used — this port never overrides it.** The node's own
    /// CLI obtains that value by asking the DATABASE for `current_date` rather than reading
    /// the operator's wall clock, and a live caller of this port should do the same; but that
    /// is a rule about where the caller *gets* the date, not licence for an implementation to
    /// substitute its own. An implementation that ignored the argument would make the age
    /// beside a patient's name depend on which clock won, with nothing on screen saying
    /// which.
    ///
    /// A failure must surface as `Err`, never as an empty list. "The search failed" and
    /// "nobody matched" are different answers, and only one of them is evidence of absence —
    /// which is precisely the distinction that decides whether a clerk creates a duplicate
    /// chart (principle 4).
    fn search(
        &self,
        query: &SearchQuery,
        today: &str,
    ) -> impl std::future::Future<Output = Result<CandidateList, DataError>> + Send;
}

/// Create a chart, attesting the search that licensed it (ADR-0061).
///
/// The one write the funnel performs, and the whole reason it is split from [`PatientSearch`].
pub trait PatientRegistration {
    /// Register a new patient and return the minted chart id.
    ///
    /// `attested` carries BOTH the query and the candidate list it produced, as one value
    /// that cannot be assembled from parts — see `cairn_gui_funnel::token`. An implementation
    /// therefore cannot be handed a query and a list that were never together, which is the
    /// failure the funnel exists to prevent.
    ///
    /// `name` is the **raw typed string the query was built from** — never a reassembled one.
    /// `cairn_node::patient::register::register_patient` states this requirement in its own
    /// doc and notes that nothing in its types can enforce it: a caller could attest a search
    /// for "Smith" while asserting the name "Jones" and it would sign both without complaint.
    /// The funnel satisfies it by construction, because the form has ONE name field and that
    /// one string feeds `SearchQuery::new` and this argument alike.
    ///
    /// `None`, or blank after trimming, means nothing was typed — an identifier-only
    /// registration. No name is then asserted, rather than an empty one (principle 4).
    ///
    /// # It CONSUMES the attestation, and hands it back on failure
    ///
    /// `AttestedSearch` is deliberately not `Clone` so that one attested search cannot
    /// create two charts. A *borrow* would have defeated that on its own — the caller keeps
    /// the original and can simply call `register` twice, no copy required — so this takes
    /// it by value, and the whole flow becomes linear:
    ///
    /// ```text
    /// record -> take -> register(by value) -> Ok: commit
    ///                                      -> Err: the search comes back -> restore
    /// ```
    ///
    /// Returning it inside the error is not decoration: `TokenStore::restore` needs exactly
    /// that value to put the search back after a failed write, and nothing else can obtain
    /// one. An implementation that loses it has made the clerk re-search, which is the
    /// *"Register fails. The form keeps its values."* requirement broken.
    ///
    /// # Cancellation is NOT specified, and that is tracked
    ///
    /// Dropping this future after the database has committed leaves the caller with neither
    /// `Ok` nor `Err`, holding the attestation. The obvious recovery — restore and let the
    /// clerk retry — mints a SECOND chart, because `register_patient` generates a fresh
    /// `Uuid::now_v7()` per call. Until
    /// [#649](https://github.com/cairn-ehr/cairn-ehr/issues/649) settles whether the id
    /// becomes caller-supplied, treat this future as **cancellation-unsafe**: do not race it
    /// against a timeout or a `select!`.
    ///
    /// # A live implementation and `&mut Client`
    ///
    /// Stated here so 2b does not discover it by fighting the compiler:
    /// `cairn_node::patient::register::register_patient` takes `&mut Client`, while this
    /// takes `&self` and must return a `Send` future. A `std::sync::Mutex` guard is not
    /// `Send`, so a live implementation needs a `tokio::sync::Mutex` or a connection pool.
    /// The mock's "compute before the async block" trick is NOT a general recipe — it works
    /// only because a fixture has no await in it.
    fn register(
        &self,
        attested: AttestedSearch,
        name: Option<&str>,
    ) -> impl std::future::Future<Output = Result<uuid::Uuid, (DataError, AttestedSearch)>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A refusal and an outage must not compare equal, and must not be reachable through one
    /// arm. The window renders them with DIFFERENT advice — "try again" against "this cannot
    /// succeed as typed" — so a caller that matched them together would hand a clerk a retry
    /// button for a verdict (#648).
    #[test]
    fn a_refusal_is_not_an_outage() {
        let refused = DataError::Refused("registration refused: no search terms".into());
        let outage = DataError::Unavailable("connection closed".into());
        assert_ne!(refused, outage);
        assert!(
            !matches!(refused, DataError::Unavailable(_)),
            "a floor verdict must never arrive through the outage arm"
        );
    }

    /// The variant carries the floor's own words, not a category label. `commands.rs`'s rule
    /// 1 — return the underlying error text, never a generic string — is what makes an in-DB
    /// refusal actionable (§9.6); a `Refused` with nothing in it would be the same silence
    /// one variant over.
    #[test]
    fn a_refusal_carries_the_floors_own_words() {
        let DataError::Refused(text) =
            DataError::Refused("db/045: attested query has no terms".into())
        else {
            panic!("constructed as Refused");
        };
        assert!(text.contains("db/045"), "got: {text}");
    }
}
