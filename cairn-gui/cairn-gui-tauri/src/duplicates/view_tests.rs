//! Goldens for every banner sentence (R5a). The wording IS the safety content (principle 3):
//! an absent banner must only ever mean "checked, none open".
use super::*;
use crate::chart_set::MemberLine;
use cairn_node::duplicate_check::CheckState;
use uuid::Uuid;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

fn member(n: u128, name: &str) -> MemberLine {
    MemberLine {
        patient_id: id(n).to_string(),
        name: name.into(),
        born: "born 1950-01-07".into(),
        text: format!(
            "{name} · born 1950-01-07 · identity confirmed · chart {}",
            id(n)
        ),
    }
}

/// A never-proposed, checked chart gets NOTHING — the webview hides
/// the section, so the chart reads exactly as before R5a.
#[test]
fn a_checked_chart_with_no_proposal_has_an_empty_section() {
    let checks = [ChartCheck {
        chart: id(1),
        pending: Ok(false),
    }];
    let lines = check_lines(
        id(1),
        &checks,
        &[],
        Ok(CheckState::Current { last_ran: None }),
    );
    assert_eq!(
        section_view(Ok((vec![], 0)), lines),
        DuplicateSection::checked_none_open()
    );
}

/// The empty section is a CLAIM ("checked, none open"), so it has a name and no `Default`: a
/// placeholder `Default::default()` would have asserted "no duplicates" without saying so.
#[test]
fn checked_none_open_is_the_all_empty_section() {
    let s = DuplicateSection::checked_none_open();
    assert!(s.entries.is_empty() && s.more.is_none() && s.error.is_none());
    assert!(s.check_lines.is_empty());
}

/// A failed proposal read is an error line, never an empty banner.
#[test]
fn a_failed_proposal_read_is_worded_never_empty() {
    let s = section_view(Err("connection reset".into()), vec![]);
    assert_eq!(
        s.error.as_deref(),
        Some("Could not check for possible duplicates: connection reset")
    );
    assert!(s.entries.is_empty());
}

#[test]
fn an_entry_names_the_other_chart_and_its_current_medications() {
    // The shared fixture carries both current and ceased drugs (link/view_tests.rs relies on it).
    let source = cairn_medication_view::fixtures::sample_chart();
    let active = source
        .rows
        .iter()
        .filter(|r| r.status == cairn_medication_view::MedicationStatus::Active)
        .count();
    assert!(active > 0 && active < source.rows.len());
    let meds = cairn_gui_tab_medications::view::build_view(&source);
    let e = entry_view(
        id(9),
        EntryFlags::default(),
        Ok(vec![member(9, "Mary SMYTHE")]),
        Ok(meds),
    );
    assert_eq!(
        e.heading,
        format!("Mary SMYTHE · born 1950-01-07 · chart {}", id(9))
    );
    assert_eq!(e.status, "Possible duplicate — not yet reviewed");
    assert_eq!(
        e.identity_lines,
        vec![format!(
            "Mary SMYTHE · born 1950-01-07 · identity confirmed · chart {}",
            id(9)
        )]
    );
    assert_eq!(
        e.medications_heading,
        "On the other record — not part of this one until linked"
    );
    assert_eq!(
        e.medications.len(),
        active,
        "ceased lines are not the other chart's current drugs"
    );
    assert!(e.notes.is_empty());
    assert_eq!(e.review_chart, id(9).to_string());
}

#[test]
fn a_vetoed_entry_says_facts_disagree_and_points_at_review() {
    let e = entry_view(
        id(9),
        EntryFlags {
            vetoed: true,
            ..Default::default()
        },
        Ok(vec![member(9, "X")]),
        Err("x".into()),
    );
    assert!(e.notes.contains(&VETO_NOTE.to_string()));
}

#[test]
fn unread_parts_of_an_entry_are_worded_and_the_entry_is_kept() {
    let e = entry_view(
        id(9),
        EntryFlags::default(),
        Err("timeout".into()),
        Err("sealed".into()),
    );
    assert_eq!(e.identity_lines, vec![format!("chart {}", id(9))]);
    assert!(e
        .notes
        .iter()
        .any(|n| n == "The other record's name and date of birth could not be read: timeout"));
    assert_eq!(e.medications, Vec::<String>::new());
    assert_eq!(
        e.medication_notes,
        vec!["Its medications could not be read here: sealed".to_string()]
    );
}

#[test]
fn entries_beyond_the_cap_are_counted_never_dropped_silently() {
    let e = entry_view(id(9), EntryFlags::default(), Ok(vec![]), Err("x".into()));
    let s = section_view(Ok((vec![e.clone(), e.clone(), e.clone()], 5)), vec![]);
    assert_eq!(
        s.more.as_deref(),
        Some("2 more possible duplicates of this record are not shown here.")
    );
    let s = section_view(Ok((vec![e], 2)), vec![]);
    assert_eq!(
        s.more.as_deref(),
        Some("1 more possible duplicate of this record is not shown here.")
    );
}

#[test]
fn a_pending_member_is_named_and_the_node_status_explains_it() {
    let checks = [
        ChartCheck {
            chart: id(1),
            pending: Ok(true),
        },
        ChartCheck {
            chart: id(2),
            pending: Ok(true),
        },
        ChartCheck {
            chart: id(3),
            pending: Err("boom".into()),
        },
    ];
    let lines = check_lines(
        id(1),
        &checks,
        &[member(2, "Ann LEE")],
        // No change waits elsewhere: the pending lines alone bring the status line, as before R7.
        Ok(CheckState::CatchingUp {
            waiting: 2,
            config_recheck: false,
            change_waiting: false,
        }),
    );
    assert_eq!(
        lines,
        vec![
            "This chart: duplicate check not yet run since its identity details last changed.".to_string(),
            format!("Linked chart Ann LEE (chart {}): duplicate check not yet run since its identity details last changed.", id(2)),
            format!("Linked chart {}: duplicate check status unknown — boom", id(3)),
            "Duplicate check running — 2 charts waiting.".to_string(),
        ]
    );
}

#[test]
fn a_stalled_node_says_so_even_when_this_record_is_checked() {
    let checks = [ChartCheck {
        chart: id(1),
        pending: Ok(false),
    }];
    let lines = check_lines(
        id(1),
        &checks,
        &[],
        Ok(CheckState::Stalled {
            waiting: 3,
            last_ran: Some("09:15".into()),
        }),
    );
    assert_eq!(
        lines,
        vec!["Duplicate check is behind — last ran 09:15; 3 charts waiting.".to_string()]
    );
}

#[test]
fn an_unreadable_node_status_is_never_omitted() {
    let checks = [ChartCheck {
        chart: id(1),
        pending: Ok(false),
    }];
    let lines = check_lines(id(1), &checks, &[], Err("denied".into()));
    assert_eq!(
        lines,
        vec!["Duplicate check status unknown: denied".to_string()]
    );
}

#[test]
fn fixture_mode_says_no_check_has_run() {
    let s = fixture_section();
    assert_eq!(
        s.check_lines,
        vec!["Duplicate check has never run on this node.".to_string()]
    );
    assert!(s.entries.is_empty() && s.error.is_none());
}

#[test]
fn a_pending_unheld_member_on_a_current_node_gets_no_contradicting_status() {
    let checks = [
        ChartCheck {
            chart: id(1),
            pending: Ok(false),
        },
        ChartCheck {
            chart: id(2),
            pending: Ok(true),
        },
    ];
    let lines = check_lines(
        id(1),
        &checks,
        &[member(2, "Ann LEE")],
        Ok(CheckState::Current {
            last_ran: Some("09:15".into()),
        }),
    );
    assert_eq!(
        lines,
        vec![format!(
            "Linked chart Ann LEE (chart {}): duplicate check not yet run since its identity details last changed.",
            id(2)
        )]
    );
}

#[test]
fn an_entry_with_no_identity_lines_says_so() {
    let e = entry_view(id(9), EntryFlags::default(), Ok(vec![]), Err("x".into()));
    assert_eq!(e.identity_lines, vec![format!("chart {}", id(9))]);
    assert!(e
        .notes
        .contains(&"No name or date of birth is recorded for the other record.".to_string()));
}

/// Type review: the entry's identity lines are read over `other_record` under the lock; its
/// medications are read after it, through the chart-open read, which re-derives the record. A
/// link landing in between would draw one set's names over another set's drugs — so a list read
/// over a different set is worded as unread, never drawn.
#[test]
fn medications_read_over_a_different_record_are_not_drawn() {
    let record = ChartSet::new([id(8), id(9)]).unwrap();
    let mut list = cairn_medication_view::fixtures::sample_chart();
    list.charts = ChartSet::single(id(8));
    let err = medications_of(&record, Ok(list.clone())).unwrap_err();
    assert_eq!(
        err,
        "the other record changed while the banner was read — reload the chart"
    );
    list.charts = record.clone();
    assert!(medications_of(&record, Ok(list)).is_ok());
    assert_eq!(
        medications_of(&record, Err("sealed".into())).unwrap_err(),
        "sealed"
    );
}

/// #736: a pair a human already accepted as the same person is not "not yet reviewed", and
/// must not offer "Different people" (it would silently overrule that human).
#[test]
fn an_accepted_entry_is_worded_by_its_status_and_offers_no_different_people() {
    let flags = EntryFlags {
        accepted: true,
        ..Default::default()
    };
    let v = entry_view(Uuid::from_u128(9), flags, Ok(vec![]), Err("x".into()));
    assert_eq!(v.status, "Accepted as the same person — not yet linked");
    assert!(!v.offers_different_people);
}

/// ADR-0078: another writer's un-attested unlink is shown, never hidden; both judgements stay.
#[test]
fn a_disputed_entry_says_so_and_still_offers_both_judgements() {
    let flags = EntryFlags {
        disputed: true,
        ..Default::default()
    };
    let v = entry_view(Uuid::from_u128(9), flags, Ok(vec![]), Err("x".into()));
    assert_eq!(v.status, "Possible duplicate — not yet reviewed");
    assert!(v.notes.contains(
        &"Recorded as not the same person, without a clinician's confirmation on record here."
            .to_string()
    ));
    assert!(v.offers_different_people);
}

/// #737: a screen reader's heading list must tell the entries apart, so each heading names
/// whose record it is — the name and date of birth a clinician reads, and the chart id.
#[test]
fn each_entry_is_headed_by_whose_record_it_is() {
    let mut namesake = member(8, "Mary SMYTHE");
    namesake.born = "born 1951-02-03".into();
    let heading = |n: u128, line: MemberLine| {
        entry_view(
            id(n),
            EntryFlags::default(),
            Ok(vec![line]),
            Err("x".into()),
        )
        .heading
    };
    assert_eq!(
        heading(9, member(9, "Mary SMYTHE")),
        format!("Mary SMYTHE · born 1950-01-07 · chart {}", id(9))
    );
    assert_eq!(
        heading(8, namesake),
        format!("Mary SMYTHE · born 1951-02-03 · chart {}", id(8))
    );
}

/// The matcher's STRONGEST case: one patient registered three times, so the open chart's banner
/// shows two other records with the same name and date of birth. Their headings must still
/// differ, or the heading list is N identical lines again (PR #795 review).
#[test]
fn exact_namesakes_are_told_apart_by_their_charts() {
    let heading = |n: u128| {
        entry_view(
            id(n),
            EntryFlags::default(),
            Ok(vec![member(n, "Mary SMYTHE")]),
            Err("x".into()),
        )
        .heading
    };
    assert_ne!(heading(8), heading(9));
}

/// A chart this node does not hold heads its entry with its worded absences — never a blank —
/// and its id, so two such entries still differ.
#[test]
fn a_not_held_chart_heads_its_entry_with_its_absences_and_its_id() {
    let line = crate::chart_set::member_line(&cairn_node::patient::person::ChartIdentity {
        patient_id: id(9),
        held: false,
        name: None,
        birth_date: None,
        trust: "unknown".into(),
    });
    let e = entry_view(
        id(9),
        EntryFlags::default(),
        Ok(vec![line]),
        Err("x".into()),
    );
    assert_eq!(
        e.heading,
        format!("(name unknown) · date of birth unknown · chart {}", id(9))
    );
}

/// A linked other record is headed by its first chart and says how many more it holds; every
/// chart still has its own identity line below.
#[test]
fn a_linked_other_records_heading_counts_its_other_charts() {
    let heading = |lines: Vec<MemberLine>| {
        entry_view(id(9), EntryFlags::default(), Ok(lines), Err("x".into())).heading
    };
    assert_eq!(
        heading(vec![member(9, "Mary SMYTHE"), member(10, "Mary SMITH")]),
        format!(
            "Mary SMYTHE · born 1950-01-07 · chart {} · and 1 linked chart",
            id(9)
        )
    );
    assert_eq!(
        heading(vec![member(9, "A"), member(10, "B"), member(11, "C")]),
        format!(
            "A · born 1950-01-07 · chart {} · and 2 linked charts",
            id(9)
        )
    );
}

/// No name to head an entry with (unread, or never recorded): the chart id still tells the
/// entries apart, and the note below says why there is no name.
#[test]
fn an_entry_with_no_identity_is_headed_by_its_chart() {
    let unread = entry_view(
        id(9),
        EntryFlags::default(),
        Err("gone".into()),
        Err("x".into()),
    );
    let empty = entry_view(id(8), EntryFlags::default(), Ok(vec![]), Err("x".into()));
    assert_eq!(unread.heading, format!("Chart {}", id(9)));
    assert_eq!(empty.heading, format!("Chart {}", id(8)));
    assert_eq!(unread.status, "Possible duplicate — not yet reviewed");
}
