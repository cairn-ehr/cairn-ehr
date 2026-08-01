//! The medication read model shared by the node's read path, the CLI, and the UI.
//!
//! WHY A SHARED CRATE. Two consumers must agree on one question — *which threads does a
//! sign-off gesture attest?* `cairn-node`'s orchestrator answers it to decide what to
//! sign; the UI answers it to tell the clinician what is about to be signed. If those
//! were two implementations, a divergence would put a green "signed" badge over a thread
//! nobody signed. So the model and the rule live here, and both sides depend on it.
//!
//! This crate is deliberately pure: no database driver, no GUI toolkit. That is what lets
//! the GUI tab crate test in milliseconds without Postgres in the build tree.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Whether a displayed medication group is still being taken.
///
/// Ceased rows are RETAINED in the list, not filtered out: a struck line stays visible on
/// a paper drug chart, and dropping it would lose that parity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MedicationStatus {
    Active,
    Ceased,
}

/// The ADR-0049 sign-off state of ONE medication thread.
///
/// `by` is the attester's hex key id, as recorded in `medication_attestation.attester_kid`.
/// Staleness is NOT computed here — it is read from `medication_thread_attestation.stale`,
/// which the database derives from the set-commitment compare. A second implementation of
/// staleness would be a second answer to a safety question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VouchState {
    /// No attestation on this thread at all.
    Absent,
    /// A current vouch.
    Fresh { by: String },
    /// A vouch whose set-commitment no longer matches the thread's content.
    Stale { by: String },
}

impl VouchState {
    /// True when a sign-off gesture must (re-)vouch this thread.
    pub fn needs_signature(&self) -> bool {
        matches!(self, VouchState::Absent | VouchState::Stale { .. })
    }

    /// The attester's key id, when there is one.
    pub fn attester(&self) -> Option<&str> {
        match self {
            VouchState::Absent => None,
            VouchState::Fresh { by } | VouchState::Stale { by } => Some(by),
        }
    }
}

/// One member thread of a displayed row, with the vouch that thread carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberVouch {
    pub medication_id: Uuid,
    pub vouch: VouchState,
}

/// One displayed row = one medication GROUP.
///
/// A group is what `patient_medication_current` emits: reconciled duplicate threads
/// (ADR-0047) collapse into a single clinical statement. Attestation, however, is
/// per-THREAD, so the row carries its members and each member's vouch. That group/thread
/// asymmetry is the most defect-prone seam in this slice — see the tests in
/// `targeting.rs` and `crates/cairn-node/tests/medication_read.rs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MedicationRow {
    pub group_id: Uuid,
    pub patient_id: Uuid,
    /// The free-text term as asserted — may legitimately be vague ("little white pill").
    pub term: String,
    /// The ADR-0059 coded display name, when the drug has been coded.
    pub coding_display: Option<String>,
    pub formulation: Option<String>,
    pub dose_amount: Option<String>,
    pub dose_unit: Option<String>,
    pub sig: Option<String>,
    pub started_value: Option<String>,
    pub started_precision: Option<String>,
    pub status: MedicationStatus,
    pub members: Vec<MemberVouch>,
    /// This group shares a duplicate key with another un-reconciled group
    /// (`patient_medication_reconciliation_flag`). Advisory worklist, never auto-resolved.
    pub reconciliation_flagged: bool,
    /// Two different drug anchors inside one reconciled group
    /// (`medication_group_coding_conflict`) — a possible mis-reconciliation.
    pub coding_conflict: bool,
}
