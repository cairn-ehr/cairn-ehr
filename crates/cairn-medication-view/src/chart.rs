//! One patient's whole chart — the rows, and what the node knows is MISSING from them.
//!
//! WHY THIS LIVES IN THE SHARED CRATE AND NOT IN THE NODE. It was born in
//! `cairn-node`'s read path, where it had exactly one consumer (the CLI). The med-list
//! window is the second, and it needs the *same* four things the CLI needs: the rows, the
//! groups the chart cannot display, the thread ids that make the repair runnable, and the
//! chart set the list was read over (ADR-0076).
//!
//! That is not a convenience. ADR-0060 decision 2 says partial completion must be
//! **reported, never implied** — so a renderer that receives only `rows` is structurally
//! incapable of obeying it: it cannot warn about a drug it was never handed. Putting the
//! whole chart here means the window and the CLI answer *"what is this chart missing?"*
//! from one definition, the same reason `sign_off_targets` lives beside it.
//!
//! Pure: no database driver, no GUI toolkit. `cairn-node` re-exports every item from
//! `medication::read`, so its old paths still resolve.
use crate::chart_set::ChartSet;
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

/// What to do about a line withheld because the record holds a DOUBTED link (#697 (b)) —
/// worded once, for every renderer, for the same reason as [`SEPARATION_INSTRUCTION`].
///
/// THE CAUSES IT NAMES. db/054 `cairn_chart_set_has_doubted_link` finds doubt three ways, and
/// the text covers all three in two clauses: an un-attested link the hard identity check flagged
/// when it arrived, or trips now ("has found a clash" — a flag raised on arrival is not
/// re-checked when demographics later change, so "finds" could be false), and a clinician's
/// attested unlink between two charts that other links still join (the A–C–X bridge).
///
/// WHY NOT THE SEPARATION REMEDY FIRST. Both charts are members of the record on screen; the
/// node only doubts that they are one person. Separating threads is right only if they are two
/// people, and even then the LINK is what is wrong — so the links are judged first. An attested
/// link outranks the machine's (ADR-0076 decision 5); an attested unlink splits the record, or
/// is recorded but leaves it joined through another link. The window cannot yet confirm a link
/// that already stands (#716), so the confirm half names the CLI verb, with its required
/// `--attester-key` (both verbs refuse to run without one). Each verb's holding rule is named
/// because the record can list a member whose registration has not reached this node: an
/// unlink needs `--from` only when NEITHER chart is held, but a link needs BOTH held
/// (`cairn-node` `chart_link::admit_judgement`) — a deliberate act from this node must not
/// attach a chart it has never seen.
///
/// WHY NOT "EITHER JUDGEMENT LIFTS THIS HOLD". It is false in reachable cases: with two doubted
/// links, judging one leaves the hold; in the A–C–X bridge, unlinking A–X leaves the record
/// joined; and after an unlink a line shared by the two charts reaches OUTSIDE the record, so it
/// stays withheld under the separation remedy instead. The text says only what is always true:
/// the hold lifts once no link is in doubt, and a separation may follow — the list will say.
pub const DOUBTED_LINK_INSTRUCTION: &str =
    "A clinician must judge this record's links. A link is in doubt when it joins two charts \
     without a clinician's confirmation on record here and the node's hard identity check has \
     found a clash between them, or when a clinician has recorded that two of the record's \
     charts are different people while other links still join them. In the window, \
     \"Not the same person…\" beside a link under \"How these charts are linked\" unlinks it. \
     In the CLI, `unlink-charts <chart_a> <chart_b> --attester-key <key-file>` unlinks (add \
     `--from <open_chart>` when neither chart is held on this node), and `link-charts <chart_a> \
     <chart_b> --attester-key <key-file>` confirms a link — both charts must be held on this \
     node to link them (the window cannot confirm a link that already stands yet). The hold \
     lifts once no link in the record is in doubt. After an unlink, a line shared between the \
     two charts may then reach a chart outside the record, and its threads must then be \
     separated — the list will say so. Judge the links before separating any threads.";

/// What to do about a group the node knows this chart set holds a thread in, but which has
/// NO line on the list (`PatientMedicationList::groups_missing_from_chart`) — worded ONCE,
/// for every renderer, for the same reason as [`SEPARATION_INSTRUCTION`].
///
/// WHY NOT THE SEPARATION REMEDY. Before the combined read (ADR-0076), a missing group WAS
/// the cross-patient case (#334): the group displayed on the other patient's chart only, and
/// separating its threads was the repair. Rows are now selected by membership, so a
/// cross-patient group is SHOWN, flagged and withheld (with the separation remedy on its own
/// line), and a missing group means something else: a concurrent reconciliation or
/// separation re-keyed the group between the read's statements, or a list view dropped a
/// group it should emit (a projection defect). Telling the operator to separate threads
/// "because they belong to different patients" would name the wrong cause and the wrong fix.
pub const MISSING_GROUP_INSTRUCTION: &str =
    "Reload the list: a group can move while the list is being read. If it is still missing, \
     the node's medication projection needs repair — report the group and its threads (named \
     with this warning); do not rely on this list as complete until then.";

/// A patient's chart, plus what the node knows is MISSING from it.
///
/// `rows` is what the clinician sees. `groups_missing_from_chart` is a safety signal: a
/// group with a locally-known member thread on a chart in `charts` that nonetheless has no
/// row. It was introduced for issue #334, when the read selected rows by the list view's
/// single display-winner patient and a group spanning two charts vanished from all but one.
/// Since the combined read (ADR-0076) selects groups through their own member threads, it is
/// empty by construction — kept as a defensive net against a projection that drops a group,
/// or a group re-keyed between the read's statements. Non-empty means this chart is
/// INCOMPLETE, not merely sparse. It does **not** stop the rest of the chart being read or
/// signed (ADR-0060) — it is something every renderer must say out loud.
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
    /// else this struct carries is scoped to the charts in `charts` — each row's `members`
    /// lists only threads whose own chart (`medication_thread_group.patient_id`) is in the
    /// set — so the *other* patient's thread appears nowhere. Without this field the node
    /// names a remedy whose arguments it never shows, and the only way out is raw SQL. The
    /// cross-patient member is deliberately the one piece of another chart's data this read
    /// path surfaces: it is a bare thread id with no clinical content attached, and it is
    /// the minimum needed to repair a wrong-chart link the node itself is complaining about.
    pub separation_targets: BTreeMap<Uuid, Vec<Uuid>>,
    /// The set of charts this list was read over (ADR-0076 decision 1): the opened chart and
    /// every chart in its link component, or just the opened chart when it is linked to
    /// nothing. Carried on the list itself (rather than only inferred from its rows) because
    /// an EMPTY list still has to say which chart it covers: decision 3 holds a chart command
    /// to the set the clinician saw, and a chart with nothing on it is still a chart that was
    /// read. It is also what a sign-off is held to: a surface
    /// that showed this list passes this set back, and a changed set refuses the gesture.
    pub charts: ChartSet,
}

impl PatientMedicationList {
    /// An empty chart over `charts`. Not an error state: a patient with nothing recorded is
    /// a real clinical situation, and it is also what a fixture-mode window shows for any
    /// patient other than the fixture one. Takes the covered set explicitly rather than
    /// defaulting it, because an empty list is exactly the case where nothing else on the
    /// struct could tell you which chart(s) were actually read.
    pub fn empty(charts: ChartSet) -> Self {
        Self {
            rows: vec![],
            groups_missing_from_chart: vec![],
            separation_targets: BTreeMap::new(),
            charts,
        }
    }
}

/// Render hazardous groups as `group <id> (member threads: <a>, <b>)` — the shape EVERY
/// cross-patient message uses, written once.
///
/// The member threads are the whole point (#338 review finding 1): the remedy those
/// messages name (`medication-separate`, see [`SEPARATION_INSTRUCTION`]) takes two THREAD
/// ids, so a message printing only the group id sends the operator looking for arguments
/// the node never shows them. (For a doubted-link line the threads only say which threads are
/// held: that remedy, [`DOUBTED_LINK_INSTRUCTION`], judges links and takes CHART ids.) A group
/// with no locally-known membership degrades honestly to "unknown locally" rather than
/// inventing a list — the same acknowledged-uncertainty direction as the rest of this model.
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

    /// The missing-group remedy must not reuse the cross-patient cause (see its doc).
    #[test]
    fn the_missing_group_instruction_names_a_reload_not_a_separation() {
        assert!(MISSING_GROUP_INSTRUCTION.contains("Reload"));
        assert!(!MISSING_GROUP_INSTRUCTION.contains("medication-separate"));
        assert!(!MISSING_GROUP_INSTRUCTION.contains("different patients"));
    }

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
            charts: ChartSet::single(uid(1)),
        };
        let json = serde_json::to_string(&list).expect("the read model must serialize");
        assert!(json.contains(&uid(2).to_string()), "{json}");
        assert!(json.contains("separation_targets"), "{json}");
    }

    #[test]
    fn an_empty_chart_carries_no_rows_and_no_hazards() {
        let list = PatientMedicationList::empty(ChartSet::single(uid(1)));
        assert!(list.rows.is_empty());
        assert!(list.groups_missing_from_chart.is_empty());
        assert!(list.separation_targets.is_empty());
    }

    /// Task brief step 1: even a chart with nothing on it must be able to say which
    /// chart(s) it was read over — the field the caller needs cannot depend on `rows`
    /// being non-empty, or an empty list would be structurally unable to answer.
    #[test]
    fn an_empty_list_still_says_which_charts_it_covers() {
        let one = Uuid::from_u128(9);
        let list = PatientMedicationList::empty(ChartSet::single(one));
        assert_eq!(list.charts.members(), &[one]);
        assert!(list.rows.is_empty());
    }

    /// #697 part 1: a doubted link's remedy is a judgement of the LINK — both verbs, the window's
    /// gesture by its label — and never thread separation.
    #[test]
    fn the_doubted_link_remedy_names_both_judgements_and_never_separation() {
        assert!(DOUBTED_LINK_INSTRUCTION.contains("`unlink-charts "));
        // "link-charts" alone would be satisfied by "unlink-charts": pin the confirm verb by
        // its own backtick-opened spelling.
        assert!(DOUBTED_LINK_INSTRUCTION.contains("`link-charts "));
        assert!(DOUBTED_LINK_INSTRUCTION.contains("Not the same person"));
        assert!(DOUBTED_LINK_INSTRUCTION.contains("How these charts are linked"));
        assert!(!DOUBTED_LINK_INSTRUCTION.contains("medication-separate"));
    }

    /// Final review F1b: every sentence must hold in every case the text is shown. "Either
    /// judgement lifts this hold" was false (two doubted links; the A–C–X bridge; a shared line
    /// reaching outside after an unlink); both verbs refuse without a human key; and separation
    /// may follow a judgement, so the text orders them rather than forbidding one.
    #[test]
    fn the_doubted_link_remedy_is_true_in_every_case_it_is_shown() {
        assert!(!DOUBTED_LINK_INSTRUCTION.contains("Either judgement lifts"));
        assert!(DOUBTED_LINK_INSTRUCTION.contains("--attester-key"));
        assert!(DOUBTED_LINK_INSTRUCTION.contains("Judge the links before separating"));
        // Residual R1: a LINK needs both charts held here (admit_judgement); a record can list
        // a member whose registration has not arrived, so the confirm verb must say so.
        assert!(DOUBTED_LINK_INSTRUCTION.contains("both charts must be held on this node"));
        // Both causes db/054 can report: the hard check's clash, and a human's unlink.
        assert!(DOUBTED_LINK_INSTRUCTION.contains("hard identity check"));
        assert!(DOUBTED_LINK_INSTRUCTION.contains("different people"));
    }
}
