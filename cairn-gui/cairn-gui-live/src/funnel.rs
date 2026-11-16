//! The two port implementations. Each is a thin adapter over a `cairn-node` orchestrator:
//! resolve the connection, call, map the error. (Registration's "orchestrator" is a request
//! to the node's terminal service, and waiting for its answer, #909.) **No clinical logic lives here** — the same
//! rule `commands.rs` follows one layer up, and for the same reason: a rule that lives in an
//! adapter is a rule no test goes looking for.
use crate::error::data_error_from;
use crate::LiveData;
use cairn_gui_data::port::{DataError, PatientRegistration, PatientSearch};
use cairn_gui_funnel::AttestedSearch;
use cairn_node::patient::register_request::{
    self as request_door, next_step, Next, RequestStatus, DEADLINE, POLL, SERVICE_NOT_RUNNING,
};
use cairn_node::terminal_service::registration::NEW_MATCH_REFUSAL;
use cairn_patient_search::{CandidateList, SearchQuery};
use std::time::Instant;
use uuid::Uuid;

// `async fn` in the impl of an RPITIT trait method is allowed, and is what clippy asks for
// here; the `+ Send` bound declared on the trait is still checked against these bodies. They
// satisfy it because a `tokio::sync::MutexGuard` IS `Send` — which is the whole reason
// `LiveData` holds a tokio mutex rather than a `std` one (see its doc).

impl PatientSearch for LiveData {
    async fn search(&self, query: &SearchQuery, today: &str) -> Result<CandidateList, DataError> {
        let db = self.db.lock().await?;
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
            .map_err(|e| data_error_from(&db.explain(e)))
    }
}

impl PatientRegistration for LiveData {
    /// Register through the node's terminal service (#909): submit a request, then poll it until
    /// the service settles it or its deadline passes. See `cairn_node::patient::register_request`.
    ///
    /// The connection is locked per call (submit, each poll, expire), never across a wait, so
    /// the chart commands sharing it are not held up for the seconds a registration may take.
    async fn register(
        &self,
        attested: AttestedSearch,
        name: Option<&str>,
    ) -> Result<Uuid, (DataError, AttestedSearch)> {
        // The query and the list come out of ONE value that has no public constructor, so this
        // call cannot be handed a pair that were never together — which is the whole reason
        // `AttestedSearch` exists (see `cairn_gui_funnel::token`). They are submitted as is:
        // the bounded list the clerk was shown, never re-derived.
        //
        // `name` is NOT protected that way. The invariant that it is the raw typed string the
        // query was built from belongs to the WINDOW (`NamedAttestation`, slice 2c), and the
        // terminal service refuses a request whose name is not the one its query was built
        // from, so a diverged pair cannot be registered either way.
        //
        // A fresh random id per press: any window login can read a request by id, and an id is
        // idempotent only until db/083's purge, so it is never derived or reused.
        let request = Uuid::new_v4();

        // The attestation goes BACK inside every error, and that is not decoration:
        // `TokenStore::restore` needs exactly this value to put the search back after a failed
        // write, and nothing else in the program can obtain one. Losing it here makes the
        // clerk re-search, which is the design's *"Register fails. The form keeps its
        // values."* broken — and it strands the token store in-flight, which latches it closed
        // until the window reloads.
        let submitted = match self.submit_request(request, &attested, name).await {
            Ok(status) => status,
            Err(e) => return Err((e, attested)),
        };
        match self
            .await_settled(request, submitted)
            .await
            .and_then(settled)
        {
            Ok(chart) => Ok(chart),
            Err(e) => Err((e, attested)),
        }
    }
}

impl LiveData {
    /// Put the request in db/083's queue. Until this answers, nothing is queued, so a failure
    /// here registered nothing and is classified like any other call's (verdict or outage).
    ///
    /// `today` is the DATABASE's date, the clock the window's searches use (`LiveData::today`):
    /// the service re-runs the search as of it.
    async fn submit_request(
        &self,
        request: Uuid,
        attested: &AttestedSearch,
        name: Option<&str>,
    ) -> Result<RequestStatus, DataError> {
        let today = self.today().await?;
        let db = self.db.lock().await?;
        request_door::submit(
            &*db,
            request,
            name,
            attested.query(),
            attested.displayed(),
            &today,
        )
        .await
        .map_err(|e| data_error_from(&db.explain(e)))
    }

    /// Wait for a submitted request to settle: poll every `POLL`, and at `DEADLINE` expire it
    /// once. `expire` may block while the service is registering the request; that wait is
    /// never timed out here (the connection's own watchdog may cut it, and then the outcome is
    /// unknown, as below).
    ///
    /// From submit on, the service may register the request at any moment until its deadline,
    /// so ANY failure here means the chart MAY exist (#815): it is `OutcomeUnknown`, never an
    /// outage the clerk would retry into a second chart.
    async fn await_settled(
        &self,
        request: Uuid,
        mut status: RequestStatus,
    ) -> Result<RequestStatus, DataError> {
        let unknown = |e: DataError| may_be_registered(request, &e);
        let since = Instant::now();
        loop {
            status = match next_step(&status, since.elapsed(), DEADLINE) {
                Next::Done(settled) => return Ok(settled),
                Next::Wait => {
                    tokio::time::sleep(POLL).await;
                    let db = self.db.lock().await.map_err(|l| unknown(l.into()))?;
                    request_door::status(&*db, request)
                        .await
                        .map_err(|e| unknown(data_error_from(&db.explain(e))))?
                }
                Next::Expire => {
                    let db = self.db.lock().await.map_err(|l| unknown(l.into()))?;
                    request_door::expire(&*db, request)
                        .await
                        .map_err(|e| unknown(data_error_from(&db.explain(e))))?
                }
            };
        }
    }
}

/// A settled request as the port's answer. **Pure.** The new-match refusal is `Stale` (the
/// record changed since the clerk's search: search again), any other refusal is the service's
/// verdict in its own words, and an expired request is the node not serving registrations (only
/// an operator can mend that). `Pending` never reaches here; if it did, it is unknown.
fn settled(status: RequestStatus) -> Result<Uuid, DataError> {
    match status {
        RequestStatus::Registered(chart) => Ok(chart),
        RequestStatus::Refused(t) if t == NEW_MATCH_REFUSAL => Err(DataError::Stale(t)),
        RequestStatus::Refused(t) => Err(DataError::Refused(t)),
        RequestStatus::Expired => Err(DataError::NotProvisioned(SERVICE_NOT_RUNNING.into())),
        RequestStatus::Pending => Err(DataError::OutcomeUnknown(
            "the registration request is still pending".into(),
        )),
    }
}

/// `e` struck after the request was queued, so the chart may exist (#815). **Pure.**
fn may_be_registered(request: Uuid, e: &DataError) -> DataError {
    let cause = match e {
        DataError::NotFound => "not found",
        DataError::Unavailable(t)
        | DataError::Refused(t)
        | DataError::NotProvisioned(t)
        | DataError::Stale(t)
        | DataError::OutcomeUnknown(t) => t,
    };
    DataError::OutcomeUnknown(format!(
        "registration request {request} went to the node's terminal service, which may yet \
         register it, and then: {cause}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The mapping the window's sentences hang on (#909): each settled outcome has its own cause.
    #[test]
    fn each_settled_outcome_is_its_own_answer() {
        let chart = Uuid::now_v7();
        assert_eq!(settled(RequestStatus::Registered(chart)), Ok(chart));
        assert_eq!(
            settled(RequestStatus::Refused(NEW_MATCH_REFUSAL.into())),
            Err(DataError::Stale(NEW_MATCH_REFUSAL.into()))
        );
        assert_eq!(
            settled(RequestStatus::Refused("not enrolled".into())),
            Err(DataError::Refused("not enrolled".into()))
        );
        assert_eq!(
            settled(RequestStatus::Expired),
            Err(DataError::NotProvisioned(SERVICE_NOT_RUNNING.into()))
        );
        assert!(matches!(
            settled(RequestStatus::Pending),
            Err(DataError::OutcomeUnknown(_))
        ));
    }

    /// After submit, even an outage or a refusal may hide a registration: never retried live.
    #[test]
    fn a_failure_after_submit_is_an_unknown_outcome_keeping_its_cause() {
        let request = Uuid::now_v7();
        for e in [
            DataError::Unavailable("connection closed".into()),
            DataError::NotProvisioned("connection closed".into()),
        ] {
            let DataError::OutcomeUnknown(t) = may_be_registered(request, &e) else {
                panic!("{e:?} after submit must be an unknown outcome");
            };
            assert!(
                t.contains("connection closed") && t.contains(&request.to_string()),
                "{t}"
            );
        }
    }
}
