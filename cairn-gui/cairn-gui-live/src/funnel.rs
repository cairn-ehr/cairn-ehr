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
        // `data_error_from` here can only ever answer `Unavailable` today, and that is a fact
        // about the floor rather than an oversight: `cairn_search_candidates` is `LANGUAGE sql
        // STABLE` with no `RAISE` in it, and the other reads `search_patients` performs are
        // plain SELECTs over views. db/045's refusals are registration-time. It is called
        // anyway — a search that grows a deterministic refusal must not have to remember to
        // start classifying, and the outage arm is the one that matters here regardless: a
        // failed search reported as an EMPTY list tells the clerk "nobody matched", which on
        // this screen means "create a new chart". See
        // `a_failed_search_is_an_error_not_an_empty_list`.
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
        // `AttestedSearch` exists (see `cairn_gui_funnel::token`).
        //
        // `name` is NOT protected that way, and this adapter cannot make it so. It is an
        // argument unrelated to `attested`, and `register_patient`'s own doc says the same of
        // its types: a caller could attest a search for "Smith" while asserting the name
        // "Jones" and it would sign both without complaint. The tests in this crate DELIBERATELY
        // diverge the two — registering "Kowalczyk Newcomer" against a search for "Kowalczyk" —
        // to prove this port preserves what it is handed rather than reconciling it.
        //
        // The invariant that `name` is the raw typed string the query was built from belongs
        // to the WINDOW: one name field feeding `SearchQuery::new` and this argument alike.
        // That is what `port.rs` means by "the funnel satisfies it by construction", and it
        // lands with slice 2c.
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
