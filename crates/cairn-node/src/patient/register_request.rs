//! The window's half of a registration through the node's terminal service (#909, design D6).
//!
//! # Where this fits
//!
//! The window no longer holds the node's signing key, and a registration is node-signed
//! (ADR-0061). So the window does not register: it writes a REQUEST to db/083's queue
//! ([`submit`]) naming the registration's inputs (the typed name, the attested search, the day),
//! and the terminal service (`terminal_service::registration::serve_one`) registers it and
//! settles the row. The window then reads the row ([`status`]) every [`POLL`] until it settles.
//!
//! # No late registration
//!
//! db/083 gives each request a [`DEADLINE`] after which the service no longer claims it. A window
//! still waiting at the deadline calls [`expire`] once. That door takes the same row lock as the
//! service's claim, so if the service is registering the request right then, `expire` WAITS for
//! it and returns what the service committed. The window therefore never reports "not
//! registered" for a request that went in. [`next_step`] is the pure rule for when to wait and
//! when to expire.
//!
//! # What the caller owes the clerk
//!
//! A request id is the caller's: generate a fresh random one per press (any window login can
//! read a request by id, and ids are idempotent only until db/083's 24 h purge). Once [`submit`]
//! has succeeded, the service may register the request at any moment until its deadline, so
//! ANY later failure (a lost connection while polling, an `expire` that gets no answer) means
//! the chart MAY exist (#815). The caller must say so, never "nothing was saved".

use anyhow::{bail, Context};
use cairn_patient_search::{CandidateList, SearchQuery};
use std::time::Duration;
use tokio_postgres::GenericClient;
use uuid::Uuid;

/// The window's words when the node's terminal service never answered a request: its deadline
/// passed with the request still pending, and [`expire`] marked it expired (so it never goes in).
pub const SERVICE_NOT_RUNNING: &str = "nothing was registered: registration needs the node's terminal service (cairn-node terminal-service), which did not answer; tell the site's operator";

/// How often the window reads its request's status.
pub const POLL: Duration = Duration::from_millis(200);

/// How long the window waits before expiring its request. Must equal the `interval` db/083's
/// `cairn_registration_submit` writes (a test reads the SQL to pin the two together): a shorter
/// wait would expire requests the service could still serve, a longer one would wait on a
/// request the service has stopped claiming.
pub const DEADLINE: Duration = Duration::from_secs(10);

/// What became of one request, as db/083's row says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestStatus {
    /// Not served yet.
    Pending,
    /// The service made this chart.
    Registered(Uuid),
    /// The service refused, in these words (written for the clerk).
    Refused(String),
    /// The window gave up on it before the service served it: nothing was registered.
    Expired,
}

/// The window's next move while it waits on a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Next {
    /// Still pending, deadline not reached: read the status again after [`POLL`].
    Wait,
    /// Still pending at the deadline: call [`expire`] once.
    Expire,
    /// Settled: this is the outcome.
    Done(RequestStatus),
}

/// When to wait, when to expire, when to stop. **Pure.**
///
/// `waited` is measured by the window from the moment [`submit`] answered, so it always runs
/// behind the database's own clock (the deadline was stamped before the answer left), and the
/// window never expires a request before db/083 would.
pub fn next_step(status: &RequestStatus, waited: Duration, deadline: Duration) -> Next {
    match status {
        RequestStatus::Pending if waited < deadline => Next::Wait,
        RequestStatus::Pending => Next::Expire,
        settled => Next::Done(settled.clone()),
    }
}

/// Read one status row (`status`, `patient_id`, `refusal`) into a [`RequestStatus`]. **Pure.**
/// db/083's settle door guarantees a registered row names its chart and a refused row its
/// reason, so a row breaking that is a bug, reported as an error rather than guessed around.
fn parse_status(
    status: &str,
    patient: Option<String>,
    refusal: Option<String>,
) -> anyhow::Result<RequestStatus> {
    Ok(match (status, patient, refusal) {
        ("pending", _, _) => RequestStatus::Pending,
        ("registered", Some(p), _) => RequestStatus::Registered(p.parse()?),
        ("refused", _, Some(r)) => RequestStatus::Refused(r),
        ("expired", _, _) => RequestStatus::Expired,
        (other, _, _) => bail!("registration request row has an unreadable status {other:?}"),
    })
}

/// Ask the terminal service to register `name` under the attested search (`query` and the
/// `displayed` list it produced), as request `request`. Returns the request's status: pending
/// on a first submit.
///
/// `query` and `displayed` are written with the SAME serde shapes the service reads them back
/// with (`serve_one`'s decode step). `name` must be the raw typed string `query` was built from;
/// the service refuses any other. `today` is the date the window's search was asked as of.
///
/// A re-send of the same id with the same inputs answers with the first one's status; the same
/// id with different inputs is refused by the door (P0001).
pub async fn submit(
    client: &impl GenericClient,
    request: Uuid,
    name: Option<&str>,
    query: &SearchQuery,
    displayed: &CandidateList,
    today: &str,
) -> anyhow::Result<RequestStatus> {
    let row = client
        .query_one(
            "SELECT cairn_registration_submit($1::text::uuid, $2, $3::text::jsonb, \
             $4::text::jsonb, $5)",
            &[
                &request.to_string(),
                &name,
                &serde_json::to_string(query)?,
                &serde_json::to_string(displayed)?,
                &today,
            ],
        )
        .await
        .with_context(|| format!("submitting registration request {request}"))?;
    match row.get::<_, &str>(0) {
        "pending" => Ok(RequestStatus::Pending),
        // Only a re-send can meet a settled row; read its chart or reason.
        _ => status(client, request).await,
    }
}

/// What became of request `request`. An unknown id is refused by the door (a caller bug).
pub async fn status(client: &impl GenericClient, request: Uuid) -> anyhow::Result<RequestStatus> {
    let row = client
        .query_one(
            "SELECT status, patient_id::text, refusal \
               FROM cairn_registration_status($1::text::uuid)",
            &[&request.to_string()],
        )
        .await
        .with_context(|| format!("reading registration request {request}"))?;
    parse_status(row.get(0), row.get(1), row.get(2))
}

/// Give up on request `request`: mark it expired if the service has not served it, and return
/// what it then is. **May block** while the service holds the request (it is registering it
/// right now); that wait ends with the service's outcome, so a caller must never time it out and
/// read the timeout as "expired".
pub async fn expire(client: &impl GenericClient, request: Uuid) -> anyhow::Result<RequestStatus> {
    let row = client
        .query_one(
            "SELECT cairn_registration_expire($1::text::uuid)",
            &[&request.to_string()],
        )
        .await
        .with_context(|| format!("expiring registration request {request}"))?;
    match row.get::<_, &str>(0) {
        "expired" => Ok(RequestStatus::Expired),
        // The service settled it first (or while expire waited on its lock): read its outcome.
        _ => match status(client, request).await? {
            RequestStatus::Pending => {
                bail!("registration request {request} is still pending after expire")
            }
            settled => Ok(settled),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const D: Duration = DEADLINE;

    #[test]
    fn pending_before_the_deadline_waits() {
        assert_eq!(
            next_step(&RequestStatus::Pending, Duration::ZERO, D),
            Next::Wait
        );
        assert_eq!(
            next_step(&RequestStatus::Pending, D - Duration::from_millis(1), D),
            Next::Wait
        );
    }

    #[test]
    fn pending_at_or_after_the_deadline_expires() {
        assert_eq!(next_step(&RequestStatus::Pending, D, D), Next::Expire);
        assert_eq!(next_step(&RequestStatus::Pending, D * 3, D), Next::Expire);
    }

    #[test]
    fn a_settled_request_is_done_whenever_it_is_read() {
        let chart = Uuid::now_v7();
        for settled in [
            RequestStatus::Registered(chart),
            RequestStatus::Refused("no".into()),
            RequestStatus::Expired,
        ] {
            for waited in [Duration::ZERO, D, D * 2] {
                assert_eq!(next_step(&settled, waited, D), Next::Done(settled.clone()));
            }
        }
    }

    /// The window's wait and db/083's deadline are one number written in two languages.
    #[test]
    fn deadline_matches_db_083() {
        let sql = include_str!("../../../../db/083_terminal_service.sql");
        assert!(
            sql.contains("now() + interval '10 seconds'"),
            "db/083's submit deadline changed: change DEADLINE with it"
        );
        assert_eq!(DEADLINE, Duration::from_secs(10));
    }

    #[test]
    fn a_status_row_reads_back_or_is_an_error() {
        let chart = Uuid::now_v7();
        let read = |s, p: Option<&str>, r: Option<&str>| {
            parse_status(s, p.map(str::to_string), r.map(str::to_string))
        };
        assert_eq!(read("pending", None, None).unwrap(), RequestStatus::Pending);
        assert_eq!(
            read("registered", Some(&chart.to_string()), None).unwrap(),
            RequestStatus::Registered(chart)
        );
        assert_eq!(
            read("refused", None, Some("why")).unwrap(),
            RequestStatus::Refused("why".into())
        );
        assert_eq!(read("expired", None, None).unwrap(), RequestStatus::Expired);
        assert!(read("registered", None, None).is_err());
        assert!(read("refused", None, None).is_err());
        assert!(read("lost", None, None).is_err());
    }
}
