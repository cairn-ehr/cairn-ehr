//! R7 (#742): the node's "running" line on a record that is itself already checked. Split from
//! `view_tests.rs` (R5a's goldens), which was already over 500 lines (#789).
use super::*;
use cairn_node::duplicate_check::CheckState;
use uuid::Uuid;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

/// #742 (R7): a duplicate registered after this chart was checked is proposed only when IT is
/// checked. Until then, "no banner" would mean "not yet known" (ED: the same unconscious patient
/// registered twice minutes apart), so a waiting change brings the node's line onto a checked
/// chart.
#[test]
fn a_checked_record_names_a_change_still_waiting_elsewhere() {
    let checks = [ChartCheck {
        chart: id(1),
        pending: Ok(false),
    }];
    let lines = check_lines(
        id(1),
        &checks,
        &[],
        Ok(CheckState::CatchingUp {
            waiting: 1,
            config_recheck: false,
            change_waiting: true,
        }),
    );
    assert_eq!(
        lines,
        vec!["Duplicate check running — 1 chart waiting.".to_string()]
    );
}

/// #742 (R7): a whole-population re-check alone was queued before this chart's own check ran, so
/// that check already saw every waiting chart. A checked record stays quiet through it: the line
/// on every chart for hours would train staff to ignore it.
#[test]
fn a_checked_record_stays_quiet_through_a_config_only_recheck() {
    let checks = [ChartCheck {
        chart: id(1),
        pending: Ok(false),
    }];
    let lines = check_lines(
        id(1),
        &checks,
        &[],
        Ok(CheckState::CatchingUp {
            waiting: 4000,
            config_recheck: true,
            change_waiting: false,
        }),
    );
    assert!(lines.is_empty(), "{lines:?}");
}
