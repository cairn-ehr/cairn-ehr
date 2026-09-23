//! Three small reads the window makes of its node, beside the two funnel ports.
//!
//! None of them is clinical logic. Each delegates to `cairn-node` (or asks Postgres one
//! question) and maps its failure through [`data_error_from`] exactly as the ports do, so a
//! verdict and an outage stay two different facts on screen here too.
use crate::error::data_error_from;
use crate::LiveData;
use cairn_gui_data::port::DataError;
use cairn_node::actor_enrolment::{device_actor_standing, require_device_actor, ActorStanding};

impl LiveData {
    /// Refuse unless this node's signing key may author — asked BEFORE a registration takes its
    /// attestation out of the token store, so a refusal here leaves the clerk's search intact.
    ///
    /// The same `require_device_actor` every CLI write command asks (#654), so on an
    /// unprovisioned node the clerk meets the refusal that names its remedy, classified
    /// `NotProvisioned` (`RefusalScope::NodeState`), instead of db/005's bare key id (#665).
    ///
    /// # Why the window calls this, and `PatientRegistration::register` does not
    ///
    /// The port suites deliberately sign with an UNENROLLED key to reach db/005's refusal
    /// INSIDE the registration transaction (`refusal_is_not_an_outage.rs` — the rollback and
    /// connection-reuse proofs depend on getting that far). A pre-check inside the port would
    /// stop them reaching it and leave those proofs green and empty. The floor still refuses an
    /// unenrolled signer whoever skips this (principle 12); this only makes the refusal legible.
    pub async fn require_provisioned(&self) -> Result<(), DataError> {
        let db = self.db.lock().await;
        require_device_actor(&db, &self.node_kid)
            .await
            .map_err(|e| data_error_from(&e))
    }

    /// Where this node's signing key stands — the window's launch probe (#654 option 2).
    ///
    /// All four answers, never a boolean: a `Retired` key told to run `enroll-device-actor`
    /// would meet db/004's resurrection refusal while following the advice (#152).
    pub async fn standing(&self) -> Result<ActorStanding, DataError> {
        let db = self.db.lock().await;
        device_actor_standing(&db, &self.node_kid)
            .await
            .map_err(|e| data_error_from(&e))
    }

    /// The DATABASE's date, as `YYYY-MM-DD`, for `PatientSearch::search`'s `today`.
    ///
    /// The node's own CLI reads the same `current_date`, so a displayed age cannot depend on
    /// which machine's clock won. Asked per search rather than cached at launch: a window left
    /// open past midnight must not show every patient a day younger than they are.
    pub async fn today(&self) -> Result<String, DataError> {
        let db = self.db.lock().await;
        let row = db
            .query_one("SELECT current_date::text", &[])
            .await
            .map_err(|e| data_error_from(&anyhow::Error::from(e)))?;
        Ok(row.get(0))
    }
}
