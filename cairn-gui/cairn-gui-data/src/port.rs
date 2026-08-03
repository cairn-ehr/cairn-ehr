//! The single read-port the UI uses. Real impl → node's native-API client (later
//! slice); mock impl → fixtures (this slice). The UI never writes/signs (§9.6).
use cairn_gui_tab::PatientRef;

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
