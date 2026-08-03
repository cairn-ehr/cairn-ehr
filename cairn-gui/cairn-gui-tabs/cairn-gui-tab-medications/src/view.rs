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
use cairn_medication_view::{
    format_hazard_groups, short_kid, sign_off_targets, withheld_rows, MedicationRow,
    MedicationStatus, PatientMedicationList, VouchState, SEPARATION_INSTRUCTION,
};
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};
use uuid::Uuid;

/// One rendered drug line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MedListRowView {
    /// Stable id for the DOM and for the cease command.
    pub group_id: String,
    /// The drug's coded name when it has one, else the term exactly as asserted.
    pub primary: String,
    /// Dose as "500 mg", or an explicit statement that none was recorded.
    pub dose: String,
    pub formulation: String,
    pub sig: String,
    pub started: String,
    /// "current" or "ceased".
    pub status_label: String,
    /// Whose signature this line carries, and whether it is out of date.
    pub vouch_label: String,
    /// True when the sign-off gesture will sign this row. Derived from the SAME
    /// `sign_off_targets` the orchestrator uses — never recomputed here.
    pub will_be_signed: bool,
    /// False for an already-ceased drug.
    pub can_cease: bool,
    /// Advisory worklist labels (duplicate suspicion, anchor conflict, wrong-chart hazard).
    pub flags: Vec<String>,
}

/// The whole window's state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MedListView {
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

/// Shown instead of a blank cell. Principle 4: an unrecorded dose is a recordable state,
/// and a blank would read either as "no dose" or as a rendering bug.
const DOSE_UNKNOWN: &str = "dose not recorded";

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

    let view_rows: Vec<MedListRowView> = list
        .rows
        .iter()
        .map(|row| MedListRowView {
            group_id: row.group_id.to_string(),
            primary: row.display_name().to_string(),
            dose: match (&row.dose_amount, &row.dose_unit) {
                (Some(amount), Some(unit)) => format!("{amount} {unit}"),
                (Some(amount), None) => amount.clone(),
                _ => DOSE_UNKNOWN.to_string(),
            },
            formulation: row.formulation.clone().unwrap_or_default(),
            sig: row.sig.clone().unwrap_or_default(),
            started: row.started_value.clone().unwrap_or_default(),
            status_label: match row.status {
                MedicationStatus::Active => "current".into(),
                MedicationStatus::Ceased => "ceased".into(),
            },
            vouch_label: vouch_label(row),
            will_be_signed: row
                .members
                .iter()
                .any(|m| targets.contains(&m.medication_id)),
            can_cease: row.status == MedicationStatus::Active,
            flags: flags(row),
        })
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
        rows: view_rows,
        sign_off_count,
        sign_off_enabled: sign_off_count > 0,
    }
}

/// Whose signature this line carries.
///
/// A reconciled group has several member threads, which can disagree. The honest summary
/// names the worst state rather than picking one member's — a group is not signed off
/// until every member is.
fn vouch_label(row: &MedicationRow) -> String {
    let unsigned = row
        .members
        .iter()
        .filter(|m| m.vouch == VouchState::Absent)
        .count();
    let stale: Vec<&str> = row
        .members
        .iter()
        .filter_map(|m| match &m.vouch {
            VouchState::Stale { by } => Some(by.as_str()),
            _ => None,
        })
        .collect();
    if unsigned > 0 {
        // No signature at all is the worse state of the two, so it wins the summary: a
        // group reported as "signed but out of date" reads as needing a refresh, while an
        // unsigned member has never been vouched by anyone.
        return "not signed".to_string();
    }
    if let Some(by) = stale.first() {
        return format!("signed by {} — out of date", short_kid(by));
    }
    match row.members.first().and_then(|m| m.vouch.attester()) {
        Some(by) => format!("signed by {}", short_kid(by)),
        // No members at all. Not expected from the read path, but "not signed" is the
        // honest reading of "nothing here vouches for this line".
        None => "not signed".to_string(),
    }
}

fn flags(row: &MedicationRow) -> Vec<String> {
    let mut out = Vec::new();
    if row.reconciliation_flagged {
        out.push("possible duplicate — not yet reconciled".to_string());
    }
    if row.coding_conflict {
        out.push("two different drug identities in this group".to_string());
    }
    if row.cross_patient {
        // Per-row, and in the row's own words, because this is where the clinician is
        // looking when they wonder why the line has no signature badge. The message names
        // the DOSE risk specifically: the displayed dose comes from a whole-group pick that
        // ignores patient, so it may be the other patient's (issue #334).
        out.push(
            "shared with another patient's record — the dose shown may not be this \
             patient's, so this line cannot be signed"
                .to_string(),
        );
    }
    out
}

/// The withheld-lines report: displayed lines that need a signature and will not get one.
///
/// PUBLIC because two surfaces render it — the chart *before* the gesture ("these lines
/// will not be signed") and the outcome *after* it ("these lines were not signed"). Those
/// are the same fact at two moments, and two hand-written renderings of it are how the
/// promise and the report start to disagree. It takes the group ids rather than the chart
/// so the after-the-fact caller can pass `SignOffOutcome::withheld`, which is what the
/// orchestrator actually did rather than what a re-read says it would do now.
pub fn withheld_report(
    withheld: &[Uuid],
    separation_targets: &BTreeMap<Uuid, Vec<Uuid>>,
) -> Option<String> {
    if withheld.is_empty() {
        return None;
    }
    Some(format!(
        "{} line(s) on this chart still need a signature but will NOT be signed: {}. {}",
        withheld.len(),
        format_hazard_groups(withheld, separation_targets),
        SEPARATION_INSTRUCTION
    ))
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
        "This chart is INCOMPLETE. {} medication group(s) known to this patient cannot be \
         displayed here, because their threads are shared with another patient's record: \
         {}. {}",
        missing.len(),
        format_hazard_groups(missing, separation_targets),
        SEPARATION_INSTRUCTION
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
    use cairn_medication_view::{MedicationRow, MedicationStatus, MemberVouch};

    fn uid(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn row(group: u128, status: MedicationStatus, members: Vec<MemberVouch>) -> MedicationRow {
        MedicationRow {
            group_id: uid(group),
            patient_id: uid(999),
            term: "metformin".into(),
            coding_display: None,
            formulation: None,
            dose_amount: Some("500".into()),
            dose_unit: Some("mg".into()),
            sig: None,
            started_value: None,
            started_precision: None,
            status,
            members,
            reconciliation_flagged: false,
            coding_conflict: false,
            cross_patient: false,
        }
    }

    fn member(id: u128, vouch: VouchState) -> MemberVouch {
        MemberVouch {
            medication_id: uid(id),
            vouch,
        }
    }

    /// A chart of exactly these rows, with nothing hidden and nothing to repair — the
    /// normal case, which is what most of these tests are about.
    fn chart(rows: Vec<MedicationRow>) -> PatientMedicationList {
        PatientMedicationList {
            rows,
            groups_missing_from_chart: vec![],
            separation_targets: BTreeMap::new(),
        }
    }

    #[test]
    fn a_coded_drug_displays_its_coded_name_over_the_free_text_term() {
        let mut r = row(
            1,
            MedicationStatus::Active,
            vec![member(1, VouchState::Absent)],
        );
        r.term = "little white pill".into();
        r.coding_display = Some("metformin hydrochloride".into());
        let view = build_view(&chart(vec![r]));
        assert_eq!(view.rows[0].primary, "metformin hydrochloride");
    }

    /// Principle 4: a vague term is a legitimate recorded value, never blanked out.
    #[test]
    fn an_uncoded_drug_displays_its_free_text_term_unaltered() {
        let mut r = row(
            1,
            MedicationStatus::Active,
            vec![member(1, VouchState::Absent)],
        );
        r.term = "little white pill".into();
        let view = build_view(&chart(vec![r]));
        assert_eq!(view.rows[0].primary, "little white pill");
    }

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

    #[test]
    fn a_fresh_vouch_names_its_signatory() {
        let rows = vec![row(
            1,
            MedicationStatus::Active,
            vec![member(
                1,
                VouchState::Fresh {
                    by: "abcdef0123456789".into(),
                },
            )],
        )];
        let view = build_view(&chart(rows));
        assert!(
            view.rows[0].vouch_label.contains("abcdef01"),
            "the clinician must see WHOSE signature it is: {}",
            view.rows[0].vouch_label
        );
    }

    #[test]
    fn a_stale_vouch_says_so() {
        let rows = vec![row(
            1,
            MedicationStatus::Active,
            vec![member(
                1,
                VouchState::Stale {
                    by: "abcdef0123456789".into(),
                },
            )],
        )];
        let view = build_view(&chart(rows));
        assert!(
            view.rows[0].vouch_label.contains("out of date"),
            "got: {}",
            view.rows[0].vouch_label
        );
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

    #[test]
    fn a_ceased_row_is_shown_marked_and_never_targeted() {
        let rows = vec![row(
            1,
            MedicationStatus::Ceased,
            vec![member(1, VouchState::Absent)],
        )];
        let view = build_view(&chart(rows));
        assert_eq!(view.rows.len(), 1, "a struck line stays on the chart");
        assert_eq!(view.rows[0].status_label, "ceased");
        assert!(!view.rows[0].will_be_signed);
        assert!(
            !view.rows[0].can_cease,
            "a ceased drug cannot be ceased again"
        );
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

    #[test]
    fn advisory_flags_are_surfaced_as_row_labels() {
        let mut r = row(
            1,
            MedicationStatus::Active,
            vec![member(1, VouchState::Absent)],
        );
        r.reconciliation_flagged = true;
        r.coding_conflict = true;
        let view = build_view(&chart(vec![r]));
        assert_eq!(view.rows[0].flags.len(), 2, "got: {:?}", view.rows[0].flags);
    }

    #[test]
    fn the_dose_reads_as_amount_and_unit() {
        let view = build_view(&chart(vec![row(
            1,
            MedicationStatus::Active,
            vec![member(1, VouchState::Absent)],
        )]));
        assert_eq!(view.rows[0].dose, "500 mg");
    }

    /// Principle 4 again: an unknown dose is shown as unknown, never as a blank that reads
    /// like "no dose" or as a fabricated default.
    #[test]
    fn an_absent_dose_is_shown_as_unknown() {
        let mut r = row(
            1,
            MedicationStatus::Active,
            vec![member(1, VouchState::Absent)],
        );
        r.dose_amount = None;
        r.dose_unit = None;
        assert_eq!(
            build_view(&chart(vec![r])).rows[0].dose,
            "dose not recorded"
        );
    }

    // ---- ADR-0060: a defect on one line never invalidates another, but it is always
    // reported. These are the tests for the reporting half (decision 2).

    /// The withheld line is SHOWN — hiding a drug is the worse failure — but it is not a
    /// sign-off target, and the row has to say so where the clinician is looking.
    #[test]
    fn a_cross_patient_line_is_shown_flagged_and_not_signed() {
        let mut r = row(
            1,
            MedicationStatus::Active,
            vec![member(1, VouchState::Absent)],
        );
        r.cross_patient = true;
        let view = build_view(&chart(vec![r]));
        assert_eq!(view.rows.len(), 1, "the drug must still be visible");
        assert!(!view.rows[0].will_be_signed);
        assert!(
            view.rows[0]
                .flags
                .iter()
                .any(|f| f.contains("another patient")),
            "the row must say why it cannot be signed: {:?}",
            view.rows[0].flags
        );
    }

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

    /// The half with no row at all: a group the node knows this patient has a thread in,
    /// but which displays on someone else's chart. The report is the ONLY surface it has.
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
        };
        let view = build_view(&list);
        let message = view
            .missing_message
            .expect("an incomplete chart must say so");
        assert!(message.contains(&uid(71).to_string()), "{message}");
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

    /// A group is not signed off until every member is: the summary names the WORST member
    /// state, so a half-signed reconciled pair never reads as done.
    #[test]
    fn a_partly_signed_group_does_not_read_as_signed() {
        let reconciled = row(
            1,
            MedicationStatus::Active,
            vec![
                member(1, VouchState::Fresh { by: "dr_b".into() }),
                member(2, VouchState::Absent),
            ],
        );
        let view = build_view(&chart(vec![reconciled]));
        assert_eq!(view.rows[0].vouch_label, "not signed");
        assert!(view.rows[0].will_be_signed);
    }
}
