use super::*;
use crate::funnel::rows::PersonRowView;
use crate::funnel::view::CandidateView;
use cairn_node::duplicate_check::CheckState;
use uuid::Uuid;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

/// The proposal's two charts (newer side's, older side's) for an entry whose sides are not read.
fn charts() -> (Uuid, Uuid) {
    (id(1), id(2))
}

/// A side's row: one chart per `(n, name, age)`.
fn row(members: &[(u128, &str, &str)]) -> PersonRowView {
    PersonRowView {
        label: None,
        members: members
            .iter()
            .map(|&(n, name, age)| CandidateView {
                patient_id: id(n).to_string(),
                name: name.into(),
                age: age.into(),
                trust: "confirmed".into(),
            })
            .collect(),
    }
}

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
    let v = entry_view(flags, "auto_candidate", Ok(None), Ok(None), charts(), None);
    assert_eq!(v.status, "Possible duplicate — not yet reviewed");
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
fn an_accepted_entry_uses_the_banners_accepted_wording() {
    let flags = crate::duplicates::view::EntryFlags {
        accepted: true,
        ..Default::default()
    };
    let v = entry_view(flags, "review", Ok(None), Ok(None), charts(), None);
    assert_eq!(v.status, crate::duplicates::view::ACCEPTED_NOT_LINKED);
}

/// Accepted AND disputed is a real state — a clinician said "same person" here while another
/// writer's un-attested "different people" still stands (principle 4: shown, never resolved by
/// the window). The entry carries both: the accepted status, and the dispute note.
#[test]
fn an_accepted_and_disputed_entry_shows_both() {
    let flags = crate::duplicates::view::EntryFlags {
        accepted: true,
        disputed: true,
        ..Default::default()
    };
    let v = entry_view(flags, "review", Ok(None), Ok(None), charts(), None);
    assert_eq!(v.status, crate::duplicates::view::ACCEPTED_NOT_LINKED);
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
        charts(),
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
    let v = entry_view(
        Default::default(),
        "review",
        Ok(None),
        Ok(None),
        charts(),
        None,
    );
    assert_eq!(v.older.row, None);
    assert_eq!(
        v.older.error.as_deref(),
        Some("This record could not be read here: no charts were found for it.")
    );
}

/// #796 (#737, one tray over): a screen reader's heading list must tell the entries apart. A
/// tray entry is a PAIR of records, so its heading names both — each by its first chart's name,
/// age and whole chart id. Two entries can share one record (A with B, A with C), and an exact
/// namesake pair is the matcher's strongest case, so only the ids make every heading unique.
#[test]
fn each_tray_entry_is_headed_by_both_of_its_records() {
    let heading = |older: u128| {
        entry_view(
            Default::default(),
            "review",
            Ok(Some(row(&[(1, "Mary SMYTHE", "74 y")]))),
            Ok(Some(row(&[(older, "Mary SMYTHE", "74 y")]))),
            (id(1), id(older)),
            None,
        )
        .heading
    };
    assert_eq!(
        heading(2),
        format!(
            "Mary SMYTHE · 74 y · chart {} — and Mary SMYTHE · 74 y · chart {}",
            id(1),
            id(2)
        )
    );
    assert_ne!(heading(2), heading(3), "one shared record, two entries");
}

/// A linked side counts its other charts, as the banner does; an unread side is headed by the
/// proposal's own chart on that side, never left out of the heading.
#[test]
fn a_tray_heading_counts_linked_charts_and_names_an_unread_side_by_its_chart() {
    let v = entry_view(
        Default::default(),
        "review",
        Ok(Some(row(&[(1, "A", "age not recorded"), (4, "B", "3 y")]))),
        Err("nope".into()),
        charts(),
        None,
    );
    assert_eq!(
        v.heading,
        format!(
            "A · age not recorded · chart {} · and 1 linked chart — and Chart {}",
            id(1),
            id(2)
        )
    );
    let unread_both = entry_view(
        Default::default(),
        "review",
        Ok(None),
        Ok(None),
        charts(),
        None,
    );
    assert_eq!(
        unread_both.heading,
        format!("Chart {} — and Chart {}", id(1), id(2))
    );
}
