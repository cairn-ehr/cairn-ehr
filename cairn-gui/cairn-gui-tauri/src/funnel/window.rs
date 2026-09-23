//! The window's funnel-facing state: which chart is open, and how fixture mode is built.
//!
//! `AppState` itself lives in `state.rs`; these `impl` blocks live here so that file stays about
//! the signing session it was written for.
use crate::funnel::backend::FunnelBackend;
use crate::funnel::view::{header_opened_by_id, ChartHeaderView};
use crate::state::AppState;
use cairn_gui_data::mock::MockData;
use cairn_gui_funnel::FunnelSession;
use uuid::Uuid;

/// A chart the window is open on, with the identity header shown over it.
#[derive(Debug, Clone)]
pub struct OpenChart {
    pub patient: Uuid,
    pub header: ChartHeaderView,
}

impl OpenChart {
    /// A chart named by id at launch (`--patient`), whose name the window has not read.
    pub fn by_id(patient: Uuid) -> Self {
        Self {
            patient,
            header: header_opened_by_id(patient),
        }
    }
}

impl AppState {
    /// The ONE fixture-mode constructor — `main.rs` and every test use it.
    ///
    /// No connection, no node key, a mock funnel backend: all three together, so fixture mode
    /// can never hold one of them without the others (a boolean that can disagree with reality
    /// is how a "mock" window ends up writing to a real database).
    pub fn mock(patient: Option<Uuid>) -> Self {
        AppState {
            db: None,
            node_sk: None,
            node_origin: String::new(),
            chart: tokio::sync::Mutex::new(patient.map(OpenChart::by_id)),
            funnel_backend: FunnelBackend::Mock(MockData::with_fixtures()),
            funnel: tokio::sync::Mutex::new(FunnelSession::new()),
            shown: tokio::sync::Mutex::new(Default::default()),
            provisioning: None,
            attester_key_path: None,
            session: tokio::sync::Mutex::new(None),
        }
    }

    /// The chart every chart command acts on — or a refusal when the front door is showing.
    ///
    /// Read afresh per command: a chart command must never act on a patient the window has
    /// since closed (Review Focus 5).
    pub async fn open_patient(&self) -> Result<Uuid, String> {
        self.chart
            .lock()
            .await
            .as_ref()
            .map(|chart| chart.patient)
            .ok_or_else(|| "no chart is open — find or register a patient first".to_string())
    }

    /// The open chart, provided it is the one the SCREEN is showing — what every chart command
    /// acts on.
    ///
    /// # Why the webview must say which chart it means (final review, Critical #1)
    ///
    /// Before slice 2c a window had one patient for its whole life, so "the open chart" and "the
    /// chart on screen" could not differ. Now the clerk switches charts, and the two can differ
    /// for as long as a read is in flight (or forever, if it fails): patient A's medication table
    /// still on screen under patient B's header. A sign-off that resolved only "the open chart"
    /// would then sign B's medications on the strength of a review of A's — the wrong-chart act
    /// the identity header exists to prevent. So the webview sends the chart id it is DISPLAYING,
    /// and a command whose id is not the open chart refuses rather than acting on either one.
    pub async fn displayed_patient(&self, displayed: &str) -> Result<Uuid, String> {
        let open = self.open_patient().await?;
        let shown: Uuid = displayed.parse().map_err(|_| {
            "this window could not tell which chart is on screen — reopen it".to_string()
        })?;
        if shown == open {
            Ok(open)
        } else {
            Err(
                "the chart on screen is not the chart that is open — nothing was done; reopen the \
                 patient you meant"
                    .to_string(),
            )
        }
    }

    /// The fixture population, for tests that arm a failure. Test-only on purpose: the shipped
    /// binary has no way to make its next write fail (#668's gating note).
    #[cfg(test)]
    pub fn mock_data(&self) -> Option<&MockData> {
        match &self.funnel_backend {
            FunnelBackend::Mock(mock) => Some(mock),
            FunnelBackend::Live(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Review Focus follow-up (final review, Critical #1): a chart command must act on the chart
    /// the SCREEN shows, never merely on whichever chart is open by the time it runs.
    #[tokio::test]
    async fn a_chart_command_is_bound_to_the_chart_on_screen() {
        let a = Uuid::from_u128(1);
        let state = AppState::mock(Some(a));
        assert_eq!(state.displayed_patient(&a.to_string()).await.unwrap(), a);
        let b = Uuid::from_u128(2);
        let err = state.displayed_patient(&b.to_string()).await.unwrap_err();
        assert!(err.contains("not the chart"), "{err}");
    }

    #[tokio::test]
    async fn a_chart_command_with_no_chart_open_is_refused() {
        let state = AppState::mock(None);
        let err = state
            .displayed_patient(&Uuid::from_u128(1).to_string())
            .await
            .unwrap_err();
        assert!(err.contains("no chart is open"), "{err}");
    }

    #[tokio::test]
    async fn an_unreadable_displayed_id_is_refused() {
        let state = AppState::mock(Some(Uuid::from_u128(1)));
        assert!(state.displayed_patient("not-a-uuid").await.is_err());
    }

    /// Fixture mode's three facts come together or not at all.
    #[test]
    fn fixture_mode_has_no_connection_and_a_mock_backend() {
        let state = AppState::mock(None);
        assert!(state.is_mock());
        assert!(state.node_sk.is_none());
        assert!(matches!(state.funnel_backend, FunnelBackend::Mock(_)));
    }
}
