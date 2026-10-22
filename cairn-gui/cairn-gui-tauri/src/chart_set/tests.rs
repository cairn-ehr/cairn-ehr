//! Tests for `chart_set/mod.rs` (moved out of line, #712 item 3): the set check, the member
//! lines and their absence words, a pane that keeps its list when an identity or link read
//! fails, and which threads a cease from this chart may touch.
use super::*;
use crate::duplicates::view::DuplicateSection;
use crate::link::record_links::RecordLinks;
use cairn_medication_view::ChartSet;
use uuid::Uuid;

fn ids(v: &[u128]) -> Vec<String> {
    v.iter().map(|n| Uuid::from_u128(*n).to_string()).collect()
}
fn set(v: &[u128]) -> ChartSet {
    ChartSet::new(v.iter().map(|n| Uuid::from_u128(*n))).unwrap()
}

#[test]
fn the_same_set_in_any_order_is_accepted_and_returned() {
    assert_eq!(
        check_displayed_set(&set(&[1, 2]), &ids(&[2, 1])),
        Ok(set(&[1, 2]))
    );
}

#[test]
fn a_combined_sign_off_report_names_the_charts_it_read() {
    let message = signed_across_message(&set(&[1, 2])).unwrap();
    assert!(message.contains("2 linked charts"), "{message}");
    assert!(
        message.contains("signed on the chart it was recorded on"),
        "{message}"
    );
    assert_eq!(signed_across_message(&set(&[1])), None);
}

/// A changed set is a TYPE, so callers cannot confuse it with an unreadable list by comparing
/// sentences, and its sentence names no remedy the window cannot perform.
#[test]
fn a_changed_set_is_a_typed_change() {
    let err = check_displayed_set(&set(&[1, 2]), &ids(&[1])).unwrap_err();
    assert_eq!(err, ChartSetError::Changed);
    assert!(err.text().contains("linked charts changed"));
    assert!(!err.text().contains("reload"));
    assert_eq!(err.text(), crate::chart_set::CHANGED);
}

/// The view a change becomes: the screen is behind the record, so the window re-reads by itself.
/// An unreadable list is a window fault, not a change: the window does NOT re-read by itself
/// (`Never`) — its sentence names the button that mends it.
#[test]
fn a_change_is_an_after_reread_view_and_an_unreadable_list_is_not() {
    use crate::funnel::view::{Recorded, Retry};
    assert_eq!(
        ChartSetError::Changed.into_view().retry,
        Retry::AfterReread(Recorded::No)
    );
    assert_eq!(ChartSetError::Unreadable.into_view().retry, Retry::Never);
}

#[test]
fn an_unparseable_id_is_refused() {
    let err = check_displayed_set(&set(&[1]), &["not-a-uuid".to_string()]).unwrap_err();
    assert_eq!(err, ChartSetError::Unreadable);
    assert!(err.text().contains("could not tell which charts"));
    // The way out is one press (#797 final review, M7): a re-read redraws the set every command
    // sends back. Not "reopen the chart", which costs three acts for the same read.
    assert!(err.text().contains("press \"Read this chart again\""));
    assert!(!err.text().contains("reopen"));
}

/// A command that names no chart at all cannot say what it saw, so it must not act.
#[test]
fn an_empty_displayed_list_is_refused() {
    let err = check_displayed_set(&set(&[1]), &[]).unwrap_err();
    assert_eq!(err, ChartSetError::Unreadable);
}

#[test]
fn a_member_line_names_every_fact_and_the_chart() {
    let id = Uuid::from_u128(7);
    let line = member_line(&ChartIdentity {
        patient_id: id,
        held: true,
        name: Some("SMYTHE, Jo".into()),
        birth_date: Some("1970-03-04".into()),
        trust: "confirmed".into(),
    });
    assert_eq!(line.patient_id, id.to_string());
    assert_eq!(
        line.text,
        format!("SMYTHE, Jo · born 1970-03-04 · identity confirmed · chart {id}")
    );
    assert_eq!(line.born, "born 1970-03-04");
}

/// Principle 4: absence is named, never a blank that reads like a rendering fault.
#[test]
fn a_member_line_names_what_was_never_recorded() {
    let id = Uuid::from_u128(8);
    let line = member_line(&ChartIdentity {
        patient_id: id,
        held: true,
        name: None,
        birth_date: None,
        trust: "unconfirmed".into(),
    });
    assert_eq!(
        line.text,
        format!(
            "(no name recorded) · date of birth not recorded · identity unconfirmed \
             · chart {id}"
        )
    );
    assert_eq!(line.born, "date of birth not recorded");
}

/// `MemberLine::name` carries the absence word, so a link line never names a chart blank.
#[test]
fn a_member_name_carries_the_absence_word() {
    let mk = |held: bool| ChartIdentity {
        patient_id: Uuid::from_u128(5),
        held,
        name: None,
        birth_date: None,
        trust: "unknown".into(),
    };
    assert_eq!(member_line(&mk(true)).name, "(no name recorded)");
    assert_eq!(member_line(&mk(false)).name, "(name unknown)");
}

/// A linked chart whose registration has not reached this node: an absent fact is
/// UNKNOWN, not "not recorded", and the line carries the `unknown` trust `trust_of` gives
/// it rather than a borrowed "confirmed".
#[test]
fn a_member_line_says_when_the_chart_is_not_held_here() {
    let id = Uuid::from_u128(9);
    let line = member_line(&ChartIdentity {
        patient_id: id,
        held: false,
        name: None,
        birth_date: None,
        trust: "unknown".into(),
    });
    assert_eq!(
        line.text,
        format!(
            "(name unknown) · date of birth unknown · identity unknown \
             · registration not yet received on this node · chart {id}"
        )
    );
    assert_eq!(
        (line.name.as_str(), line.born.as_str()),
        ("(name unknown)", "date of birth unknown")
    );
    // Static messages, never the line itself: a member line carries a chart's name and
    // date of birth BY DESIGN, and CodeQL's cleartext-logging rule rightly treats echoing
    // it into panic output as a leak — synthetic fixture or not.
    assert!(
        !line.text.contains("recorded"),
        "an absent fact on an unregistered chart reads as unknown, never 'not recorded'"
    );
}

/// Other events about an unregistered chart can arrive first (a demographic stream does
/// not create its `patient_chart` row). A name that DID arrive is shown, not discarded.
#[test]
fn a_member_line_keeps_a_name_that_arrived_before_the_registration() {
    let id = Uuid::from_u128(10);
    let line = member_line(&ChartIdentity {
        patient_id: id,
        held: false,
        name: Some("Jo BLOGGS".into()),
        birth_date: None,
        trust: "unknown".into(),
    });
    assert!(
        line.text.starts_with("Jo BLOGGS · "),
        "the name that arrived leads the line"
    );
    assert!(
        line.text.contains("registration not yet received"),
        "the line says the registration is missing"
    );
}

/// Availability over consistency: a clinician must always be able to READ. If the member
/// identities cannot be read, the medication list is still shown — with a warning that says
/// the list is combined, over how many charts, that no member's identity state is known, and
/// why — rather than the whole chart failing to open.
#[test]
fn a_failed_identity_read_keeps_the_list_and_says_so() {
    let mut list = cairn_medication_view::fixtures::sample_chart();
    list.charts = set(&[1, 0xB]);
    let pane = chart_pane(
        &list,
        Uuid::from_u128(1),
        Err("connection reset".into()),
        Ok(RecordLinks::default()),
        DuplicateSection::checked_none_open(),
        None,
    );
    assert_eq!(pane.list.rows.len(), list.rows.len(), "the list is kept");
    assert!(pane.members.is_empty());
    let warning = pane
        .members_error
        .expect("the failure is reported, never implied");
    assert!(warning.contains("could not be read"), "{warning}");
    assert!(warning.contains("2 charts"), "{warning}");
    assert!(
        warning.contains("identity states"),
        "the lost trust states are named, not only the names: {warning}"
    );
    assert!(warning.contains("connection reset"), "{warning}");
}

#[test]
fn a_successful_identity_read_carries_no_warning() {
    let pane = chart_pane(
        &cairn_medication_view::fixtures::sample_chart(),
        Uuid::from_u128(1),
        Ok(vec![]),
        Ok(RecordLinks::default()),
        DuplicateSection::checked_none_open(),
        None,
    );
    assert!(pane.members_error.is_none());
    assert_eq!(pane.list.charts.len(), 1);
}

/// Availability again: an unread link list is worded beside the list, never a failed open.
#[test]
fn a_failed_link_read_keeps_the_list_and_says_so() {
    let list = cairn_medication_view::fixtures::sample_chart();
    let pane = chart_pane(
        &list,
        Uuid::from_u128(1),
        Ok(vec![]),
        Err("x".into()),
        DuplicateSection::checked_none_open(),
        None,
    );
    assert_eq!(pane.list.rows.len(), list.rows.len());
    assert!(pane.links.is_empty());
    let warning = pane.links_error.expect("the failure is reported");
    assert!(warning.contains("could not be read"), "{warning}");
}

fn plan_row(cross_patient: bool, members: &[(u128, u128)]) -> MedicationRow {
    let mut row = cairn_medication_view::fixtures::sample_chart().rows[0].clone();
    row.cross_patient = cross_patient;
    row.members = members
        .iter()
        .map(|(thread, chart)| cairn_medication_view::MemberVouch {
            medication_id: Uuid::from_u128(*thread),
            vouch: cairn_medication_view::VouchState::Absent,
            patient_id: Uuid::from_u128(*chart),
        })
        .collect();
    row
}

/// #697 (b): in a doubted set a line recorded only on ANOTHER member is withheld; ceasing
/// it from this chart writes nothing and names every thread it held back.
#[test]
fn a_hazard_line_only_on_another_member_ceases_nothing_from_here() {
    let plan = cease_plan(&plan_row(true, &[(11, 2)]), Uuid::from_u128(1));
    assert!(plan.write.is_empty());
    assert_eq!(plan.held_back.len(), 1);
    assert!(plan.held_back[0].contains(&Uuid::from_u128(11).to_string()));
}

/// The reason alone withholds: `cross_patient` unset, `wrong_chart.doubted_link` set. Guards
/// `cease_plan` against reverting to reading `row.cross_patient` only.
#[test]
fn a_doubted_link_reason_alone_holds_back_another_members_thread() {
    let mut row = plan_row(false, &[(11, 2)]);
    row.wrong_chart.doubted_link = true;
    let plan = cease_plan(&row, Uuid::from_u128(1));
    assert!(plan.write.is_empty());
    assert_eq!(plan.held_back.len(), 1);
}

/// An ordinary line on a combined list: every thread stopped, each on its OWN chart.
#[test]
fn an_ordinary_line_ceases_every_thread_on_its_own_chart() {
    let plan = cease_plan(&plan_row(false, &[(10, 1), (11, 2)]), Uuid::from_u128(1));
    assert_eq!(
        plan.write,
        vec![
            (Uuid::from_u128(10), Uuid::from_u128(1)),
            (Uuid::from_u128(11), Uuid::from_u128(2))
        ]
    );
    assert!(plan.held_back.is_empty());
}

/// A hazard line: only the opened chart's thread is stopped; the other is held back and
/// named, never written onto a chart that may be someone else's.
#[test]
fn a_hazard_line_ceases_only_the_opened_charts_threads_and_names_the_rest() {
    let plan = cease_plan(&plan_row(true, &[(10, 1), (11, 2)]), Uuid::from_u128(1));
    assert_eq!(plan.write, vec![(Uuid::from_u128(10), Uuid::from_u128(1))]);
    assert_eq!(plan.held_back.len(), 1);
    assert!(
        plan.held_back[0].contains(&Uuid::from_u128(11).to_string()),
        "{}",
        plan.held_back[0]
    );
    assert!(
        plan.held_back[0].contains("NOT stopped"),
        "{}",
        plan.held_back[0]
    );
}

/// A single chart linked to nothing: a hazard line's members are all on the opened chart,
/// so nothing changes from the pre-ADR-0076 behaviour.
#[test]
fn a_never_linked_hazard_line_still_ceases_its_own_thread() {
    let plan = cease_plan(&plan_row(true, &[(10, 1)]), Uuid::from_u128(1));
    assert_eq!(plan.write, vec![(Uuid::from_u128(10), Uuid::from_u128(1))]);
    assert!(plan.held_back.is_empty());
}

/// A single chart has no member lines, database or not.
#[tokio::test]
async fn a_single_chart_has_no_member_lines() {
    let state = AppState::mock(Some(Uuid::from_u128(1)));
    assert_eq!(linked_members(&state, &set(&[1])).await.unwrap(), vec![]);
}

/// A LINKED set with nothing to read identities from must not come back as an empty list:
/// that would render a combined list as a single chart. It is an error, which `chart_pane`
/// turns into the member warning.
#[tokio::test]
async fn a_linked_set_without_a_database_is_an_error_not_an_empty_list() {
    let state = AppState::mock(Some(Uuid::from_u128(1)));
    assert!(linked_members(&state, &set(&[1, 2])).await.is_err());
}

/// #841: Withhold and Stop are drawn only where the command could write them. A wrong-chart
/// hazard line with no thread on the opened chart can only be refused (Withhold: a typed reason
/// and a press for nothing) or stop nothing (Stop, which holds every thread back — and asks for
/// the key first), steps paper never has. The offer asks each command's own rule
/// (`hold::plan::withhold_target`, `cease_plan`), so they cannot drift.
///
/// Hiding them must not hide the way forward: the line SAYS where to stop or withhold it, as the
/// refused press used to. And only a hazard line is narrowed — an ordinary line whose own thread
/// is missing from its members (a reconcile re-keying a group between the list's reads) keeps
/// both, so the press names that anomaly and its remedy rather than the buttons vanishing.
#[test]
fn withhold_and_stop_are_offered_only_where_they_can_write() {
    let mut list = cairn_medication_view::fixtures::sample_chart();
    list.rows = vec![
        plan_row(true, &[(11, 2)]), // hazard, nothing on chart 1: neither, and said
        plan_row(true, &[(10, 1), (11, 2)]), // hazard, a thread on chart 1: both
        plan_row(false, &[(0xABCD, 1)]), // ordinary, own thread missing: both (the press names it)
    ];
    assert!(crate::hold::plan::withhold_target(&list.rows[2], Uuid::from_u128(1)).is_err());
    let pane = chart_pane(
        &list,
        Uuid::from_u128(1),
        Ok(vec![]),
        Ok(RecordLinks::default()),
        DuplicateSection::checked_none_open(),
        None,
    );
    let offered: Vec<(bool, bool)> = pane
        .list
        .rows
        .iter()
        .map(|r| (r.can_withhold, r.can_cease))
        .collect();
    assert_eq!(offered, [(false, false), (true, true), (true, true)]);
    let said = |i: usize| {
        pane.list.rows[i]
            .flags
            .contains(&NOT_ON_OPENED_CHART.to_string())
    };
    assert!(said(0), "{:?}", pane.list.rows[0].flags);
    assert!(!said(1) && !said(2), "said only where something was hidden");
    // An ordinary line is untouched: the sample chart's current lines all offer it.
    let sample = cairn_medication_view::fixtures::sample_chart();
    let opened = sample.charts.members()[0];
    let pane = chart_pane(
        &sample,
        opened,
        Ok(vec![]),
        Ok(RecordLinks::default()),
        DuplicateSection::checked_none_open(),
        None,
    );
    for (row, built) in pane.list.rows.iter().zip(build_view(&sample).rows) {
        assert_eq!(row.can_withhold, built.can_withhold, "{}", row.primary);
        assert_eq!(row.can_cease, built.can_cease, "{}", row.primary);
        assert_eq!(row.flags, built.flags, "{}", row.primary);
    }
}

/// #844: Release is drawn only where `release_plan` can write — the #841 rule finished. A hazard
/// line whose holds all sit on another chart's threads offered a press that ended nothing (and,
/// as a report, wiped the typed reason). The window sends every hold the row drew, so the offer
/// asks `release_plan` with all of them. The line then says where to release it.
#[test]
fn release_is_offered_only_where_it_can_write() {
    let hold = |id: u128, thread: u128| cairn_medication_view::MedicationHold {
        withhold_event_id: Uuid::from_u128(id),
        medication_id: Uuid::from_u128(thread),
        reason: "r".into(),
        starts_at: "s".into(),
        until: "reviewed".into(),
        withholder: "k".into(),
        pending: false,
    };
    let with_holds = |mut row: cairn_medication_view::MedicationRow, holds: &[(u128, u128)]| {
        row.holds = holds.iter().map(|(id, t)| hold(*id, *t)).collect();
        row
    };
    let mut list = cairn_medication_view::fixtures::sample_chart();
    list.rows = vec![
        // hazard, its hold on chart 2's thread only: no Release, and said
        with_holds(plan_row(true, &[(10, 1), (11, 2)]), &[(90, 11)]),
        // hazard, a hold on chart 1's thread: Release
        with_holds(plan_row(true, &[(10, 1), (11, 2)]), &[(90, 11), (91, 10)]),
        // ordinary, hold on another chart's thread: Release (an ordinary line is never narrowed)
        with_holds(plan_row(false, &[(10, 1), (11, 2)]), &[(90, 11)]),
    ];
    let pane = chart_pane(
        &list,
        Uuid::from_u128(1),
        Ok(vec![]),
        Ok(RecordLinks::default()),
        DuplicateSection::checked_none_open(),
        None,
    );
    let rows = &pane.list.rows;
    assert_eq!(
        rows.iter().map(|r| r.can_release).collect::<Vec<_>>(),
        [false, true, true]
    );
    let said = |i: usize| {
        rows[i]
            .flags
            .contains(&HOLD_NOT_ON_OPENED_CHART.to_string())
    };
    assert!(said(0), "{:?}", rows[0].flags);
    assert!(!said(1) && !said(2), "said only where Release was hidden");
    // Withhold and Stop still stand on row 0 (thread 10 is on chart 1), so the #841 sentence —
    // "no record on the chart you opened" — would be false there and is not said.
    assert!(rows[0].can_withhold && rows[0].can_cease);
    assert!(!rows[0].flags.contains(&NOT_ON_OPENED_CHART.to_string()));
}
