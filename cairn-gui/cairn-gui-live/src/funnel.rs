//! The two port implementations. Each is a thin adapter over a `cairn-node` orchestrator:
//! resolve the connection, call, map the error. **No clinical logic lives here** — the same
//! rule `commands.rs` follows one layer up, and for the same reason: a rule that lives in an
//! adapter is a rule no test goes looking for.
use crate::error::data_error_from;
use crate::LiveData;
use cairn_gui_data::port::{DataError, PatientRegistration, PatientSearch};
use cairn_gui_funnel::AttestedSearch;
use cairn_patient_search::{CandidateList, SearchQuery};
use uuid::Uuid;

// `async fn` in the impl of an RPITIT trait method is allowed, and is what clippy asks for
// here; the `+ Send` bound declared on the trait is still checked against these bodies. They
// satisfy it because a `tokio::sync::MutexGuard` IS `Send` — which is the whole reason
// `LiveData` holds a tokio mutex rather than a `std` one (see its doc).

impl PatientSearch for LiveData {
    async fn search(&self, query: &SearchQuery, today: &str) -> Result<CandidateList, DataError> {
        let db = self.db.lock().await;
        // `today` is passed STRAIGHT THROUGH. The port's doc forbids an implementation from
        // substituting its own clock: the age beside a patient's name would then depend on
        // which clock won, with nothing on screen saying which.
        cairn_node::patient::search::search_patients(&*db, query, today)
            .await
            .map_err(|e| data_error_from(&e))
    }
}

impl PatientRegistration for LiveData {
    async fn register(
        &self,
        attested: AttestedSearch,
        name: Option<&str>,
    ) -> Result<Uuid, (DataError, AttestedSearch)> {
        let mut db = self.db.lock().await;
        // The query and the list come out of ONE value that has no public constructor, so this
        // call cannot be handed a pair that were never together — which is the whole reason
        // `AttestedSearch` exists (see `cairn_gui_funnel::token`). `name` is the raw typed
        // string the query was built from, never reassembled; the funnel's single name field
        // is what makes that true by construction rather than by convention.
        let outcome = cairn_node::patient::register::register_patient(
            &mut db,
            &self.node_sk,
            &self.node_kid,
            &self.node_origin,
            name,
            attested.query(),
            attested.displayed(),
        )
        .await;

        // The attestation goes BACK inside the error, and that is not decoration:
        // `TokenStore::restore` needs exactly this value to put the search back after a failed
        // write, and nothing else in the program can obtain one. Losing it here makes the
        // clerk re-search, which is the design's *"Register fails. The form keeps its
        // values."* broken — and it strands the token store in-flight, which latches it closed
        // until the window reloads.
        match outcome {
            Ok(id) => Ok(id),
            Err(e) => Err((data_error_from(&e), attested)),
        }
    }
}
