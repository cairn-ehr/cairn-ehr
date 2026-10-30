use super::*;
use cairn_medication_view::{ElsewhereLine, MedicationRow, MemberVouch, VouchState};

fn uid(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

/// One active, uncoded row displaying under `display_chart` with the given members,
/// each `(thread, chart it lives on)`. Only the fields `thread_charts` reads matter.
fn row(group: u128, display_chart: u128, members: &[(u128, u128)]) -> MedicationRow {
    MedicationRow {
        group_id: uid(group),
        display_chart: uid(display_chart),
        term: "metformin".into(),
        coding_display: None,
        formulation: None,
        dose_amount: None,
        dose_unit: None,
        sig: None,
        started_value: None,
        started_precision: None,
        status: MedicationStatus::Active,
        members: members
            .iter()
            .map(|&(thread, chart)| MemberVouch {
                medication_id: uid(thread),
                vouch: VouchState::Absent,
                patient_id: uid(chart),
            })
            .collect(),
        reconciliation_flagged: false,
        coding_conflict: false,
        cross_patient: false,
        wrong_chart: Default::default(),
        source_charts: vec![],
        holds: vec![],
        cessation_reason: None,
        other_cessation_reasons: vec![],
        stopped_member_reasons: vec![],
    }
}

/// The decision-2 rule at its sharpest: a reconciled group spanning two linked charts
/// displays under ONE of them (chart 1 here), yet its thread on chart 2 must sign on
/// chart 2. Reading the row's display chart would get exactly that thread wrong.
#[test]
fn each_thread_maps_to_its_own_chart_not_the_display_chart() {
    let rows = [row(10, 1, &[(10, 1), (11, 2)]), row(20, 2, &[(20, 2)])];
    let map = thread_charts(&rows);
    assert_eq!(map.len(), 3);
    assert_eq!(map[&uid(10)], uid(1));
    assert_eq!(map[&uid(11)], uid(2), "the member's chart, not the row's");
    assert_eq!(map[&uid(20)], uid(2));
}

#[test]
fn a_thread_with_no_chart_is_a_failed_line_never_a_guess() {
    let map = thread_charts(&[row(10, 1, &[(10, 1)])]);
    assert_eq!(chart_of_thread(&map, uid(10)), Ok(uid(1)));
    let line = chart_of_thread(&map, uid(99)).unwrap_err();
    assert_eq!(line.medication_id, uid(99));
    assert!(
        line.error.contains("its chart could not be read"),
        "{}",
        line.error
    );
}

#[test]
fn an_unchanged_set_passes_whatever_order_it_was_built_in() {
    let shown = ChartSet::new([uid(2), uid(1)]).unwrap();
    let now = ChartSet::new([uid(1), uid(2), uid(1)]).unwrap();
    assert!(ensure_same_charts(&shown, &now, "while this list was on screen").is_ok());
}

#[test]
fn the_same_targets_pass() {
    assert!(ensure_same_targets(&[uid(1), uid(2)], &[uid(1), uid(2)]).is_ok());
}

/// The target-set half of the whole-gesture refusal is a stale picture too (#797 review): a
/// colleague's line landing between the two reads means the screen is behind the record, so
/// the window re-reads — the sentence says only what happened, never a remedy the window
/// lacks ("refresh the list").
#[test]
fn changed_targets_are_refused_as_stale_naming_both_halves() {
    let err = ensure_same_targets(&[uid(1), uid(2)], &[uid(1), uid(3)]).unwrap_err();
    assert_eq!(
        crate::db_diagnosis::refusal_scope(&err),
        Some(crate::db_diagnosis::RefusalScope::Stale)
    );
    assert_eq!(
        format!("{err:#}"),
        format!(
            "the medication list changed while it was being signed (thread(s) added: {}; \
             removed: {}); nothing was signed",
            uid(3),
            uid(2)
        )
    );
}

/// The second compare (first read against the second) uses the other window and is `Stale` too.
#[test]
fn the_second_compare_is_stale_with_its_own_window() {
    let err = ensure_same_charts(
        &ChartSet::single(uid(1)),
        &ChartSet::new([uid(1), uid(2)]).unwrap(),
        "while it was being signed",
    )
    .unwrap_err();
    assert_eq!(
        crate::db_diagnosis::refusal_scope(&err),
        Some(crate::db_diagnosis::RefusalScope::Stale)
    );
    assert!(format!("{err:#}").starts_with("the linked charts changed while it was being signed ("));
}

#[test]
fn a_changed_set_is_refused_as_stale_naming_both_sets() {
    let shown = ChartSet::single(uid(1));
    let now = ChartSet::new([uid(1), uid(2)]).unwrap();
    let err = ensure_same_charts(&shown, &now, "while this list was on screen").unwrap_err();
    assert_eq!(
        crate::db_diagnosis::refusal_scope(&err),
        Some(crate::db_diagnosis::RefusalScope::Stale)
    );
    let msg = format!("{err:#}");
    assert_eq!(
        msg,
        format!(
            "the linked charts changed while this list was on screen (shown: {one}; now: \
             {one}, {two}); nothing was signed",
            one = uid(1),
            two = uid(2)
        )
    );
}

/// #853 review: a line elsewhere at EITHER read is reported, once per line, the first
/// read's copy and order kept.
#[test]
fn union_elsewhere_reports_a_line_from_either_read_once() {
    let line = |g: u128, name: &str| ElsewhereLine {
        group_id: uid(g),
        name: Some(name.into()),
        status: None,
        charts: vec![],
    };
    assert_eq!(
        union_elsewhere(
            vec![line(1, "first"), line(2, "first")],
            vec![line(2, "second"), line(3, "second")]
        ),
        vec![line(1, "first"), line(2, "first"), line(3, "second")]
    );
}
