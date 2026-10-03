//! The med-list view model: everything the window shows, computed in Rust.
//!
//! The webview renders this and decides nothing. Every clinical display question — which
//! name to show, whose signature a line carries, whether the gesture will sign this row,
//! what the chart cannot show — is answered here, under `cargo test`, because a wrong
//! answer is a clinical falsehood on screen and a webview is not a place we can test that.
//!
//! # The two rules this module exists to keep
//!
//! 1. **One targeting rule.** `will_be_signed` on a row and the count on the button both
//!    come from a single `sign_off_targets` call — the same function the node's
//!    orchestrator uses. A second implementation would eventually disagree, and a
//!    disagreement here paints a "signed" badge over a thread nobody signed.
//! 2. **Partial completion is reported, never implied** (ADR-0060 decision 2). A line the
//!    gesture will not sign, and a group the chart cannot display at all, each get a
//!    message naming the remedy *and its arguments*. Silence would let "signed off 11
//!    medications" stand over a chart with a twelfth nobody knows about.
use crate::row_view::build_row;
pub use crate::row_view::MedListRowView;
use cairn_medication_view::{
    format_hazard_groups, sign_off_targets, withheld_rows, MedicationStatus, PatientMedicationList,
    WithheldLine, WrongChartReasons, DOUBTED_LINK_INSTRUCTION, MISSING_GROUP_INSTRUCTION,
    SEPARATION_INSTRUCTION,
};
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};
use uuid::Uuid;

/// The whole window's state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MedListView {
    /// The chart SET this list was read over (`PatientMedicationList::charts`), as uuid
    /// strings in the set's sorted order — one entry for a never-linked chart.
    ///
    /// The window keeps this and sends it back with every chart command (ADR-0076 decision 3):
    /// a sign-off must sign the list the clinician SAW, and if a link or unlink changed the set
    /// while the list was on screen, the backend refuses rather than signing a list nobody
    /// reviewed.
    pub charts: Vec<String>,
    pub rows: Vec<MedListRowView>,
    /// How many THREADS the gesture will sign. Not the row count — a reconciled group can
    /// contribute more than one, and the clinician is entitled to know the real number.
    pub sign_off_count: usize,
    pub sign_off_enabled: bool,
    /// Why there is nothing to do, when there is nothing to do.
    pub empty_message: Option<String>,
    /// Lines that are DISPLAYED, still need a signature, and will deliberately not get one
    /// (today: cross-patient groups, issue #334). `None` in normal operation.
    pub withheld_message: Option<String>,
    /// Groups the node knows this patient has threads in but cannot display at all — the
    /// chart is INCOMPLETE, not merely sparse. `None` in normal operation.
    pub missing_message: Option<String>,
}

/// Build the whole window state from one chart read.
///
/// Takes the whole `PatientMedicationList` rather than its rows, because two of the three
/// things this function must report — the withheld lines' repair arguments and the groups
/// with no row at all — do not exist inside `rows` (see ADR-0060 decision 2 and the module
/// doc). A signature taking only `&[MedicationRow]` would make the omission unfixable at
/// this layer rather than merely absent.
pub fn build_view(list: &PatientMedicationList) -> MedListView {
    // ONE call to the shared rule. The badge on each row and the count on the button both
    // come from this set, so what the clinician is told will be signed is, by
    // construction, what the orchestrator will sign.
    let targets: HashSet<_> = sign_off_targets(&list.rows).into_iter().collect();

    // One row per chart row; what a single line says is `row_view`'s job. Whether rows name
    // their source chart is a property of the whole list, so it is decided here, once.
    let linked = list.charts.is_linked();
    let view_rows: Vec<MedListRowView> = list
        .rows
        .iter()
        .map(|row| build_row(row, &targets, linked))
        .collect();

    let sign_off_count = targets.len();
    let active_rows = list
        .rows
        .iter()
        .filter(|row| row.status == MedicationStatus::Active)
        .count();

    MedListView {
        empty_message: empty_message(list.rows.len(), active_rows, sign_off_count),
        withheld_message: withheld_report(&withheld_rows(&list.rows), &list.separation_targets),
        missing_message: missing_report(&list.groups_missing_from_chart, &list.separation_targets),
        charts: list.charts.members().iter().map(Uuid::to_string).collect(),
        rows: view_rows,
        sign_off_count,
        sign_off_enabled: sign_off_count > 0,
    }
}

/// The withheld-lines report: displayed lines that need a signature and will not get one.
/// One sentence per reason, each with its own remedy (#697). A line with both reasons is
/// named in both.
///
/// PUBLIC because two surfaces render it — the chart *before* the gesture ("these lines
/// will not be signed") and the outcome *after* it ("these lines were not signed"). Those
/// are the same fact at two moments, and two hand-written renderings of it are how the
/// promise and the report start to disagree. It takes the group ids rather than the chart
/// so the after-the-fact caller can pass `SignOffOutcome::withheld`, which is what the
/// orchestrator actually did rather than what a re-read says it would do now.
pub fn withheld_report(
    withheld: &[WithheldLine],
    separation_targets: &BTreeMap<Uuid, Vec<Uuid>>,
) -> Option<String> {
    // The group ids of the lines whose reasons satisfy `pick`, in the order given. Each line's
    // reasons go through `worded`, so a line whose set is empty is still counted (as the
    // outside case) rather than falling through both blocks below.
    let groups = |pick: fn(WrongChartReasons) -> bool| -> Vec<Uuid> {
        withheld
            .iter()
            .filter(|l| pick(l.reasons.worded()))
            .map(|l| l.group_id)
            .collect()
    };
    let outside = groups(|r| r.outside_set);
    let doubted = groups(|r| r.doubted_link);
    let mut parts = Vec::new();
    if !outside.is_empty() {
        parts.push(format!(
            "{} line(s) on this chart still need a signature but will NOT be signed: {}. {}",
            outside.len(),
            format_hazard_groups(&outside, separation_targets),
            SEPARATION_INSTRUCTION
        ));
    }
    if !doubted.is_empty() {
        parts.push(format!(
            "{} line(s) on this record still need a signature but will NOT be signed until the \
             record's links are no longer in doubt — this record's links are in doubt, and they \
             are not recorded only on this chart, so the node cannot yet vouch that they are \
             this patient's: {}. {}",
            doubted.len(),
            format_hazard_groups(&doubted, separation_targets),
            DOUBTED_LINK_INSTRUCTION
        ));
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

/// The incomplete-chart report: groups with no row at all.
///
/// This is the harder half to surface, and the one a renderer is most likely to drop:
/// there is nothing on screen to hang it off, because the whole point is that the drug
/// could not be displayed. Public for the same reason as `withheld_report`.
pub fn missing_report(
    missing: &[Uuid],
    separation_targets: &BTreeMap<Uuid, Vec<Uuid>>,
) -> Option<String> {
    if missing.is_empty() {
        return None;
    }
    Some(format!(
        "This chart is INCOMPLETE. {} medication group(s) this record holds a thread in have \
         no line on this list: {}. {}",
        missing.len(),
        format_hazard_groups(missing, separation_targets),
        MISSING_GROUP_INSTRUCTION
    ))
}

/// Why there is nothing to sign, when there is nothing to sign.
///
/// Three distinct states, deliberately not collapsed (#338 review finding 2). Saying
/// "every drug carries a current signature" about a chart whose only drug is a ceased,
/// never-signed one is a plain falsehood: that drug carries no signature at all.
fn empty_message(total_rows: usize, active_rows: usize, sign_off_count: usize) -> Option<String> {
    if total_rows == 0 {
        // Deliberately does NOT claim the patient takes nothing: an empty chart means
        // nothing has been recorded here, which is not the same clinical statement.
        // Recording "nil medications, reviewed" is issue #331.
        Some("No medications recorded on this chart.".to_string())
    } else if active_rows == 0 {
        Some("No current medications on this chart — every line here has been stopped.".to_string())
    } else if sign_off_count == 0 {
        Some("Every current drug on this chart carries a current signature.".to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_rows::{chart, member, row, uid};
    use cairn_medication_view::{MedicationStatus, VouchState};

    /// The badge and the button must agree, because they come from ONE rule.
    #[test]
    fn rows_that_will_be_signed_match_the_sign_off_count() {
        let rows = vec![
            row(
                1,
                MedicationStatus::Active,
                vec![member(1, VouchState::Absent)],
            ),
            row(
                2,
                MedicationStatus::Active,
                vec![member(
                    2,
                    VouchState::Fresh {
                        by: "dr_b_key".into(),
                    },
                )],
            ),
            row(
                3,
                MedicationStatus::Active,
                vec![member(
                    3,
                    VouchState::Stale {
                        by: "dr_b_key".into(),
                    },
                )],
            ),
        ];
        let view = build_view(&chart(rows));
        assert_eq!(view.sign_off_count, 2, "two threads need a signature");
        assert!(view.rows[0].will_be_signed);
        assert!(
            !view.rows[1].will_be_signed,
            "Dr B's current signature stands"
        );
        assert!(view.rows[2].will_be_signed);
        assert!(view.sign_off_enabled);
    }

    /// Issue #331's honest surface: nothing to sign, and the reason is stated rather than
    /// leaving a dead button.
    #[test]
    fn an_empty_chart_disables_the_gesture_and_explains_why() {
        let view = build_view(&chart(vec![]));
        assert_eq!(view.sign_off_count, 0);
        assert!(!view.sign_off_enabled);
        assert!(view.empty_message.is_some());
    }

    #[test]
    fn a_fully_signed_chart_disables_the_gesture() {
        let rows = vec![row(
            1,
            MedicationStatus::Active,
            vec![member(1, VouchState::Fresh { by: "me".into() })],
        )];
        let view = build_view(&chart(rows));
        assert!(!view.sign_off_enabled);
        assert_eq!(view.sign_off_count, 0);
    }

    /// The #338 review finding 2 falsehood, at the UI layer: a chart holding nothing but a
    /// ceased, never-signed drug has nothing to sign — but saying "every drug carries a
    /// current signature" about it is a plain lie. That drug carries NO signature; it is a
    /// struck line that is never re-signed.
    #[test]
    fn a_ceased_only_chart_never_claims_everything_is_signed() {
        let rows = vec![row(
            1,
            MedicationStatus::Ceased,
            vec![member(1, VouchState::Absent)],
        )];
        let message = build_view(&chart(rows))
            .empty_message
            .expect("must explain");
        assert!(
            !message.contains("signature"),
            "must not claim signedness about a chart with no current drugs: {message}"
        );
    }

    // ---- ADR-0060: a defect on one line never invalidates another, but it is always
    // reported. These are the tests for the reporting half (decision 2).

    /// One bad line never stops the others being signed (ADR-0060). The saline case: the
    /// unsignable potassium line must not take the signable saline line down with it.
    #[test]
    fn a_withheld_line_does_not_block_the_rest_of_the_chart() {
        let mut hazard = row(
            1,
            MedicationStatus::Active,
            vec![member(1, VouchState::Absent)],
        );
        hazard.cross_patient = true;
        let good = row(
            2,
            MedicationStatus::Active,
            vec![member(2, VouchState::Absent)],
        );
        let view = build_view(&chart(vec![hazard, good]));
        assert_eq!(view.sign_off_count, 1, "the clean line is still signable");
        assert!(view.sign_off_enabled);
    }

    /// Reported, never implied: a withheld line must produce a message naming the remedy
    /// AND its arguments, because `medication-separate` takes two THREAD ids.
    #[test]
    fn withheld_lines_produce_a_message_naming_the_remedy_and_its_arguments() {
        let mut hazard = row(
            1,
            MedicationStatus::Active,
            vec![member(1, VouchState::Absent)],
        );
        hazard.cross_patient = true;
        let list = PatientMedicationList {
            rows: vec![hazard],
            groups_missing_from_chart: vec![],
            separation_targets: BTreeMap::from([(uid(1), vec![uid(1), uid(2)])]),
            charts: cairn_medication_view::ChartSet::single(uid(999)),
        };
        let message = build_view(&list)
            .withheld_message
            .expect("must be reported");
        assert!(message.contains("medication-separate"), "{message}");
        assert!(
            message.contains(&uid(2).to_string()),
            "the OTHER patient's thread id is the argument they cannot otherwise get: {message}"
        );
    }

    /// #697 part 1: a doubted-link line's report names the LINK judgement, never separation.
    #[test]
    fn a_doubted_link_report_names_the_link_judgement_not_separation() {
        let mut hazard = row(
            1,
            MedicationStatus::Active,
            vec![member(1, VouchState::Absent)],
        );
        hazard.cross_patient = true;
        hazard.wrong_chart.doubted_link = true;
        let message = build_view(&chart(vec![hazard]))
            .withheld_message
            .expect("reported");
        assert!(message.contains("unlink-charts"), "{message}");
        assert!(
            message.contains(&uid(1).to_string()),
            "the line is named: {message}"
        );
        assert!(!message.contains("medication-separate"), "{message}");
        // Final review F2: true for a line on A and X seen from X too, and when the line's own
        // chart is human-linked while another link is doubted.
        assert!(
            message.contains("will NOT be signed until the record's links are no longer in doubt"),
            "{message}"
        );
        assert!(message.contains("cannot yet vouch"), "{message}");
        assert!(!message.contains("from this chart"), "{message}");
    }

    /// The report prints one block per reason, so a withheld line with an empty reason set
    /// must still be counted — worded as the pre-#697 outside case — not silently dropped.
    #[test]
    fn a_withheld_line_with_no_recorded_reason_is_still_reported() {
        let line = WithheldLine {
            group_id: uid(1),
            reasons: cairn_medication_view::WrongChartReasons::default(),
        };
        let message = withheld_report(&[line], &BTreeMap::new()).expect("reported");
        assert!(message.contains(&uid(1).to_string()), "{message}");
    }

    #[test]
    fn a_line_with_both_reasons_is_reported_under_both_remedies() {
        let mut hazard = row(
            1,
            MedicationStatus::Active,
            vec![member(1, VouchState::Absent)],
        );
        hazard.cross_patient = true;
        hazard.wrong_chart = cairn_medication_view::WrongChartReasons {
            outside_set: true,
            doubted_link: true,
        };
        let message = build_view(&chart(vec![hazard]))
            .withheld_message
            .expect("reported");
        assert!(message.contains("medication-separate"), "{message}");
        assert!(message.contains("unlink-charts"), "{message}");
    }

    /// The half with no row at all: a group the node knows this record has a thread in, but
    /// which has no line (a group re-keyed mid-read, or a projection defect — a cross-patient
    /// group is SHOWN since ADR-0076). The report is the ONLY surface it has, and it must not
    /// name the cross-patient cause or its separation remedy.
    #[test]
    fn a_group_that_cannot_be_displayed_is_reported_as_missing() {
        let list = PatientMedicationList {
            rows: vec![row(
                1,
                MedicationStatus::Active,
                vec![member(1, VouchState::Absent)],
            )],
            groups_missing_from_chart: vec![uid(70)],
            separation_targets: BTreeMap::from([(uid(70), vec![uid(70), uid(71)])]),
            charts: cairn_medication_view::ChartSet::single(uid(999)),
        };
        let view = build_view(&list);
        let message = view
            .missing_message
            .expect("an incomplete chart must say so");
        assert!(message.contains(&uid(71).to_string()), "{message}");
        assert!(
            !message.contains("another patient"),
            "a missing group is no longer the cross-patient case: {message}"
        );
        assert!(message.contains("Reload"), "{message}");
        assert!(
            view.sign_off_enabled,
            "an incomplete chart still signs the lines it CAN show (ADR-0060)"
        );
    }

    /// A healthy chart must stay quiet. A warning that fires on every chart is a warning
    /// nobody reads — the same reason `withheld_rows` reports only outstanding lines.
    #[test]
    fn a_healthy_chart_reports_nothing_to_repair() {
        let view = build_view(&chart(vec![row(
            1,
            MedicationStatus::Active,
            vec![member(1, VouchState::Absent)],
        )]));
        assert!(view.withheld_message.is_none());
        assert!(view.missing_message.is_none());
    }

    /// A reconciled group is ONE row but several threads, and the button counts THREADS.
    /// A clinician told "sign off 1" who actually signs 2 was not told the truth.
    #[test]
    fn the_count_is_threads_not_rows() {
        let mut reconciled = row(
            1,
            MedicationStatus::Active,
            vec![member(1, VouchState::Absent), member(2, VouchState::Absent)],
        );
        reconciled.reconciliation_flagged = true;
        let view = build_view(&chart(vec![reconciled]));
        assert_eq!(view.rows.len(), 1, "one displayed line");
        assert_eq!(view.sign_off_count, 2, "two threads get signed");
    }
}
