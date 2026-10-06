//! Goldens for every banner sentence (R5a). The wording IS the safety content (principle 3):
//! an absent banner must only ever mean "checked, none open".
use super::*;
use crate::chart_set::MemberLine;
use cairn_node::chart_link::LinkEffect;
use cairn_node::duplicate_check::CheckState;
use uuid::Uuid;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

fn member(n: u128, name: &str) -> MemberLine {
    MemberLine {
        patient_id: id(n).to_string(),
        name: name.into(),
        text: format!(
            "{name} · born 1950-01-07 · identity confirmed · chart {}",
            id(n)
        ),
    }
}

/// Review Focus 5 (golden): a never-proposed, checked chart gets NOTHING — the webview hides
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
        DuplicateSection::default()
    );
}

/// Review Focus 5: a failed proposal read is an error line, never an empty banner.
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
    let e = entry_view(id(9), false, Ok(vec![member(9, "Mary SMYTHE")]), Ok(meds));
    assert_eq!(e.heading, "Possible duplicate — not yet reviewed");
    assert_eq!(
        e.identity_lines,
        vec![format!(
            "Mary SMYTHE · born 1950-01-07 · identity confirmed · chart {}",
            id(9)
        )]
    );
    assert_eq!(
        e.medications_heading,
        "On the other chart — not part of this record"
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
    let e = entry_view(id(9), true, Ok(vec![member(9, "X")]), Err("x".into()));
    assert!(e.notes.contains(&VETO_NOTE.to_string()));
}

#[test]
fn unread_parts_of_an_entry_are_worded_and_the_entry_is_kept() {
    let e = entry_view(id(9), false, Err("timeout".into()), Err("sealed".into()));
    assert_eq!(e.identity_lines, vec![format!("chart {}", id(9))]);
    assert!(e
        .notes
        .iter()
        .any(|n| n == "The other chart's name and date of birth could not be read: timeout"));
    assert_eq!(e.medications, Vec::<String>::new());
    assert_eq!(
        e.medication_notes,
        vec!["Its medications could not be read here: sealed".to_string()]
    );
}

#[test]
fn entries_beyond_the_cap_are_counted_never_dropped_silently() {
    let e = entry_view(id(9), false, Ok(vec![]), Err("x".into()));
    let s = section_view(Ok((vec![e.clone(), e.clone(), e], 5)), vec![]);
    assert_eq!(
        s.more.as_deref(),
        Some("2 more possible duplicates of this record are not shown here.")
    );
    let e = entry_view(id(9), false, Ok(vec![]), Err("x".into()));
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
        Ok(CheckState::CatchingUp {
            waiting: 2,
            config_recheck: false,
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
fn different_people_reports_each_pair_and_reloads() {
    let r = different_people_report(vec![PairResult {
        low: id(1),
        high: id(9),
        outcome: Ok(LinkEffect::TookEffect),
    }])
    .unwrap();
    assert!(r.reload);
    assert_eq!(
        r.sentence,
        format!(
            "Recorded: charts {} and {} are different people. This possible duplicate is closed on \
             this node, and on every node once the judgement syncs.",
            id(1),
            id(9)
        )
    );
}

#[test]
fn a_partly_failed_judgement_names_the_pair_left_open() {
    let r = different_people_report(vec![
        PairResult {
            low: id(1),
            high: id(8),
            outcome: Ok(LinkEffect::TookEffect),
        },
        PairResult {
            low: id(2),
            high: id(9),
            outcome: Err(refused("held elsewhere")),
        },
    ])
    .unwrap();
    assert!(r.sentence.contains(&format!(
        "NOT recorded for charts {} and {}: held elsewhere — it stays on the banner.",
        id(2),
        id(9)
    )));
}

#[test]
fn a_wholly_failed_judgement_is_the_refusal_itself() {
    let err = different_people_report(vec![PairResult {
        low: id(1),
        high: id(9),
        outcome: Err(refused("held elsewhere")),
    }])
    .unwrap_err();
    assert_eq!(err.text, "held elsewhere");
    assert_eq!(
        different_people_report(vec![]).unwrap_err().text,
        NOTHING_OPEN
    );
}
