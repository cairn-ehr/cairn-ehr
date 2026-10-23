//! Lines on ANOTHER chart that carry something recorded for this one (#848).
//!
//! A medication event names the chart it was recorded for, and it is projected onto its thread
//! whatever chart that thread is on (converge-and-flag, db/031: a node-local veto would fork
//! the event set). So an event recorded for this patient can sit on a line this chart's list
//! never shows:
//! - a later-HLC assert of the same thread on another chart MOVED the thread there (the
//!   statement overlay keeps one chart per thread), and the drug left this list with no word;
//! - a dose, dose correction, release or W recorded for this patient landed on another chart's
//!   thread (a mistyped thread id, a buggy client).
//!
//! The other chart's list shows that line as a `patient_conflict` hazard (#836). This is the
//! half that tells THIS chart: one entry per such line, named and placed, with a remedy. A line
//! already on this list is never repeated here (its hazard is said on the line itself).
use crate::row::MedicationStatus;
use serde::Serialize;
use uuid::Uuid;

/// One line on another chart carrying an event recorded for a chart in this list's set.
///
/// Serialize only, like `MedicationRow`: nothing reads it back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ElsewhereLine {
    /// The line (medication group) the event sits on.
    pub group_id: Uuid,
    /// The drug as the line shows it (coded display, else the asserted term). `None` when the
    /// line has no readable statement here: no data is never a name (principle 4).
    pub name: Option<String>,
    /// Whether that line is current or stopped; `None` when neither list view emits it.
    pub status: Option<MedicationStatus>,
    /// The charts the line is recorded on now, sorted. Empty when the node knows none.
    pub charts: Vec<Uuid>,
}

/// What to do about an [`ElsewhereLine`] — worded once, for every renderer.
///
/// WHY "RECORD IT AGAIN". The event cannot be removed (append-only) and no verb re-files it to
/// another thread, so — as on a paper chart where an entry went on the wrong patient's sheet —
/// it is written again where it belongs. The same remedy as `CONFLICT_INSTRUCTION`, seen from
/// the chart the event was meant for. Whether the drug is this patient's at all is a clinical
/// question the record cannot answer, so the sentence asks it rather than assuming either way.
/// Conditioned on the drug NOT being on this list already (PR review): a W or dose misfiled
/// onto another chart's metoprolol, for a patient whose own metoprolol line is right here, must
/// be re-recorded on that line, never answered with a second metoprolol line (a double-dose
/// reading hazard).
pub const ELSEWHERE_INSTRUCTION: &str =
    "Something recorded for this patient (the drug itself, or a dose, stop, release or W on it) \
     sits on that line, so it does not show on this list. The drug may be this patient's, now \
     recorded on the wrong chart, or another patient's with this patient's entry on it by \
     mistake. Check with the patient and the prescriber whether this patient takes it. If they \
     do and it is not already on this list, record it again on this chart; if it is already on \
     this list, record the dose, stop or W again on that line here. The entries stay on record; \
     they cannot be removed.";

/// The report every renderer shows for `lines`, or `None` when there are none. **Pure.**
pub fn elsewhere_report(lines: &[ElsewhereLine]) -> Option<String> {
    if lines.is_empty() {
        return None;
    }
    let items: Vec<String> = lines.iter().map(describe).collect();
    Some(format!(
        "{} medication line(s) on another chart hold something recorded for this patient: {}. {}",
        lines.len(),
        items.join("; "),
        ELSEWHERE_INSTRUCTION
    ))
}

/// One line's description: name, status, where it is, and its id (for tracing it).
fn describe(line: &ElsewhereLine) -> String {
    let name = line
        .name
        .as_deref()
        .unwrap_or("a drug this node cannot name");
    let status = match line.status {
        Some(MedicationStatus::Active) => "current",
        Some(MedicationStatus::Ceased) => "stopped",
        None => "status not read",
    };
    let charts = if line.charts.is_empty() {
        "a chart not read here".to_string()
    } else {
        let ids: Vec<String> = line.charts.iter().map(Uuid::to_string).collect();
        format!("chart {}", ids.join(", "))
    };
    format!("{name} ({status}, on {charts}; line {})", line.group_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(
        name: Option<&str>,
        status: Option<MedicationStatus>,
        charts: &[u128],
    ) -> ElsewhereLine {
        ElsewhereLine {
            group_id: Uuid::from_u128(9),
            name: name.map(str::to_string),
            status,
            charts: charts.iter().copied().map(Uuid::from_u128).collect(),
        }
    }

    #[test]
    fn no_line_elsewhere_says_nothing() {
        assert_eq!(elsewhere_report(&[]), None);
    }

    /// The drug is NAMED and PLACED, and the remedy follows: a count alone would leave the
    /// clinician asking which drug, and on which chart.
    #[test]
    fn a_line_is_named_placed_and_given_its_remedy() {
        let text =
            elsewhere_report(&[line(Some("apixaban"), Some(MedicationStatus::Active), &[2])])
                .unwrap();
        assert!(
            text.starts_with("1 medication line(s) on another chart"),
            "{text}"
        );
        assert!(text.contains("apixaban (current, on chart "), "{text}");
        assert!(text.contains(&Uuid::from_u128(2).to_string()), "{text}");
        assert!(text.contains(&Uuid::from_u128(9).to_string()), "{text}");
        assert!(text.ends_with(ELSEWHERE_INSTRUCTION), "{text}");
    }

    /// The remedy must not double a line (PR review A): when the drug IS on this list, it is the
    /// misfiled dose, stop or W that is recorded again there, never a second line for the drug.
    #[test]
    fn the_remedy_never_asks_for_a_second_line_of_a_listed_drug() {
        assert!(ELSEWHERE_INSTRUCTION.contains("not already on this list"));
        assert!(ELSEWHERE_INSTRUCTION.contains("on that line"));
    }

    /// What the node could not read is said as such, never left blank or guessed.
    #[test]
    fn an_unread_name_status_or_chart_is_said_as_unread() {
        let text = elsewhere_report(&[line(None, None, &[])]).unwrap();
        assert!(text.contains("a drug this node cannot name"), "{text}");
        assert!(text.contains("status not read"), "{text}");
        assert!(text.contains("a chart not read here"), "{text}");
        let stopped = elsewhere_report(&[line(Some("x"), Some(MedicationStatus::Ceased), &[2])]);
        assert!(stopped.unwrap().contains("x (stopped,"));
    }
}
