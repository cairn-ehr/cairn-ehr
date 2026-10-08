//! Goldens for every banner sentence (R5a). The wording IS the safety content (principle 3):
//! an absent banner must only ever mean "checked, none open".
use super::*;
use crate::chart_set::MemberLine;
use crate::funnel::view::{ErrorView, Retry};
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
    let e = entry_view(
        id(9),
        EntryFlags::default(),
        Ok(vec![member(9, "Mary SMYTHE")]),
        Ok(meds),
    );
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
            "Recorded: charts {} and {} are different people. This pair is closed on \
             this node, and on every node once the judgement syncs.",
            id(1),
            id(9)
        )
    );
}

#[test]
fn a_partly_failed_judgement_names_the_pair_not_confirmed() {
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
    assert!(r.sentence.ends_with(&format!(
        "Not confirmed for charts {} and {}: held elsewhere.",
        id(2),
        id(9)
    )));
}

/// What `unlink_charts` returns when the connection drops DURING the commit, as the window words
/// it: no refusal marker and no SQLSTATE, so `data_error_from` reads an outage — "not
/// confirmed", `Retry::Now`. The judgement may have been recorded.
fn commit_unknown() -> ErrorView {
    different_people_error_view(&anyhow::anyhow!(
        "commit outcome unknown for event {} — check whether it was recorded before retrying, \
         or the judgement may be recorded twice",
        id(77)
    ))
}

/// Final review I2: a lone commit-unknown pair is its own ErrorView, unchanged — "not
/// confirmed", and the button kept (`Now`), exactly as a lone Link or Unlink outage is.
#[test]
fn a_lone_commit_unknown_pair_is_its_own_error_unchanged() {
    let err = different_people_report(vec![PairResult {
        low: id(1),
        high: id(9),
        outcome: Err(commit_unknown()),
    }])
    .unwrap_err();
    assert_eq!(err, commit_unknown());
    assert_eq!(err.retry, Retry::Now);
    assert_eq!(
        err.text,
        format!(
            "The \"different people\" judgement was not confirmed: commit outcome unknown for \
             event {} — check whether \
             it was recorded before retrying, or the judgement may be recorded twice",
            id(77)
        )
    );
}

/// Final review I2: a commit-unknown pair beside a recorded one is "Not confirmed" — never
/// "NOT recorded", and never "it stays on the banner": it may have been recorded, and only the
/// banner's re-read (`reload`) says which.
#[test]
fn a_commit_unknown_pair_beside_a_recorded_one_is_not_confirmed_never_not_recorded() {
    let r = different_people_report(vec![
        PairResult {
            low: id(1),
            high: id(8),
            outcome: Ok(LinkEffect::TookEffect),
        },
        PairResult {
            low: id(2),
            high: id(9),
            outcome: Err(commit_unknown()),
        },
    ])
    .unwrap();
    assert!(
        r.reload,
        "the banner must re-read to learn what the lost commit did"
    );
    assert_eq!(
        r.sentence,
        format!(
            "Recorded: charts {} and {} are different people. This pair is closed on this node, \
             and on every node once the judgement syncs. Not confirmed for charts {} and {}: The \
             \"different people\" judgement was not confirmed: commit outcome unknown for event {} \
             — check whether it was recorded before retrying, or the judgement may be recorded \
             twice.",
            id(1),
            id(8),
            id(2),
            id(9),
            id(77)
        )
    );
}

/// Final review I2 (the #713 hazard): when EVERY pair failed, a refused first pair must not
/// hide a commit-unknown second one. One ErrorView names both, and carries the commit-unknown's
/// `Now` — a `Never` would claim a decided outcome that may not exist.
#[test]
fn every_failed_pair_is_named_and_a_commit_unknown_is_never_hidden_behind_a_verdict() {
    let err = different_people_report(vec![
        PairResult {
            low: id(1),
            high: id(8),
            outcome: Err(refused(
                "The \"different people\" judgement was refused: held elsewhere",
            )),
        },
        PairResult {
            low: id(2),
            high: id(9),
            outcome: Err(commit_unknown()),
        },
    ])
    .unwrap_err();
    assert_eq!(err.retry, Retry::Now);
    assert_eq!(
        err.text,
        format!(
            "Not confirmed for charts {} and {}: The \"different people\" judgement was refused: \
             held elsewhere. Not confirmed for charts {} and {}: The \"different people\" \
             judgement was not confirmed: commit outcome unknown for event {} — check whether it \
             was recorded before retrying, or the judgement may be recorded twice.",
            id(1),
            id(8),
            id(2),
            id(9),
            id(77)
        )
    );
}

/// The combined retry is the most retryable class among the failed pairs, whatever their order.
#[test]
fn the_combined_retry_is_the_most_retryable_among_the_failed_pairs() {
    let fail = |n: u128, retry: Retry| PairResult {
        low: id(n),
        high: id(n + 50),
        outcome: Err(ErrorView {
            text: "x".into(),
            retry,
        }),
    };
    let combined = |rs: Vec<PairResult>| different_people_report(rs).unwrap_err().retry;
    assert_eq!(
        combined(vec![fail(1, Retry::Now), fail(2, Retry::Never)]),
        Retry::Now
    );
    assert_eq!(
        combined(vec![fail(1, Retry::Never), fail(2, Retry::AfterOperator)]),
        Retry::AfterOperator
    );
    assert_eq!(
        combined(vec![fail(1, Retry::Never), fail(2, Retry::Never)]),
        Retry::Never
    );
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

/// Silent-failure review M1: "Different people" was pressed over two charts that were never
/// linked, so its failure must never read "The unlink was …" — that would send the clinician
/// looking for a link to undo.
#[test]
fn a_different_people_failure_is_never_worded_as_an_unlink() {
    let e = different_people_error_view(&anyhow::anyhow!("connection reset"));
    assert_eq!(
        e.text,
        "The \"different people\" judgement was not confirmed: connection reset"
    );
    assert_eq!(e.retry, Retry::Now);
}

#[test]
fn still_joined_and_outranked_sentences_are_pinned() {
    let r = different_people_report(vec![
        PairResult {
            low: id(1),
            high: id(8),
            outcome: Ok(LinkEffect::StillJoined),
        },
        PairResult {
            low: id(2),
            high: id(9),
            outcome: Ok(LinkEffect::Outranked),
        },
    ])
    .unwrap();
    assert_eq!(
        r.sentence,
        format!(
            "Recorded that charts {} and {} are different people — but they read as one record \
             through other links, so nothing was split. The links joining them are listed under \
             \"How these charts are linked\". Recorded that charts {} and {} are different people, \
             but a judgement already standing for that pair outranks it; the charts stay as that \
             judgement left them.",
            id(1),
            id(8),
            id(2),
            id(9)
        )
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
    assert_eq!(v.heading, "Accepted as the same person — not yet linked");
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
    assert_eq!(v.heading, "Possible duplicate — not yet reviewed");
    assert!(v.notes.contains(
        &"Recorded as not the same person, without a clinician's confirmation on record here."
            .to_string()
    ));
    assert!(v.offers_different_people);
}
