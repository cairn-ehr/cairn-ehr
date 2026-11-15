//! What one typed line signs and what a medication write answers: the assertion the line becomes
//! ([`charted_input`], for the list's Sign), its pick's coding ([`substance`]), and the answer
//! shape of the window's single-act medication writes ([`MedChartWrite`]: Given, Entered in error,
//! "checked: not current"). Pure.
use crate::allergy_check::CodingArg;
use crate::chart_set::ChartPane;
use cairn_event::medication::{ChartedAs, SubstanceCoding};
use cairn_node::medication::line::Directions;
use cairn_node::medication::AssertMedicationInput;

/// What a single-act medication write answers once it landed: what happened, and the medication
/// list read again when that read worked. A write that wrote nothing answers an `ErrorView`
/// instead.
#[derive(Debug, serde::Serialize)]
pub struct MedChartWrite {
    pub outcome: String,
    pub pane: Option<ChartPane>,
}

/// The assertion the line signs: the directions as typed are the `sig` (blank: none), and the
/// read-back's dose and form fill their fields only where the parse was certain (`d` leaves the
/// rest `None`). The press sets the source and the start: Start is the clinician, today (`day`);
/// Already taking is the patient, start not known. Pure (the caller passes today).
pub(crate) fn charted_input<'a>(
    term: &'a str,
    coding: Option<SubstanceCoding<'a>>,
    d: &'a Directions,
    directions: &'a str,
    charted_as: ChartedAs,
    today: &'a str,
) -> AssertMedicationInput<'a> {
    let start = charted_as == ChartedAs::StartedHere;
    AssertMedicationInput {
        term,
        coding,
        formulation: d.form.as_deref(),
        dose_amount: d.dose_amount.as_deref(),
        dose_unit: d.dose_unit.as_deref(),
        sig: Some(directions).filter(|s| !s.trim().is_empty()),
        info_source: if start {
            "clinician-observed"
        } else {
            "patient-reported"
        },
        started: start.then_some(today),
        started_precision: start.then_some("day"),
        source_note: None,
    }
}

pub(crate) fn substance(c: &CodingArg) -> SubstanceCoding<'_> {
    SubstanceCoding {
        system: &c.system,
        code: &c.code,
        display: &c.display,
    }
}
