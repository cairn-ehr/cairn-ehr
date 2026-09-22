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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataError {
    NotFound,
    Unavailable(String),
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
    /// The node's own CLI reads it from the DATABASE's `current_date`, never the operator's
    /// wall clock, and a live implementation of this port should do the same.
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
    fn register(
        &self,
        attested: &AttestedSearch,
        name: Option<&str>,
    ) -> impl std::future::Future<Output = Result<uuid::Uuid, DataError>> + Send;
}
