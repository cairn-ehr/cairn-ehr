//! Fixture-backed ports — what `--mock` runs on.
//!
//! This is a **shipped mode, not a toy**: it is what the operator accessibility pass and the
//! timing runbook use on a laptop, with no database anywhere. So the fixtures have to
//! exercise the shapes that actually break things (see [`fixtures`]), and the ports have to
//! behave honestly rather than conveniently.
//!
//! [`ClinicalData`] lives here; the two funnel ports are implemented in the private
//! `funnel` submodule, whose doc explains — at length, and deliberately — why the fixture
//! matching rule is **not** `db/046`'s and must not be generalised from.

pub mod fixtures;
mod funnel;

use crate::port::{ClinicalData, DataError, Demographics, NoteRef};
use cairn_gui_tab::PatientRef;
use fixtures::{FixturePatient, FIXTURE_UUID};
use std::sync::Mutex;
use uuid::Uuid;

/// The fixture population, plus the one cross-reference note the note→pane demo needs.
///
/// # Why the population is behind a `Mutex`
///
/// Registering in `--mock` mints a patient into this set so the next browse finds it — the
/// only thing that makes the *browse → nothing fits → register → prompt → commit* walk mean
/// anything. The ports take `&self` (they are read-shaped, and the real implementations will
/// be), so the write needs interior mutability. A `std::sync::Mutex` rather than a `RefCell`
/// because the window awaits these from a multi-threaded runtime; the lock is never held
/// across an `await` (see the `funnel` submodule).
pub struct MockData {
    patients: Mutex<Vec<FixturePatient>>,
    note_refs: Vec<NoteRef>,
    /// The failure the NEXT funnel-port call returns instead of its fixture answer.
    /// **One-shot**: taking it clears it.
    ///
    /// # Why a mock that can fail is not a contradiction
    ///
    /// `--mock` is the mode the operator-accessibility pass and the §1.2 timing runbook run
    /// in, and since #648 a failed call is *three* facts rather than two: a floor verdict
    /// (`Refused` — *"this cannot succeed as typed"*) needs different words on screen than an
    /// outage (`Unavailable` — *"try again"*). Those two sentences are the window's to write,
    /// and without this slot neither is producible under `--mock` at all — the only coverage
    /// would be `cairn-gui-live`'s DB-gated suite, which tests the *classification* and
    /// renders nothing. See [#660](https://github.com/cairn-ehr/cairn-ehr/issues/660).
    ///
    /// One slot shared by both ports, not one each: a test arms it immediately before the call
    /// it means to fail, and two slots would let it arm the wrong one and pass for the wrong
    /// reason.
    next_failure: Mutex<Option<DataError>>,
}

impl MockData {
    pub fn with_fixtures() -> Self {
        Self {
            patients: Mutex::new(fixtures::starting_population()),
            note_refs: vec![NoteRef {
                id: "xray-2026-07-01".to_string(),
                one_line: "Chest X-ray 2026-07-01 — no acute abnormality".to_string(),
            }],
            next_failure: Mutex::new(None),
        }
    }

    /// Arm the next funnel-port call to fail with `e` instead of answering from fixtures.
    ///
    /// `&self`, not `&mut self`: both ports take `&self`, and this type already keeps its
    /// mutable state behind a `Mutex` for exactly that reason. A `&mut self` setter would
    /// force a caller to juggle a mutable binding across a borrow the port holds.
    pub fn fail_next(&self, e: DataError) {
        *self.next_failure.lock().expect("the armed failure") = Some(e);
    }

    /// Take the armed failure if there is one, clearing it. **One-shot.**
    ///
    /// Private: arming is a caller's affordance, consuming is the ports' business. The ports
    /// call this *inside* their async bodies, never when the future is built — see the note
    /// above their impls.
    fn armed_failure(&self) -> Option<DataError> {
        self.next_failure.lock().expect("the armed failure").take()
    }

    /// Look one patient up by id, cloning it out so no lock guard escapes.
    ///
    /// An unparseable id is simply not found, which is the honest answer: no chart can have
    /// it. Parsing rather than comparing strings also makes the lookup insensitive to the
    /// hyphenation and case a caller happens to use.
    fn find(&self, patient_uuid: &str) -> Option<FixturePatient> {
        let wanted = Uuid::parse_str(patient_uuid).ok()?;
        self.patients
            .lock()
            .expect("fixture population")
            .iter()
            .find(|p| p.uuid == wanted)
            .cloned()
    }
}

impl ClinicalData for MockData {
    fn demographics(&self, patient_uuid: &str) -> Result<Demographics, DataError> {
        let patient = self.find(patient_uuid).ok_or(DataError::NotFound)?;
        Ok(Demographics {
            patient: PatientRef {
                uuid: patient.uuid.to_string(),
                display_name: patient.display_name,
            },
            sex: patient.sex,
            birth_date: patient.birth_date,
            identifiers: patient.identifiers,
        })
    }

    fn note_refs(&self, patient_uuid: &str) -> Result<Vec<NoteRef>, DataError> {
        // A KNOWN patient with no notes gets an empty list; only an UNKNOWN one is
        // `NotFound`. Those are different answers — "this chart has no cross-references" is
        // a real clinical state, and collapsing it into "no such patient" would make every
        // fixture but one look like it did not exist.
        let patient = self.find(patient_uuid).ok_or(DataError::NotFound)?;
        // Compare the PARSED ids, so the one fixture that carries notes is found however the
        // caller spelled its uuid.
        if patient.uuid.to_string() == FIXTURE_UUID {
            Ok(self.note_refs.clone())
        } else {
            Ok(Vec::new())
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
    use cairn_medication_view::{MedicationStatus, VouchState};

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

    #[test]
    fn a_known_patient_with_no_notes_has_none_rather_than_not_existing() {
        // "This chart has no cross-references" and "there is no such chart" are different
        // answers, and the second one about a patient who IS on file is a lie the window
        // would render as an error.
        let data = MockData::with_fixtures();
        let other = data
            .demographics("00000000-0000-0000-0000-0000000000bb")
            .expect("a second fixture patient exists now");
        assert!(data.note_refs(&other.patient.uuid).unwrap().is_empty());
        assert!(matches!(
            data.note_refs("00000000-0000-0000-0000-000000000099"),
            Err(DataError::NotFound)
        ));
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
