//! What `med_chart.rs`'s Sign writes and answers: the assertion one typed line becomes
//! ([`charted_input`]), and the answer once it landed ([`charted`]). Pure; kept apart so
//! `med_chart.rs` holds the commands and their refusals.
use crate::allergy_check::CodingArg;
use crate::chart_set::ChartPane;
use cairn_event::medication::{ChartedAs, SubstanceCoding};
use cairn_node::medication::line::Directions;
use cairn_node::medication::AssertMedicationInput;

/// What Sign answers once the line is signed: what happened, and the medication list read
/// again when that read worked. A Sign that signed nothing answers an `ErrorView` instead.
#[derive(Debug, serde::Serialize)]
pub struct MedChartWrite {
    pub outcome: String,
    pub pane: Option<ChartPane>,
}

/// What a Sign that LANDED answers: always an Ok (as `allergies::written`). With the list read
/// again, the pane; without it, a sentence saying it was signed and how to see it — never an
/// error, which the window would offer to retry and so chart the drug twice. Pure.
pub(crate) fn charted(reread: Result<ChartPane, String>) -> MedChartWrite {
    match reread {
        Ok(pane) => MedChartWrite {
            outcome: "Charted.".into(),
            pane: Some(pane),
        },
        Err(e) => MedChartWrite {
            outcome: format!(
                "Signed — but the medication list could not be read again ({e}). \
                 Use “Read this chart again”."
            ),
            pane: None,
        },
    }
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
    }
}

pub(crate) fn substance(c: &CodingArg) -> SubstanceCoding<'_> {
    SubstanceCoding {
        system: &c.system,
        code: &c.code,
        display: &c.display,
    }
}
