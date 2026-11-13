//! The set-aside versions under the charted lines (batch charting slice 2a, design §3.2 / §3.3
//! display): a version a source reported and the clinician set aside is never a current line,
//! but it stays readable, collapsed under the line it lost to — that line's set-aside versions,
//! stopped line included.
//!
//! Read in the SAME snapshot as the chart (`read_chart_and_versions`), so a version is never
//! shown beside a list from another moment; joined to its line in Rust (the tab crate's
//! `versions_under`, on the line's member threads) and worded in Rust (`version_line`). The
//! webview only draws the words.
use crate::commands::read_chart_of;
use crate::state::AppState;
use cairn_gui_tab_medications::{version_line, versions_under, MedListRowView};
use cairn_medication_view::{MedicationRow, PatientMedicationList};
use cairn_node::medication::set_aside_read::{read_set_asides, SetAsideRow};
use uuid::Uuid;

/// The chart (as [`read_chart_of`]) and the set-aside versions over its chart set, from ONE
/// snapshot. Fixture mode has no versions.
pub(crate) async fn read_chart_and_versions(
    state: &AppState,
    patient: Uuid,
) -> anyhow::Result<(PatientMedicationList, Vec<SetAsideRow>)> {
    let Some(db) = state.db.as_ref() else {
        return Ok((read_chart_of(state, patient).await?, vec![]));
    };
    let mut db = db.lock().await?;
    cairn_node::medication::snapshot::in_snapshot(&mut db, async |tx| {
        let list = cairn_node::medication::read::list_patient_medications(tx, patient).await?;
        let versions = read_set_asides(tx, list.charts.members()).await?;
        Ok((list, versions))
    })
    .await
    .map_err(|e| db.explain(e))
}

/// Put each version under its line: `shown` is `build_view`'s rows, in `rows`' order (as
/// `chart_set::chart_pane` zips them). Pure.
pub(crate) fn attach_versions(
    shown: &mut [MedListRowView],
    rows: &[MedicationRow],
    versions: &[SetAsideRow],
) {
    let lines: Vec<(Uuid, String)> = versions
        .iter()
        .map(|v| {
            let line = version_line(&v.source, &v.drug_line, v.why.as_deref());
            (v.alternative_to, line)
        })
        .collect();
    for (shown, row) in shown.iter_mut().zip(rows) {
        shown.versions = versions_under(row, &lines);
    }
}
