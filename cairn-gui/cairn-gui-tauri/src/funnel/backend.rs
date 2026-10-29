//! Mock or live: the one place the funnel's commands learn which mode the window launched in.
//!
//! The ports are deliberately not dyn-compatible (`cairn_gui_data::port`'s note on `impl
//! Future + Send`), so the window dispatches on an enum rather than a trait object. That is not
//! a workaround: the window KNOWS its mode, and an enum built once at launch is a value that
//! cannot later disagree with itself — the reasoning `AppState::is_mock` already carries.
use crate::funnel::view::provisioning_sentence;
use cairn_gui_data::mock::MockData;
use cairn_gui_data::port::{DataError, PatientRegistration, PatientSearch};
use cairn_gui_funnel::{AttestedSearch, NamedAttestation};
use cairn_gui_live::{LiveData, NodeConnection};
use cairn_node::actor_enrolment::ActorStanding;
use cairn_patient_search::{CandidateList, SearchQuery};
use std::time::{Duration, SystemTime};
use uuid::Uuid;

/// Where the funnel's searches and its one write go.
pub enum FunnelBackend {
    /// `--mock`: a fixture population held for the life of the window, so a patient registered
    /// in this session is found by the next browse (#668 — it used to be rebuilt per command).
    Mock(MockData),
    /// A real node, sharing the chart commands' connection.
    Live(LiveData),
}

impl FunnelBackend {
    /// Run a candidate search, supplying `today` from the right clock.
    ///
    /// Live: the DATABASE's date, asked per search (a window open past midnight must not age
    /// every patient wrong), exactly as `cairn-node`'s CLI does. Mock: there is no database, so
    /// it is this process's UTC date — a fixture's displayed age may be a day off near midnight
    /// in a far-from-UTC zone, and that is a fixture being a fixture.
    pub async fn search(&self, query: &SearchQuery) -> Result<CandidateList, DataError> {
        match self {
            FunnelBackend::Mock(mock) => mock.search(query, &utc_today(SystemTime::now())).await,
            FunnelBackend::Live(live) => {
                let today = live.today().await?;
                live.search(query, &today).await
            }
        }
    }

    /// Register, handing the port the raw typed name the attested search ran on.
    ///
    /// Takes the [`NamedAttestation`] whole and splits it only here, at the port call: the
    /// name cannot be swapped for another on the way from the session to the node.
    ///
    /// ⚠️ Cancellation-unsafe (#649, #669): never race this against a timeout or `select!`.
    pub async fn register(
        &self,
        taken: NamedAttestation,
    ) -> Result<Uuid, (DataError, AttestedSearch)> {
        let (attested, name) = taken.into_parts();
        match self {
            FunnelBackend::Mock(mock) => mock.register(attested, Some(&name)).await,
            FunnelBackend::Live(live) => live.register(attested, Some(&name)).await,
        }
    }

    /// Refuse, naming the remedy, unless this node may write (#665). The mock always may — it
    /// writes to nothing — unless a test armed a `NotProvisioned`, which is how the order of
    /// this check against taking the search is tested.
    pub async fn require_provisioned(&self) -> Result<(), DataError> {
        match self {
            FunnelBackend::Mock(mock) => mock.require_provisioned().await,
            FunnelBackend::Live(live) => live.require_provisioned().await,
        }
    }

    /// The provisioning line's sentence, probed NOW (#882): once an operator mends the fault, or
    /// enrols the node after launch, the webview's next ask (boot, or a search that succeeded
    /// while the line warned) clears it without a restart. The mock writes to nothing, so it
    /// never has one.
    ///
    /// Bounded by `PROBE_WITHIN`: the webview's `boot()` shows the front door only after this
    /// answers, and a connection lock held by a command stuck on a dead socket (#875) must not
    /// blank the window.
    pub async fn provisioning(&self) -> Option<String> {
        match self {
            FunnelBackend::Mock(_) => None,
            FunnelBackend::Live(live) => provisioning_sentence(
                probe_within(live.standing(), PROBE_WITHIN).await,
                live.node_kid(),
            ),
        }
    }
}

/// How long the provisioning probe waits: a whole reopen's wait, then as long again for the
/// query. Longer than the reopen's own bound on purpose: a reopen cut off by this timer would
/// record no failure, so its backoff and its log line would be lost.
const PROBE_WITHIN: Duration = NodeConnection::REOPEN_WITHIN.saturating_mul(2);

/// `probe`'s answer, or an outage naming the wait if none came `within` — which
/// `provisioning_sentence` then says as "could not check".
async fn probe_within(
    probe: impl std::future::Future<Output = Result<ActorStanding, DataError>>,
    within: Duration,
) -> Result<ActorStanding, DataError> {
    tokio::time::timeout(within, probe)
        .await
        .unwrap_or_else(|_| {
            Err(DataError::Unavailable(format!(
                "no answer within {within:?}"
            )))
        })
}

/// `now` as a UTC calendar date, `YYYY-MM-DD` — the mock's `today`.
///
/// Pure (the clock is the caller's) and dependency-free: Howard Hinnant's `civil_from_days`,
/// the proleptic-Gregorian conversion from a day count since 1970-01-01. A clock set before the
/// epoch yields `1970-01-01` rather than panicking; no machine running this has one.
pub fn utc_today(now: SystemTime) -> String {
    let secs = now
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let days = (secs / 86_400) as i64;
    // Shift the epoch to 0000-03-01 so each 400-year era starts just after a leap day.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_from_march = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_from_march + 2) / 5 + 1;
    let month = if month_from_march < 10 {
        month_from_march + 3
    } else {
        month_from_march - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::UNIX_EPOCH;

    #[test]
    fn utc_today_is_the_civil_date() {
        assert_eq!(utc_today(UNIX_EPOCH), "1970-01-01");
        // 2000-02-29 is day 11 016 — a leap day in a century year is where civil-from-days
        // arithmetic goes wrong if it goes wrong anywhere.
        assert_eq!(
            utc_today(UNIX_EPOCH + Duration::from_secs(11_016 * 86_400 + 3_600)),
            "2000-02-29"
        );
        assert_eq!(
            utc_today(UNIX_EPOCH + Duration::from_secs(20_719 * 86_400)),
            "2026-09-23"
        );
        // The last second of a day is still that day.
        assert_eq!(
            utc_today(UNIX_EPOCH + Duration::from_secs(20_720 * 86_400 - 1)),
            "2026-09-23"
        );
    }

    /// PR #887 review: `boot()` shows the front door only after `funnel_status` answers, so a
    /// probe stuck behind a hung connection lock must give up and say so, not blank the window.
    #[tokio::test]
    async fn a_probe_with_no_answer_gives_up_and_says_so() {
        let within = Duration::from_millis(20);
        let outcome = probe_within(std::future::pending(), within).await;
        assert_eq!(
            outcome,
            Err(DataError::Unavailable("no answer within 20ms".into()))
        );
        let answered = probe_within(async { Ok(ActorStanding::Enrolled) }, within).await;
        assert_eq!(answered, Ok(ActorStanding::Enrolled));
    }

    #[test]
    fn a_clock_before_the_epoch_does_not_panic() {
        assert_eq!(
            utc_today(UNIX_EPOCH - Duration::from_secs(10)),
            "1970-01-01"
        );
    }

    #[tokio::test]
    async fn the_mock_backend_is_always_provisioned_and_searches_fixtures() {
        let b = FunnelBackend::Mock(MockData::with_fixtures());
        b.require_provisioned().await.unwrap();
        let list = b
            .search(&SearchQuery::new("mich", None, &[]))
            .await
            .unwrap();
        assert!(!list.people.is_empty());
    }
}
