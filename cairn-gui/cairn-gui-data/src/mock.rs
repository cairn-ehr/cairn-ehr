//! Fixture-backed ClinicalData for slice 1 (no node). One patient with a
//! multi-script name to also feed the Spike 0004 shaping check.
use crate::port::{ClinicalData, DataError, Demographics, NoteRef};
use cairn_gui_tab::PatientRef;

const FIXTURE_UUID: &str = "00000000-0000-0000-0000-0000000000aa";

pub struct MockData {
    demographics: Demographics,
    note_refs: Vec<NoteRef>,
}

impl MockData {
    pub fn with_fixtures() -> Self {
        let patient = PatientRef {
            uuid: FIXTURE_UUID.to_string(),
            // Latin / Arabic / Devanagari / Han in one label feeds the IME/shaping pass.
            display_name: "Amina أمينة अमीना 阿明娜".to_string(),
        };
        Self {
            demographics: Demographics {
                patient,
                sex: "female".to_string(),
                birth_date: "1984-03-02".to_string(),
                identifiers: vec![
                    ("MRN".to_string(), "12345".to_string()),
                    ("National".to_string(), "QLD-998877".to_string()),
                ],
            },
            note_refs: vec![NoteRef {
                id: "xray-2026-07-01".to_string(),
                one_line: "Chest X-ray 2026-07-01 — no acute abnormality".to_string(),
            }],
        }
    }
}

impl ClinicalData for MockData {
    fn demographics(&self, patient_uuid: &str) -> Result<Demographics, DataError> {
        if patient_uuid == self.demographics.patient.uuid {
            Ok(self.demographics.clone())
        } else {
            Err(DataError::NotFound)
        }
    }

    fn note_refs(&self, patient_uuid: &str) -> Result<Vec<NoteRef>, DataError> {
        if patient_uuid == self.demographics.patient.uuid {
            Ok(self.note_refs.clone())
        } else {
            Err(DataError::NotFound)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::port::ClinicalData;

    const FIXTURE_UUID: &str = "00000000-0000-0000-0000-0000000000aa";

    #[test]
    fn mock_returns_fixture_demographics() {
        let data = MockData::with_fixtures();
        let d = data
            .demographics(FIXTURE_UUID)
            .expect("fixture patient exists");
        assert_eq!(d.patient.uuid, FIXTURE_UUID);
        assert!(
            !d.identifiers.is_empty(),
            "fixture has at least one identifier"
        );
    }

    #[test]
    fn mock_unknown_patient_is_not_found() {
        let data = MockData::with_fixtures();
        assert!(matches!(
            data.demographics("no-such"),
            Err(crate::port::DataError::NotFound)
        ));
    }

    #[test]
    fn mock_has_a_cross_reference_note() {
        let data = MockData::with_fixtures();
        let refs = data.note_refs(FIXTURE_UUID).unwrap();
        assert!(
            !refs.is_empty(),
            "fixture provides a cross-reference for the note→pane demo"
        );
    }
}
