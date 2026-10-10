use super::*;
use cairn_node::duplicate_check::CheckState;

#[test]
fn zero_on_a_current_node_hides_the_tray() {
    let v = tray_count_view(Ok(0), Ok(CheckState::Current { last_ran: None }));
    assert_eq!(
        v,
        TrayCountView {
            summary: None,
            status_line: None
        }
    );
}

#[test]
fn zero_on_a_node_that_never_ran_is_shown_with_the_reason() {
    let state = CheckState::NeverRun { waiting: 0 };
    let v = tray_count_view(Ok(0), Ok(state.clone()));
    assert_eq!(v.summary.as_deref(), Some("Possible duplicates (0)"));
    assert_eq!(
        v.status_line,
        Some(cairn_node::duplicate_check::status_line(&state))
    );
}

#[test]
fn a_count_on_a_current_node_has_no_status_line() {
    let v = tray_count_view(Ok(3), Ok(CheckState::Current { last_ran: None }));
    assert_eq!(v.summary.as_deref(), Some("Possible duplicates (3)"));
    assert_eq!(v.status_line, None);
}

#[test]
fn a_failed_count_is_worded_never_hidden() {
    let v = tray_count_view(
        Err("boom".into()),
        Ok(CheckState::Current { last_ran: None }),
    );
    assert_eq!(
        v.summary.as_deref(),
        Some("Possible duplicates — could not be checked: boom")
    );
}

#[test]
fn an_unreadable_status_is_worded() {
    let v = tray_count_view(Ok(0), Err("gone".into()));
    assert_eq!(v.summary.as_deref(), Some("Possible duplicates (0)"));
    assert_eq!(
        v.status_line.as_deref(),
        Some("Duplicate check status unknown: gone")
    );
}

#[test]
fn more_counts_the_entries_not_shown() {
    assert_eq!(more_line(20, 20), None);
    assert_eq!(
        more_line(20, 21).as_deref(),
        Some("1 more possible duplicate, older than these.")
    );
    assert_eq!(
        more_line(20, 25).as_deref(),
        Some("5 more possible duplicates, older than these.")
    );
}

#[test]
fn an_entry_names_both_sides_and_its_notes() {
    let flags = crate::duplicates::view::EntryFlags {
        vetoed: true,
        accepted: false,
        disputed: true,
    };
    let v = entry_view(flags, "auto_candidate", Ok(None), Ok(None), None);
    assert_eq!(v.heading, "Possible duplicate — not yet reviewed");
    assert_eq!(v.newer_label, "Registered more recently");
    assert_eq!(v.older_label, "Already on file");
    assert_eq!(
        v.notes,
        vec![
            "The matcher rates this a strong match.".to_string(),
            crate::duplicates::view::VETO_NOTE.to_string(),
            crate::duplicates::view::DISPUTED_NOTE.to_string(),
        ]
    );
    assert_eq!(v.open_chart, None);
}

#[test]
fn an_accepted_entry_uses_the_banners_heading() {
    let flags = crate::duplicates::view::EntryFlags {
        accepted: true,
        ..Default::default()
    };
    let v = entry_view(flags, "review", Ok(None), Ok(None), None);
    assert_eq!(v.heading, crate::duplicates::view::ACCEPTED_HEADING);
}

/// Accepted AND disputed is a real state — a clinician said "same person" here while another
/// writer's un-attested "different people" still stands (principle 4: shown, never resolved by
/// the window). The entry carries both: the accepted heading, and the dispute note.
#[test]
fn an_accepted_and_disputed_entry_shows_both() {
    let flags = crate::duplicates::view::EntryFlags {
        accepted: true,
        disputed: true,
        ..Default::default()
    };
    let v = entry_view(flags, "review", Ok(None), Ok(None), None);
    assert_eq!(v.heading, crate::duplicates::view::ACCEPTED_HEADING);
    assert_eq!(
        v.notes,
        vec![crate::duplicates::view::DISPUTED_NOTE.to_string()]
    );
}

#[test]
fn an_unreadable_side_is_worded_and_never_dropped() {
    let v = entry_view(
        Default::default(),
        "review",
        Err("nope".into()),
        Ok(None),
        None,
    );
    assert_eq!(v.newer.row, None);
    assert_eq!(
        v.newer.error.as_deref(),
        Some("This record could not be read here: nope")
    );
}

#[test]
fn a_failed_list_is_an_error_line() {
    let v = worklist_view(Err("down".into()));
    assert_eq!(
        v.error.as_deref(),
        Some("Could not read the possible duplicates: down")
    );
    assert!(v.entries.is_empty());
}

/// A side read with NO chart is a worded line, never a silent blank.
#[test]
fn a_side_with_no_chart_is_worded_never_blank() {
    let v = entry_view(Default::default(), "review", Ok(None), Ok(None), None);
    assert_eq!(v.older.row, None);
    assert_eq!(
        v.older.error.as_deref(),
        Some("This record could not be read here: no charts were found for it.")
    );
}
