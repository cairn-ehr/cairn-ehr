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
use cairn_medication_view::{ChartSet, PatientMedicationList};
use cairn_node::patient::person::ChartIdentity;
use serde::Serialize;
use uuid::Uuid;

/// Why a command was refused when the webview's list of displayed charts could not be read.
/// Not the same fault as a changed set: nothing about the record moved — the window itself sent
/// something it should not have — so the remedy is to reopen, not to reload.
const UNREADABLE: &str = "this window could not tell which charts are on screen — reopen the chart";

/// Why a command was refused when the set changed while its list was on screen.
const CHANGED: &str =
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

/// One member chart's line under the identity header of a combined record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MemberLine {
    /// The member chart's id — the same id a row's source label names.
    pub patient_id: String,
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
/// A chart this node does not HOLD (`ChartIdentity::held`, a link naming a chart that has not
/// arrived here) gets its own wording: its name and date are absent because nothing arrived,
/// not because nothing was recorded, and "(no name recorded)" would say the latter.
pub fn member_line(identity: &ChartIdentity) -> MemberLine {
    let facts = if identity.held {
        let name = identity.name.as_deref().unwrap_or("(no name recorded)");
        let born = identity
            .birth_date
            .as_deref()
            .map(|d| format!("born {d}"))
            .unwrap_or_else(|| "date of birth not recorded".to_string());
        format!("{name} · {born}")
    } else {
        "(chart not yet received on this node — name and date of birth unknown)".to_string()
    };
    MemberLine {
        patient_id: identity.patient_id.to_string(),
        text: format!(
            "{facts} · identity {} · chart {}",
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
    ChartPane {
        list: build_view(list),
        members,
        members_error,
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

    /// A linked chart that has not reached this node: nothing was RECORDED-as-absent, nothing
    /// ARRIVED. The line says so, and carries the `unknown` trust `trust_of` gives it rather
    /// than a borrowed "confirmed".
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
                "(chart not yet received on this node — name and date of birth unknown) \
                 · identity unknown · chart {id}"
            )
        );
        assert!(!line.text.contains("recorded"), "{}", line.text);
    }

    /// Availability over consistency: a clinician must always be able to READ. If the member
    /// identities cannot be read, the medication list is still shown — with a warning that says
    /// the list is combined, over how many charts, that no member's identity state is known, and
    /// why — rather than the whole chart failing to open.
    #[test]
    fn a_failed_identity_read_keeps_the_list_and_says_so() {
        let mut list = cairn_medication_view::fixtures::sample_chart();
        list.charts = set(&[1, 0xB]);
        let pane = chart_pane(&list, Err("connection reset".into()));
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
        let pane = chart_pane(&cairn_medication_view::fixtures::sample_chart(), Ok(vec![]));
        assert!(pane.members_error.is_none());
        assert_eq!(pane.list.charts.len(), 1);
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
