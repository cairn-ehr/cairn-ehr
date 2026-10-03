//! The wrong-chart hazard rule (#334, ADR-0076, #697 (b)), pure so it is tested without a
//! database. Moved out of `read.rs` when the rule grew a third input (the opened chart): the
//! read assembles the facts, and this decides what they mean.
//!
//! A medication line is withheld from sign-off when signing it could vouch for another
//! person's drug. Two independent reasons, each worded with its own remedy downstream
//! (`cairn_medication_view::WrongChartReasons`):
//!
//! 1. `outside_set` — the group reaches a chart OUTSIDE the set the list was read over.
//!    Linked charts are one person (ADR-0076 decision 1); a chart outside the set is not.
//! 2. `doubted_link` — the set holds a link this node DOUBTS (db/054
//!    `cairn_chart_set_has_doubted_link`: an un-attested link flagged by db/018 or tripping the
//!    hard veto now, or a clinician's attested unlink between two charts the set still joins)
//!    and the line is not recorded ONLY on the opened chart. The maintainer's #697 option (b):
//!    a signature is a claim about a person, and a hard veto or a human's "not the same person"
//!    is positive evidence the other member may be someone else. The line stays visible —
//!    hiding it would be the hazard if the two charts ARE one person — and humans judging the
//!    links lift it once db/054 finds no doubt left.
//!    The rule does not read the link graph to find WHICH pair is doubted: every line not on the
//!    opened chart is withheld. That over-warns in a set of three or more, the direction this
//!    module always errs in.
//!
//! NOT COVERED, deliberately (an under-warn, so stated): an UN-attested unlink between two
//! charts the set still joins is not a doubt. Only a human's "not the same person" is (design
//! D4); counting a machine's or a peer agent's unlink would let any unreviewed writer freeze
//! sign-off on a record (ADR-0030). Pinned by `tests/doubted_link_withholds.rs`.
use cairn_medication_view::{ChartSet, WrongChartReasons};
use uuid::Uuid;

/// Whether a group touching `group_charts` reaches a chart outside `set`.
fn reaches_outside(set: &ChartSet, group_charts: &[Uuid]) -> bool {
    !set.contains_all(group_charts)
}

/// Whether every chart the group touches is the opened chart. An EMPTY list (a group re-keyed
/// mid-read, see `MedicationRow::source_charts`) is NOT "only on the opened chart": unknown
/// provenance must not read as safe.
fn only_on_opened(opened: Uuid, group_charts: &[Uuid]) -> bool {
    !group_charts.is_empty() && group_charts.iter().all(|c| *c == opened)
}

/// The reasons a group touching `group_charts` is a wrong-chart hazard, read over `set` from
/// the `opened` chart. `group_charts` is every chart the group touches, the union of the read's
/// three sources (`read.rs` `touched_charts`); duplicates are harmless.
pub(crate) fn wrong_chart_reasons(
    set: &ChartSet,
    opened: Uuid,
    set_has_doubted_link: bool,
    group_charts: &[Uuid],
) -> WrongChartReasons {
    WrongChartReasons {
        outside_set: reaches_outside(set, group_charts),
        doubted_link: set_has_doubted_link && !only_on_opened(opened, group_charts),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    /// Charts 1 and 2 are linked; 1 is the one opened.
    fn set12() -> ChartSet {
        ChartSet::new([u(1), u(2)]).unwrap()
    }

    #[test]
    fn a_group_wholly_inside_an_undoubted_set_is_not_a_hazard() {
        assert!(!wrong_chart_reasons(&set12(), u(1), false, &[u(2), u(1)]).any());
        assert!(!wrong_chart_reasons(&set12(), u(1), false, &[u(2)]).any());
    }

    #[test]
    fn a_group_reaching_one_chart_outside_is_a_hazard() {
        let r = wrong_chart_reasons(&set12(), u(1), false, &[u(2), u(3)]);
        assert_eq!(
            r,
            WrongChartReasons {
                outside_set: true,
                doubted_link: false
            }
        );
    }

    /// The opened chart's own line shows that chart's own dose and vouches only for the
    /// patient whose chart is open: signable, doubted link or not. Duplicates (the read's sources
    /// overlap) must not read as two charts.
    #[test]
    fn in_a_doubted_set_a_line_only_on_the_opened_chart_is_signable() {
        assert!(!wrong_chart_reasons(&set12(), u(1), true, &[u(1)]).any());
        assert!(!wrong_chart_reasons(&set12(), u(1), true, &[u(1), u(1)]).any());
    }

    /// #697 (b) itself: the OTHER member's one-chart line is withheld — signing it would vouch
    /// for a possible stranger's medication under this clinician's name.
    #[test]
    fn in_a_doubted_set_a_line_only_on_another_member_is_withheld() {
        let r = wrong_chart_reasons(&set12(), u(1), true, &[u(2)]);
        assert_eq!(
            r,
            WrongChartReasons {
                outside_set: false,
                doubted_link: true
            }
        );
    }

    /// Guards the shared-line case: a line shared between the opened chart and the other member
    /// is withheld for the DOUBTED reason — it lies inside the set, so it is not the outside case.
    #[test]
    fn in_a_doubted_set_a_line_shared_with_the_opened_chart_is_withheld() {
        let r = wrong_chart_reasons(&set12(), u(1), true, &[u(1), u(2)]);
        assert_eq!(
            r,
            WrongChartReasons {
                outside_set: false,
                doubted_link: true
            }
        );
    }

    #[test]
    fn in_a_doubted_set_a_line_with_no_known_chart_is_withheld() {
        assert!(wrong_chart_reasons(&set12(), u(1), true, &[]).doubted_link);
    }

    #[test]
    fn a_line_can_carry_both_reasons() {
        let r = wrong_chart_reasons(&set12(), u(1), true, &[u(2), u(3)]);
        assert_eq!(
            r,
            WrongChartReasons {
                outside_set: true,
                doubted_link: true
            }
        );
    }
}
