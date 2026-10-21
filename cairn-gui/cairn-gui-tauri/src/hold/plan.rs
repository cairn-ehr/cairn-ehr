//! Pure decisions behind the window's Withhold and Release gestures (#832): which thread a W is
//! written on, and which holds a Release ends. No database here, so the safety rules are
//! tested directly (`plan_tests.rs`). The model is `chart_set::cease_plan`.
use cairn_medication_view::MedicationRow;
use uuid::Uuid;

/// The thread a Withhold on `row` is written on, and the chart that thread's statement is on.
///
/// Every thread is written on its OWN chart (ADR-0076 decision 2), so the chart is read off the
/// member, never assumed to be the opened one. An ordinary line is withheld on its
/// representative thread (`row.group_id`, the ADR-0047 min-UUID collapse): one hold per line,
/// whichever linked chart that thread lives on. A wrong-chart HAZARD line may carry another
/// person's drug, so "withhold this" can only be trusted for the patient in front of the
/// clinician: the representative thread is used only if it is on the opened chart, else the
/// first thread that is, else the gesture is refused with a sentence (never silently written
/// onto a chart the node doubts).
pub fn withhold_target(row: &MedicationRow, opened: Uuid) -> Result<(Uuid, Uuid), String> {
    let hazard = row.is_wrong_chart_hazard();
    let representative = row.members.iter().find(|m| m.medication_id == row.group_id);
    if let Some(m) = representative {
        if !hazard || m.patient_id == opened {
            return Ok((m.medication_id, m.patient_id));
        }
    }
    if !hazard {
        // Not a hazard, yet the line's own thread is not a member: say that, not the hazard
        // sentence, and never guess another thread (a refusal names its real cause, §9.6).
        return Err(format!(
            "this line's own record (thread {}) is not among its records on this chart — \
             reopen the chart and try again",
            row.group_id
        ));
    }
    match row.members.iter().find(|m| m.patient_id == opened) {
        Some(m) => Ok((m.medication_id, m.patient_id)),
        None => Err(
            "this line may carry another person's drug and has no record on this \
                     chart — withhold it from its own chart"
                .to_string(),
        ),
    }
}

/// What a Release from the opened chart writes, holds back, and finds already gone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleasePlan {
    /// `(thread, its own chart, the withhold event ids to end)`, in member order — one write
    /// per thread (ADR-0076 decision 2).
    pub write: Vec<(Uuid, Uuid, Vec<Uuid>)>,
    /// Holds deliberately NOT released, each with the sentence the report shows for it.
    pub held_back: Vec<String>,
    /// Shown hold ids no longer on the line (ended or released meanwhile).
    pub gone: Vec<Uuid>,
}

/// Plan a Release of the holds the clinician was SHOWN (`shown`), from the chart `opened`.
///
/// A release never ends a hold its author was not shown (ADR-0081 decision 4): a hold that
/// started after the screen was drawn stays in force, so only ids in `shown` are ever written
/// (a repeated id counts once). Shown ids no longer on the line are named `gone`. On a
/// wrong-chart hazard line only the opened chart's threads are written; the others are held
/// back and NAMED, as `cease_plan` does. A hold on a thread that is not a member of the line
/// has no chart to be written on, so it is held back too, never guessed.
pub fn release_plan(row: &MedicationRow, shown: &[Uuid], opened: Uuid) -> ReleasePlan {
    let hazard = row.is_wrong_chart_hazard();
    let mut plan = ReleasePlan {
        write: vec![],
        held_back: vec![],
        gone: vec![],
    };
    for id in shown {
        if !row.holds.iter().any(|h| h.withhold_event_id == *id) && !plan.gone.contains(id) {
            plan.gone.push(*id);
        }
    }
    // Shown holds still on the line, each once (iterating the line's holds dedupes `shown`).
    let live: Vec<_> = row
        .holds
        .iter()
        .filter(|h| shown.contains(&h.withhold_event_id))
        .collect();
    for m in &row.members {
        let ids: Vec<Uuid> = live
            .iter()
            .filter(|h| h.medication_id == m.medication_id)
            .map(|h| h.withhold_event_id)
            .collect();
        if ids.is_empty() {
            continue;
        }
        if hazard && m.patient_id != opened {
            for id in ids {
                plan.held_back.push(format!(
                    "hold {id} on thread {} on chart {}: NOT released from here — this line \
                     may carry another person's drug; release it from its own chart",
                    m.medication_id, m.patient_id
                ));
            }
        } else {
            plan.write.push((m.medication_id, m.patient_id, ids));
        }
    }
    for h in live {
        if !row
            .members
            .iter()
            .any(|m| m.medication_id == h.medication_id)
        {
            plan.held_back.push(format!(
                "hold {} on thread {}: NOT released — that thread is not part of this line",
                h.withhold_event_id, h.medication_id
            ));
        }
    }
    plan
}

#[cfg(test)]
#[path = "plan_tests.rs"]
mod tests;
