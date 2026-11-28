//! What a chart is (ADR-0115), for every command that prints about ONE patient.
//!
//! `patient-search` shows a chart's kind on its row (`candidate_text.rs`). The commands
//! that take a chart id and print its medicines, its confidentiality grades, its safety
//! signals or its duplicate-check state name no row to mark, so they say it once, first,
//! on a line of its own. Without that line an operator reading a synthetic chart's
//! medication list sees nothing to tell it from a person's.
//!
//! One reader and one sentence per kind, here, so the four commands cannot word it four
//! ways (ADR-0115 decision 7: shown wherever the patient is shown).

use cairn_patient_search::PatientKind;
use tokio_postgres::GenericClient;
use uuid::Uuid;

/// Ask the floor what `patient` is. The ONE function that knows is `cairn_chart_synthetic`
/// (db/088); nothing in this crate reads the registration's column for itself.
pub async fn chart_kind<C: GenericClient + Sync>(
    client: &C,
    patient: Uuid,
) -> anyhow::Result<PatientKind> {
    let word: String = client
        .query_one(
            "SELECT cairn_chart_synthetic($1::text::uuid)",
            &[&patient.to_string()],
        )
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "reading whether the chart is synthetic: {}",
                crate::db_diagnosis::legible_db_error(&e)
            )
        })?
        .get(0);
    Ok(PatientKind::from_floor(&word))
}

/// The line a command prints before anything else about a chart, or `None` for a chart of
/// a real person, which needs no line. **Pure.**
///
/// A real chart prints nothing on purpose: a line that is always there is not read, and the
/// one that matters would be lost among them (the same reason the window has no "REAL DATA"
/// band).
///
/// `Unknown` DOES print. This node holds no registration for the chart, so it cannot say
/// the patient is a person, and saying nothing would let the output read as if it had
/// (principle 4: an acknowledged unknown, never a precise untruth).
pub fn kind_banner(kind: PatientKind) -> Option<&'static str> {
    match kind {
        PatientKind::Synthetic => Some("SYNTHETIC patient: not a real person."),
        PatientKind::Unknown => Some(
            "Registration not received here: this node cannot say whether this chart is of \
             a real person.",
        ),
        PatientKind::Real => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_synthetic_chart_and_an_unknown_one_each_say_so_and_a_real_one_says_nothing() {
        assert_eq!(
            kind_banner(PatientKind::Synthetic),
            Some("SYNTHETIC patient: not a real person.")
        );
        let unknown = kind_banner(PatientKind::Unknown).expect("unknown is never silent");
        assert!(unknown.starts_with("Registration not received here"));
        assert!(
            !unknown.contains("SYNTHETIC"),
            "unknown must not be worded as synthetic either: {unknown}"
        );
        assert_eq!(kind_banner(PatientKind::Real), None);
    }

    /// The registration's own success line and this banner must read as the same statement,
    /// so an operator recognises it: both begin "SYNTHETIC patient: not a real person."
    #[test]
    fn the_banner_matches_the_registrations_own_wording() {
        let said_at_registration =
            crate::patient::register::registered_synthetic_patient_lines(Uuid::nil(), Uuid::nil());
        let banner = kind_banner(PatientKind::Synthetic).unwrap();
        assert!(
            said_at_registration.lines().any(|l| l.starts_with(banner)),
            "{said_at_registration}"
        );
    }
}
