//! Tests for `hold/plan.rs`: the thread a W is written on and the holds a Release ends.
use super::*;
use cairn_medication_view::{MedicationHold, MedicationRow, MemberVouch, VouchState};
use uuid::Uuid;

fn u(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

/// A line whose group is `group`, with `(thread, chart)` members and `(hold id, thread)` holds.
fn row(
    hazard: bool,
    group: u128,
    members: &[(u128, u128)],
    holds: &[(u128, u128)],
) -> MedicationRow {
    let mut row = cairn_medication_view::fixtures::sample_chart().rows[0].clone();
    row.cross_patient = hazard;
    row.group_id = u(group);
    row.members = members
        .iter()
        .map(|(t, c)| MemberVouch {
            medication_id: u(*t),
            vouch: VouchState::Absent,
            patient_id: u(*c),
        })
        .collect();
    row.holds = holds
        .iter()
        .map(|(id, t)| MedicationHold {
            withhold_event_id: u(*id),
            medication_id: u(*t),
            reason: "r".into(),
            starts_at: "s".into(),
            until: "reviewed".into(),
            withholder: "k".into(),
            pending: false,
        })
        .collect();
    row
}

#[test]
fn a_w_goes_on_the_lines_own_thread_on_its_own_chart() {
    // Representative thread 11 lives on chart 2, not the opened chart 1.
    let r = row(false, 11, &[(10, 1), (11, 2)], &[]);
    assert_eq!(withhold_target(&r, u(1)), Ok((u(11), u(2))));
}

#[test]
fn a_hazard_line_withholds_on_the_opened_charts_thread() {
    let r = row(true, 11, &[(10, 1), (11, 2)], &[]);
    assert_eq!(withhold_target(&r, u(1)), Ok((u(10), u(1))));
}

#[test]
fn a_hazard_line_with_nothing_on_this_chart_is_refused() {
    let r = row(true, 11, &[(11, 2)], &[]);
    let err = withhold_target(&r, u(1)).unwrap_err();
    assert!(err.contains("another person's drug"), "{err}");
}

#[test]
fn a_release_ends_only_the_holds_it_was_shown() {
    let r = row(false, 10, &[(10, 1)], &[(100, 10), (101, 10)]);
    let p = release_plan(&r, &[u(100)], u(1));
    assert_eq!(p.write, vec![(u(10), u(1), vec![u(100)])]);
    assert!(p.held_back.is_empty() && p.gone.is_empty());
}

#[test]
fn a_release_writes_one_event_per_thread() {
    let r = row(false, 10, &[(10, 1), (11, 2)], &[(100, 10), (101, 11)]);
    // Listed twice: still written once.
    let p = release_plan(&r, &[u(100), u(101), u(100)], u(1));
    assert_eq!(
        p.write,
        vec![(u(10), u(1), vec![u(100)]), (u(11), u(2), vec![u(101)])]
    );
}

#[test]
fn a_hold_that_ended_meanwhile_is_named_gone() {
    let r = row(false, 10, &[(10, 1)], &[(100, 10)]);
    let p = release_plan(&r, &[u(100), u(999)], u(1));
    assert_eq!(p.gone, vec![u(999)]);
    assert_eq!(p.write.len(), 1);
}

#[test]
fn a_hazard_line_holds_back_another_charts_hold() {
    let r = row(true, 10, &[(10, 1), (11, 2)], &[(100, 10), (101, 11)]);
    let p = release_plan(&r, &[u(100), u(101)], u(1));
    assert_eq!(p.write, vec![(u(10), u(1), vec![u(100)])]);
    assert_eq!(p.held_back.len(), 1);
    assert!(
        p.held_back[0].contains("NOT released from here"),
        "{}",
        p.held_back[0]
    );
    assert!(p.held_back[0].contains(&u(101).to_string()));
}

#[test]
fn a_hold_on_a_non_member_thread_is_held_back() {
    let r = row(false, 10, &[(10, 1)], &[(100, 77)]);
    let p = release_plan(&r, &[u(100)], u(1));
    assert!(p.write.is_empty());
    assert_eq!(p.held_back.len(), 1);
}
