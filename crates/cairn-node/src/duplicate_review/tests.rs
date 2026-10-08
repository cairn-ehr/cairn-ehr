//! Pure tests of the banner's grouping (`group_by_other_record`), orientation and record
//! checks. Kept beside `mod.rs` rather than inside it so that file stays under its line budget.
use super::*;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}
fn set(v: &[u128]) -> ChartSet {
    ChartSet::new(v.iter().map(|n| id(*n))).unwrap()
}
fn prop(here: u128, other: u128, vetoed: bool, created_ms: i64) -> OpenProposal {
    OpenProposal {
        here: id(here),
        other: id(other),
        vetoed,
        created_ms,
        accepted: false,
        disputed: false,
    }
}

#[test]
fn a_pair_is_oriented_by_which_side_the_record_holds() {
    let record = set(&[1, 2]);
    assert_eq!(orient(id(1), id(9), &record), Some((id(1), id(9))));
    assert_eq!(orient(id(2), id(9), &record), Some((id(2), id(9))));
    assert_eq!(orient(id(0), id(1), &record), Some((id(1), id(0))));
    // Both inside or both outside: not a banner pair.
    assert_eq!(orient(id(1), id(2), &record), None);
    assert_eq!(orient(id(8), id(9), &record), None);
}

/// Review Focus 2: two of my charts proposed against two charts of ONE other record are one
/// entry, standing for both pairs; Review compares against the NEWEST proposal's chart.
#[test]
fn proposals_against_one_other_record_are_one_entry() {
    let other = set(&[8, 9]);
    let got = group_by_other_record(vec![
        (prop(1, 8, false, 100), other.clone()),
        (prop(2, 9, true, 200), other.clone()),
    ]);
    assert_eq!(got.len(), 1);
    let e = &got[0];
    assert_eq!(e.other_record, other);
    assert_eq!(e.review_chart, id(9), "the newest proposal's other chart");
    assert_eq!(e.pairs, vec![(id(1), id(8)), (id(2), id(9))]);
    assert!(e.vetoed, "any vetoed pair marks the entry");
    assert_eq!(e.newest_ms, 200);
}

/// Two proposals with the same `created_ms`: Review's chart must not depend on input order.
#[test]
fn a_created_time_tie_picks_the_same_review_chart_in_either_order() {
    let other = set(&[8, 9]);
    let (p1, p2) = (prop(1, 8, false, 100), prop(2, 9, false, 100));
    let fwd = group_by_other_record(vec![
        (p1.clone(), other.clone()),
        (p2.clone(), other.clone()),
    ]);
    let rev = group_by_other_record(vec![(p2, other.clone()), (p1, other)]);
    assert_eq!(
        fwd[0].review_chart,
        id(8),
        "the smaller other chart wins the tie"
    );
    assert_eq!(fwd[0].review_chart, rev[0].review_chart);
    assert_eq!(fwd, rev);
}

#[test]
fn entries_are_newest_first_and_records_stay_apart() {
    let got = group_by_other_record(vec![
        (prop(1, 7, false, 100), set(&[7])),
        (prop(1, 9, false, 300), set(&[9])),
    ]);
    let order: Vec<Uuid> = got.iter().map(|e| e.review_chart).collect();
    assert_eq!(order, vec![id(9), id(7)]);
}

/// Type review: the proposal read and each other side's `person_charts` are separate
/// statements, so a link landing in between can put the displayed chart inside the "other"
/// record. That record reads as one with this one — the view's own meaning of "not open" —
/// so it is not another person's entry.
#[test]
fn a_record_holding_a_displayed_chart_is_not_another_record() {
    let mine = set(&[1, 2]);
    assert!(is_another_record(&set(&[8, 9]), &mine));
    assert!(!is_another_record(&set(&[2, 9]), &mine));
    assert!(!is_another_record(&set(&[1]), &mine));
}

#[test]
fn nothing_found_is_no_entries() {
    assert!(group_by_other_record(vec![]).is_empty());
}

/// T6: an entry's `accepted` and `disputed` are ANY of its pairs' — the first proposal accepted,
/// the second disputed, and the entry carries both. An `=` in place of `|=` keeps only the last
/// proposal's flags and fails this.
#[test]
fn accepted_and_disputed_are_ored_across_one_records_proposals() {
    let other = set(&[8, 9]);
    let mut first = prop(1, 8, false, 100);
    first.accepted = true;
    let mut second = prop(2, 9, false, 200);
    second.disputed = true;
    let got = group_by_other_record(vec![(first, other.clone()), (second, other)]);
    assert_eq!(got.len(), 1);
    assert!(
        got[0].accepted,
        "the first proposal's acceptance survives the second"
    );
    assert!(got[0].disputed);
}
