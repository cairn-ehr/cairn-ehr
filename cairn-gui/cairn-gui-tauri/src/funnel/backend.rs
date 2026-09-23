//! Mock or live: the one place the funnel's commands learn which mode the window launched in.
//!
//! The ports are deliberately not dyn-compatible (`cairn_gui_data::port`'s note on `impl
//! Future + Send`), so the window dispatches on an enum rather than a trait object. That is not
//! a workaround: the window KNOWS its mode, and an enum built once at launch is a value that
//! cannot later disagree with itself — the reasoning `AppState::is_mock` already carries.
use cairn_gui_data::mock::MockData;
use cairn_gui_data::port::{DataError, PatientRegistration, PatientSearch};
use cairn_gui_funnel::AttestedSearch;
use cairn_gui_live::LiveData;
use cairn_patient_search::{CandidateList, SearchQuery};
use std::time::SystemTime;
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
    /// ⚠️ Cancellation-unsafe (#649, #669): never race this against a timeout or `select!`.
    pub async fn register(
        &self,
        attested: AttestedSearch,
        name: &str,
    ) -> Result<Uuid, (DataError, AttestedSearch)> {
        match self {
            FunnelBackend::Mock(mock) => mock.register(attested, Some(name)).await,
            FunnelBackend::Live(live) => live.register(attested, Some(name)).await,
        }
    }

    /// Refuse, naming the remedy, unless this node may write (#665). The mock always may: it
    /// writes to nothing.
    pub async fn require_provisioned(&self) -> Result<(), DataError> {
        match self {
            FunnelBackend::Mock(_) => Ok(()),
            FunnelBackend::Live(live) => live.require_provisioned().await,
        }
    }
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
    use std::time::{Duration, UNIX_EPOCH};

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
        assert!(!list.candidates.is_empty());
    }
}
