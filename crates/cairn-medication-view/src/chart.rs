//! One patient's whole chart — the rows, and what the node knows is MISSING from them.
//!
//! WHY THIS LIVES IN THE SHARED CRATE AND NOT IN THE NODE. It was born in
//! `cairn-node`'s read path, where it had exactly one consumer (the CLI). The med-list
//! window is the second, and it needs the *same* three things the CLI needs: the rows, the
//! groups the chart cannot display, and the thread ids that make the repair runnable.
//!
//! That is not a convenience. ADR-0060 decision 2 says partial completion must be
//! **reported, never implied** — so a renderer that receives only `rows` is structurally
//! incapable of obeying it: it cannot warn about a drug it was never handed. Putting the
//! whole chart here means the window and the CLI answer *"what is this chart missing?"*
//! from one definition, the same reason `sign_off_targets` lives beside it.
//!
//! Pure: no database driver, no GUI toolkit. `cairn-node` re-exports every item from
//! `medication::read`, so its old paths still resolve.
use crate::row::MedicationRow;
use serde::Serialize;
use std::collections::BTreeMap;
use uuid::Uuid;

/// The one sentence that tells an operator how to clear a cross-patient group.
///
/// It is a const, not several hand-written copies, because it is quoted by every
/// user-facing message about the hazard (the CLI's withheld-line warning, the CLI's chart
/// warnings, and now the window's row warning). A remedy that drifts between them is worse
/// than no remedy: the operator learns to distrust whichever one they read second.
///
/// WHY `medication-separate` AND WHY WITHOUT `--attest-as`. Separation is the repair
/// primitive for exactly this inconsistency and the db/033 door deliberately never blocks
/// it (unlike reconciliation, which refuses a cross-patient link at local author time). But
/// the verb takes a SINGLE `patient` argument that it stamps onto both threads' vouches
/// when `--attest-as` is given — and for a cross-patient group no single patient is right
/// for both, so attesting here would file a vouch under the wrong chart for one of them.
/// Device-additive separation carries no such claim, so that is what we tell them to run.
pub const SEPARATION_INSTRUCTION: &str =
    "Clear it with `medication-separate <patient> <thread_a> <thread_b>`, naming BOTH member \
     threads listed below — run it WITHOUT `--attest-as`, because the threads belong to \
     different patients and a vouch would record the wrong chart for one of them. Separation \
     is deliberately never blocked (db/033).";

/// A patient's chart, plus what the node knows is MISSING from it.
///
/// `rows` is what the clinician sees. `groups_missing_from_chart` is a safety signal that
/// exists BECAUSE a reconciled group can span more than one patient (issue #334): the
/// group then displays on only ONE patient's chart, so a patient whose thread was pulled
/// into such a group can have locally-known medication content the node simply cannot show
/// here. Non-empty means this chart is INCOMPLETE, not merely sparse. It does **not** stop
/// the rest of the chart being read or signed (ADR-0060) — it is something every renderer
/// must say out loud.
///
/// WHAT IT DOES NOT CATCH. The signal is derived from `medication_thread_group`, which
/// db/033 drives from `medication_statement` alone. A thread known locally ONLY through an
/// orphan cessation — a stop event that arrived before the statement it stops, the
/// late-arrival case db/033 calls out — has no `medication_thread_group` row, so it
/// contributes nothing here and a group holding only such threads still escapes detection.
/// That thread is invisible to the read path with or without a group, so this is a
/// pre-existing limit of the projection rather than a gap this signal introduced; it is
/// recorded here so nobody reads `groups_missing_from_chart` as a total guarantee of
/// completeness. It is a guarantee about *displayable* content only.
///
/// `separation_targets` is what makes the other two ACTIONABLE — see its own comment.
#[derive(Debug, Clone, Serialize)]
pub struct PatientMedicationList {
    pub rows: Vec<MedicationRow>,
    pub groups_missing_from_chart: Vec<Uuid>,
    /// For each group this chart flags as a cross-patient hazard — whether it is displayed
    /// here (`MedicationRow::cross_patient`) or invisible here
    /// (`groups_missing_from_chart`) — the group's FULL member-thread list, including
    /// members belonging to OTHER patients. Sorted, and empty in normal operation.
    ///
    /// WHY THIS EXISTS (#338 review finding 1). Every message about a cross-patient group
    /// points the operator at `medication-separate`, which takes TWO THREAD IDS. Everything
    /// else this struct carries is scoped to one patient — `rows` shows only groups that
    /// display under this patient, and the node's vouch read filters members by
    /// `medication_thread_group.patient_id` — so the *other* patient's thread appears
    /// nowhere. Without this field the node names a remedy whose arguments it never shows,
    /// and the only way out is raw SQL. The cross-patient member is deliberately the one
    /// piece of another chart's data this read path surfaces: it is a bare thread id with
    /// no clinical content attached, and it is the minimum needed to repair a wrong-chart
    /// link the node itself is complaining about.
    pub separation_targets: BTreeMap<Uuid, Vec<Uuid>>,
}

impl PatientMedicationList {
    /// An empty chart. Not an error state: a patient with nothing recorded is a real
    /// clinical situation, and it is also what a fixture-mode window shows for any patient
    /// other than the fixture one.
    pub fn empty() -> Self {
        Self {
            rows: vec![],
            groups_missing_from_chart: vec![],
            separation_targets: BTreeMap::new(),
        }
    }
}

/// Render hazardous groups as `group <id> (member threads: <a>, <b>)` — the shape EVERY
/// cross-patient message uses, written once.
///
/// The member threads are the whole point (#338 review finding 1): the remedy those
/// messages name (`medication-separate`, see [`SEPARATION_INSTRUCTION`]) takes two THREAD
/// ids, so a message printing only the group id sends the operator looking for arguments
/// the node never shows them. A group with no locally-known membership degrades honestly to
/// "unknown locally" rather than inventing a list — the same acknowledged-uncertainty
/// direction as the rest of this model.
///
/// Public because four call sites render it — the CLI's withheld-line warning, the CLI's
/// chart warnings, the sign-off report, and the window's per-row warning. Four hand-written
/// copies of one repair instruction is how they drift.
pub fn format_hazard_groups(
    groups: &[Uuid],
    separation_targets: &BTreeMap<Uuid, Vec<Uuid>>,
) -> String {
    groups
        .iter()
        .map(|group| match separation_targets.get(group) {
            Some(members) if !members.is_empty() => {
                let rendered: Vec<String> = members.iter().map(|m| m.to_string()).collect();
                format!("group {group} (member threads: {})", rendered.join(", "))
            }
            _ => format!("group {group} (member threads: unknown locally)"),
        })
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uid(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    /// The case the whole `separation_targets` field exists for: the rendered message must
    /// carry the THREAD ids, because those are what `medication-separate` takes.
    #[test]
    fn a_hazard_group_renders_its_member_threads() {
        let targets = BTreeMap::from([(uid(1), vec![uid(1), uid(2)])]);
        let rendered = format_hazard_groups(&[uid(1)], &targets);
        assert!(
            rendered.contains(&uid(1).to_string()) && rendered.contains(&uid(2).to_string()),
            "both member threads must appear: {rendered}"
        );
    }

    /// Acknowledged uncertainty (principle 4): a group whose membership this node cannot
    /// see says so, rather than rendering an empty list that reads as "no other threads".
    #[test]
    fn a_group_with_no_known_members_says_so() {
        let rendered = format_hazard_groups(&[uid(7)], &BTreeMap::new());
        assert!(
            rendered.contains("unknown locally"),
            "must not imply the group is a singleton: {rendered}"
        );
        assert!(rendered.contains(&uid(7).to_string()));
    }

    /// An empty membership vector is the same uncertainty as an absent key — neither may
    /// render as a confident empty list.
    #[test]
    fn an_empty_member_list_degrades_like_a_missing_one() {
        let targets = BTreeMap::from([(uid(3), vec![])]);
        assert!(format_hazard_groups(&[uid(3)], &targets).contains("unknown locally"));
    }

    #[test]
    fn several_hazard_groups_render_together() {
        let targets = BTreeMap::from([
            (uid(1), vec![uid(1), uid(2)]),
            (uid(5), vec![uid(5), uid(6)]),
        ]);
        let rendered = format_hazard_groups(&[uid(1), uid(5)], &targets);
        assert_eq!(rendered.matches("group ").count(), 2, "{rendered}");
    }

    #[test]
    fn no_hazard_groups_render_to_nothing() {
        assert_eq!(format_hazard_groups(&[], &BTreeMap::new()), "");
    }

    /// The repair instruction must actually name the verb and its two-thread shape — a
    /// message that says "separate it" without saying how is what finding 1 was about.
    #[test]
    fn the_separation_instruction_names_the_verb_and_both_arguments() {
        assert!(SEPARATION_INSTRUCTION.contains("medication-separate"));
        assert!(SEPARATION_INSTRUCTION.contains("thread_a"));
        assert!(SEPARATION_INSTRUCTION.contains("thread_b"));
        // Attesting a cross-patient separation would file a vouch under the wrong chart
        // for one of the two threads; the instruction must warn against it.
        assert!(SEPARATION_INSTRUCTION.contains("--attest-as"));
    }

    /// `medication-list --json` serializes this struct WHOLE, and a `BTreeMap` keyed by
    /// `Uuid` is the one field that could fail at runtime rather than at compile time:
    /// serde_json only accepts map keys that serialize as strings. Nothing else exercises
    /// that path until an operator hits `--json` on a chart with a cross-patient group —
    /// i.e. exactly when they are least able to afford a serializer error.
    #[test]
    fn the_whole_list_serializes_to_json_including_its_uuid_keyed_map() {
        let list = PatientMedicationList {
            rows: vec![],
            groups_missing_from_chart: vec![uid(1)],
            separation_targets: BTreeMap::from([(uid(1), vec![uid(1), uid(2)])]),
        };
        let json = serde_json::to_string(&list).expect("the read model must serialize");
        assert!(json.contains(&uid(2).to_string()), "{json}");
        assert!(json.contains("separation_targets"), "{json}");
    }

    #[test]
    fn an_empty_chart_carries_no_rows_and_no_hazards() {
        let list = PatientMedicationList::empty();
        assert!(list.rows.is_empty());
        assert!(list.groups_missing_from_chart.is_empty());
        assert!(list.separation_targets.is_empty());
    }
}
