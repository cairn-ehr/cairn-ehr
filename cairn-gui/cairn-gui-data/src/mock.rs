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

    fn medications(
        &self,
        patient_uuid: &str,
    ) -> Result<cairn_medication_view::PatientMedicationList, DataError> {
        // Any other patient has an EMPTY chart rather than a NotFound: an empty chart is a
        // real clinical state and the window must render it honestly. That deliberately
        // differs from `demographics` above, where an unknown patient really is absent — a
        // patient with no medications recorded still exists.
        if patient_uuid == cairn_medication_view::fixtures::FIXTURE_PATIENT {
            Ok(cairn_medication_view::fixtures::sample_chart())
        } else {
            Ok(cairn_medication_view::PatientMedicationList::empty())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::port::ClinicalData;
    use cairn_medication_view::{MedicationStatus, VouchState};

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

    /// The mock exists so the window runs with no database — what the operator
    /// accessibility pass and the timing runbook need on a laptop. It must therefore
    /// exercise the interesting shapes, not one bland row.
    #[test]
    fn the_fixture_chart_covers_absent_fresh_stale_and_ceased() {
        let chart = MockData::with_fixtures()
            .medications(cairn_medication_view::fixtures::FIXTURE_PATIENT)
            .expect("the fixture patient has a chart");
        assert!(chart.rows.len() >= 4, "the fixture must show several drugs");

        let vouches: Vec<&VouchState> = chart
            .rows
            .iter()
            .flat_map(|r| r.members.iter().map(|m| &m.vouch))
            .collect();
        assert!(vouches.iter().any(|v| matches!(v, VouchState::Absent)));
        assert!(vouches
            .iter()
            .any(|v| matches!(v, VouchState::Fresh { .. })));
        assert!(vouches
            .iter()
            .any(|v| matches!(v, VouchState::Stale { .. })));
        assert!(chart
            .rows
            .iter()
            .any(|r| r.status == MedicationStatus::Ceased));
    }

    /// ADR-0060 decision 2: partial completion is reported, never implied. The mock must
    /// carry the "cannot be shown" signal too, or the `--mock` window silently exercises a
    /// happier path than the real one and the operator pass never sees the warning.
    #[test]
    fn the_fixture_chart_reports_what_it_cannot_show() {
        let chart = MockData::with_fixtures()
            .medications(cairn_medication_view::fixtures::FIXTURE_PATIENT)
            .unwrap();
        assert!(
            !chart.groups_missing_from_chart.is_empty(),
            "the fixture must exercise the incomplete-chart report"
        );
        assert!(!chart.separation_targets.is_empty(), "…and its remedy");
    }

    /// An unknown chart is EMPTY, not an error — an empty chart is a real clinical state,
    /// and it is a different answer from `demographics`, where an unknown patient is
    /// genuinely NotFound. A patient with no medications recorded still exists.
    #[test]
    fn an_unknown_patient_has_an_empty_chart() {
        assert!(MockData::with_fixtures()
            .medications("11111111-1111-1111-1111-111111111111")
            .unwrap()
            .rows
            .is_empty());
    }
}
