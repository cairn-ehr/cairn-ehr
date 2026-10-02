//! The chart SET on screen: the window's half of ADR-0076 decisions 1 and 3.
//!
//! A chart linked to other charts of the same person opens as ONE combined record (decision 1):
//! the medication list is read over every chart in the link component, and the identity header
//! lists each member's OWN line — no winner chosen, because the disagreement between two
//! duplicates is often the very typo that made them. This module turns those member identities
//! into the lines the header shows ([`member_line`]).
//!
//! And because the set can change under a clinician's eyes — a link or unlink landing while the
//! list is being reviewed — every chart command names the set that was DISPLAYED, and refuses
//! when it no longer matches (decision 3). That is PR #674's rule ("a chart command acts on the
//! chart on screen", `AppState::displayed_patient`) widened from one chart to a set, and
//! [`check_displayed_set`] is where it lives, once, as a pure function.
//!
//! # Two checks, both wanted
//!
//! The window checks the displayed set against its OWN fresh read (here), and the node's
//! sign-off orchestrator checks it again against ITS first read. They are two snapshots taken at
//! two moments: the window's is the early one, testable against `AppState::mock` with no
//! database; the orchestrator's is the authoritative one, taken inside the act itself. Neither
//! replaces the other. A CEASE gets only the first: `cease_medication` writes one thread and
//! has no notion of a chart set, so the window's check is the whole of decision 3 for it (see
//! `commands::cease_impl`).
use crate::state::AppState;
use cairn_gui_tab_medications::view::{build_view, MedListView};
use cairn_medication_view::{ChartSet, MedicationRow, PatientMedicationList};
use cairn_node::patient::person::ChartIdentity;
use serde::Serialize;
use uuid::Uuid;

/// Why a command was refused when the webview's list of displayed charts could not be read.
/// Not the same fault as a changed set: nothing about the record moved — the window itself sent
/// something it should not have — so the remedy is to reopen, not to reload.
const UNREADABLE: &str = "this window could not tell which charts are on screen — reopen the chart";

/// Why a command was refused when the set changed while its list was on screen. Crate-visible
/// so the link commands (`link::resolve_pair` for this record, `link::link_impl` for the other
/// one) can tell a CHANGED set from an unreadable one and reword only the former.
pub(crate) const CHANGED: &str =
    "the linked charts changed while this list was on screen — nothing was done; reload the chart";

/// Refuse unless the charts the webview DISPLAYED are exactly the set just read; on a match,
/// return the displayed set.
///
/// `read` is the set a fresh read of the open chart resolved; `displayed` is what the webview
/// sent back — the `charts` field of the `MedListView` it rendered. Order does not matter (both
/// sides go through `ChartSet`, which sorts and de-duplicates); membership does. An empty or
/// unparseable list is refused as "could not tell", never treated as a match: a command that
/// cannot say what it saw must not act.
///
/// Returns the DISPLAYED set rather than `()` so a caller hands the node's orchestrator what
/// the clinician actually saw, visibly at the call site — equal to `read` here, but the
/// orchestrator's own compare is then literally against the screen, not against a set a
/// reviewer has to prove equal to it.
pub fn check_displayed_set(read: &ChartSet, displayed: &[String]) -> Result<ChartSet, String> {
    let parsed: Vec<Uuid> = displayed
        .iter()
        .map(|id| id.parse::<Uuid>())
        .collect::<Result<_, _>>()
        .map_err(|_| UNREADABLE.to_string())?;
    let shown = ChartSet::new(parsed).ok_or_else(|| UNREADABLE.to_string())?;
    if &shown == read {
        Ok(shown)
    } else {
        Err(CHANGED.to_string())
    }
}

/// The sign-off report's sentence naming the charts a combined gesture read across, or `None`
/// for a single chart.
///
/// The CLI prints the same fact after a combined sign-off (`main.rs`, `medication-sign-off`),
/// and for the same reason: "Signed 3 medication thread(s)" on a combined list could be read
/// as "recorded on the chart I opened". Each line was signed on the chart it was recorded on
/// (ADR-0076 decision 2), and the report says so. `None` for a never-linked chart, whose report
/// is unchanged.
pub fn signed_across_message(charts: &ChartSet) -> Option<String> {
    charts.is_linked().then(|| {
        format!(
            "This gesture read across {} linked charts; each line was signed on the chart it \
             was recorded on.",
            charts.members().len()
        )
    })
}

/// Which member threads a cease gesture on one displayed line writes, and which it holds back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CeasePlan {
    /// `(thread, the chart its own statement is on)` — each cessation is written to its
    /// thread's own chart (ADR-0076 decision 2).
    pub write: Vec<(Uuid, Uuid)>,
    /// Threads deliberately NOT stopped, each with the sentence the report shows for it.
    pub held_back: Vec<String>,
}

/// Plan a cease of `row` from the chart the clinician opened.
///
/// An ordinary line stops every member thread, each on its own chart. A line withheld as a
/// WRONG-CHART HAZARD (`MedicationRow::is_wrong_chart_hazard`) stops only the threads on the opened
/// chart: the node itself says this line may carry another person's drug — the group reaches
/// a chart outside the set, or the set holds a link the node doubts and the line is not
/// recorded only on the opened chart — so "stop this drug" can only
/// be trusted to mean it for the patient in front of the clinician. The other threads are
/// held back and NAMED, never skipped in silence (ADR-0060 decision 2); they can be stopped
/// from their own chart. Without this, Stop on a line spanning a doubted link would write a
/// cessation onto a chart the node believes may be someone else's.
///
/// This holds back more than strictly needed when the other thread is on a linked chart of
/// the same person (a hazard line reaching OUTSIDE the set also spans the set): stopping less
/// and saying so is the safe error. A never-linked chart is unaffected — all its members are
/// on the opened chart. A doubted-set line on another member only (#697 (b)) is held back
/// entirely. Pure, so the rule is tested without a database.
pub fn cease_plan(row: &MedicationRow, opened: Uuid) -> CeasePlan {
    let mut plan = CeasePlan {
        write: vec![],
        held_back: vec![],
    };
    for m in &row.members {
        if !row.is_wrong_chart_hazard() || m.patient_id == opened {
            plan.write.push((m.medication_id, m.patient_id));
        } else {
            plan.held_back.push(format!(
                "thread {} on chart {}: NOT stopped from here — this line may carry another \
                 person's drug, so only this chart's own threads are stopped; stop it from its \
                 own chart",
                m.medication_id, m.patient_id
            ));
        }
    }
    plan
}

/// One member chart's line under the identity header of a combined record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MemberLine {
    /// The member chart's id — the same id a row's source label names.
    pub patient_id: String,
    /// The display name as the line shows it — the name, or its worded absence — so a link line
    /// can name both charts without a second read.
    pub name: String,
    /// The whole line as the clinician reads it; see [`member_line`].
    pub text: String,
}

/// Render one member's own identity line.
///
/// Every absence is NAMED rather than left blank (principle 4): a header reading "born " with
/// nothing after it is indistinguishable from a rendering fault, and "(no name recorded)" is a
/// recordable state the clinician needs to see — the wording matches the single-chart header's
/// (`funnel::view`). The chart id is always last and always whole: it is what ties this line to
/// the source label on each medication row.
///
/// A chart whose registration this node does not HOLD (`ChartIdentity::held` — a link naming
/// a chart that has not fully arrived here) is worded differently: an absent name or date is
/// then UNKNOWN, not "not recorded" — nothing says it was never recorded, only that it has not
/// arrived — and the line says the registration is missing. A name or date that did arrive
/// is still shown.
pub fn member_line(identity: &ChartIdentity) -> MemberLine {
    let (no_name, no_dob) = if identity.held {
        ("(no name recorded)", "date of birth not recorded")
    } else {
        ("(name unknown)", "date of birth unknown")
    };
    let name = identity.name.as_deref().unwrap_or(no_name);
    let born = identity
        .birth_date
        .as_deref()
        .map(|d| format!("born {d}"))
        .unwrap_or_else(|| no_dob.to_string());
    let not_held = if identity.held {
        ""
    } else {
        " · registration not yet received on this node"
    };
    MemberLine {
        patient_id: identity.patient_id.to_string(),
        name: name.to_string(),
        text: format!(
            "{name} · {born} · identity {}{not_held} · chart {}",
            identity.trust, identity.patient_id
        ),
    }
}

/// What `med_list` hands the webview: the list, and — when the chart is linked to others — one
/// identity line per member chart, shown under the identity header (ADR-0076 decision 1).
#[derive(Debug, Serialize)]
pub struct ChartPane {
    pub list: MedListView,
    /// Empty unless the list is a combined read over linked charts whose names were read.
    pub members: Vec<MemberLine>,
    /// Set when the chart is linked but its member identities could NOT be read. The list is
    /// still shown — a clinician must always be able to read (availability over consistency) —
    /// and this says, where the member lines would have been, that the list is combined and
    /// what is missing: the names, dates of birth AND identity states, all three (they are read
    /// together). The last matters most — a member `under-review` is exactly what should stop a
    /// clinician trusting the combination, and its line is gone. Without this warning a combined
    /// list would read as a single chart.
    pub members_error: Option<String>,
    /// The standing links joining the record's charts, one line each (R2b-2) — empty for a
    /// chart linked to nothing.
    pub links: Vec<crate::link::record_links::RecordLinkView>,
    /// Set when those links could not be read; the list and the member lines still show.
    pub links_error: Option<String>,
}

/// Assemble the pane from one chart read and the outcome of reading its member names.
///
/// Pure, so the availability rule is tested with no database: whatever happened to the member
/// read, the list itself always goes to the screen. A failure becomes `members_error`, worded to
/// say the list is combined, over how many charts, and that each member's identity state is
/// unknown — the facts the missing header lines would have conveyed.
pub fn chart_pane(
    list: &PatientMedicationList,
    members: Result<Vec<MemberLine>, String>,
    edges: Result<Vec<cairn_node::patient::edges::RecordEdge>, String>,
) -> ChartPane {
    let (members, members_error) = match members {
        Ok(members) => (members, None),
        Err(e) => (
            vec![],
            Some(format!(
                "The linked charts' names, dates of birth and identity states could not be read \
                 — this list covers {} charts, and whether any of them is under review is \
                 unknown: {e}",
                list.charts.members().len()
            )),
        ),
    };
    let (links, links_error) =
        crate::link::record_links::links_section(edges, &members, list.charts.is_linked());
    ChartPane {
        list: build_view(list),
        members,
        members_error,
        links,
        links_error,
    }
}

/// The member lines for a chart set, or none when it is not linked.
///
/// None for a single chart: its identity is already the header, and a one-line "linked charts"
/// list would claim a link that does not exist. A LINKED set with no database to read from
/// (fixture mode — unreachable today, since `AppState::mock` serves single-chart fixtures only)
/// is an error, not an empty list, for the reason that follows. A failed read is
/// an ERROR, not an empty list: a combined record whose header silently lost its member lines
/// would read as a single chart while its rows came from several. The caller
/// ([`chart_pane`]) turns that error into a warning beside the list, never a failed open.
pub async fn linked_members(
    state: &AppState,
    charts: &ChartSet,
) -> Result<Vec<MemberLine>, String> {
    if !charts.is_linked() {
        return Ok(vec![]);
    }
    let Some(db) = state.db.as_ref() else {
        return Err("there is no database to read the linked charts' identities from".into());
    };
    let db = db.lock().await;
    let identities = cairn_node::patient::person::chart_identities(&*db, charts)
        .await
        .map_err(|e| format!("{e:#}"))?;
    Ok(identities.iter().map(member_line).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    fn a_different_set_is_refused_with_a_reload_sentence() {
        let err = check_displayed_set(&set(&[1, 2]), &ids(&[1])).unwrap_err();
        assert!(err.contains("linked charts changed"), "{err}");
        assert!(err.contains("reload"), "{err}");
    }

    #[test]
    fn an_unparseable_id_is_refused() {
        let err = check_displayed_set(&set(&[1]), &["not-a-uuid".to_string()]).unwrap_err();
        assert!(err.contains("could not tell which charts"), "{err}");
    }

    /// A command that names no chart at all cannot say what it saw, so it must not act.
    #[test]
    fn an_empty_displayed_list_is_refused() {
        let err = check_displayed_set(&set(&[1]), &[]).unwrap_err();
        assert!(err.contains("could not tell which charts"), "{err}");
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
        let pane = chart_pane(&list, Err("connection reset".into()), Ok(vec![]));
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
            Ok(vec![]),
            Ok(vec![]),
        );
        assert!(pane.members_error.is_none());
        assert_eq!(pane.list.charts.len(), 1);
    }

    /// Availability again: an unread link list is worded beside the list, never a failed open.
    #[test]
    fn a_failed_link_read_keeps_the_list_and_says_so() {
        let list = cairn_medication_view::fixtures::sample_chart();
        let pane = chart_pane(&list, Ok(vec![]), Err("x".into()));
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
}

/// Every chart command actually CALLS the set check (ADR-0076 decision 3).
///
/// Kept beside the check rather than in `commands.rs`, which is long already: these pin that the
/// two writing commands refuse a changed set and that the read hands the webview the set to send
/// back. A refactor that dropped the check from one command would otherwise stay green while
/// signing a list nobody reviewed.
#[cfg(test)]
mod command_tests {
    use crate::commands::{cease_impl, med_list_impl, sign_off_impl};
    use crate::state::AppState;
    use uuid::Uuid;

    fn open_on_the_fixture_chart() -> (AppState, String) {
        let fixture: Uuid = cairn_gui_data::mock::fixtures::FIXTURE_UUID
            .parse()
            .unwrap();
        (AppState::mock(Some(fixture)), fixture.to_string())
    }

    /// The set check runs BEFORE fixture mode's own refusal, so a set that changed is reported
    /// as exactly that even here — which is also what proves the order.
    #[tokio::test]
    async fn sign_off_refuses_a_changed_set() {
        let (state, on_screen) = open_on_the_fixture_chart();
        let err = sign_off_impl(
            &state,
            &on_screen,
            vec![on_screen.clone(), Uuid::from_u128(99).to_string()],
        )
        .await
        .unwrap_err();
        assert!(err.contains("linked charts changed"), "{err}");
    }

    /// Cease is a write on the same list, so it carries the same rule — ahead of the fixture
    /// refusal and ahead of the reason check.
    #[tokio::test]
    async fn cease_refuses_a_changed_set() {
        let (state, on_screen) = open_on_the_fixture_chart();
        let err = cease_impl(
            &state,
            &on_screen,
            vec![on_screen.clone(), Uuid::from_u128(99).to_string()],
            &Uuid::from_u128(10).to_string(),
            "",
        )
        .await
        .unwrap_err();
        assert!(err.contains("linked charts changed"), "{err}");
    }

    /// The read hands the webview the set it must send back, and — the fixture chart being
    /// linked to nothing (fixture charts never are) — no member lines.
    #[tokio::test]
    async fn the_medication_list_names_its_chart_set() {
        let (state, on_screen) = open_on_the_fixture_chart();
        let pane = med_list_impl(&state, &on_screen).await.unwrap();
        assert_eq!(pane.list.charts, vec![on_screen]);
        assert!(pane.members.is_empty());
    }
}
