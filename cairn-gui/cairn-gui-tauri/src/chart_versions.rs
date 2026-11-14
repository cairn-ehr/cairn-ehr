//! The set-aside versions under the charted lines (batch charting slice 2a, design §3.2 / §3.3
//! display): a version a source reported and the clinician set aside is never a current line,
//! but it stays readable, collapsed under the line it lost to — that line's set-aside versions,
//! stopped line included.
//!
//! Read in the SAME snapshot as the chart (`read_chart_and_versions`), so a version is never
//! shown beside a list from another moment; joined to its line in Rust (the tab crate's
//! `versions_under`, on the line's member threads) and worded in Rust (`version_line`). The
//! webview only draws the words.
//!
//! Slice 2b adds how each version was settled (`read_settlements`, in the same snapshot and
//! savepoint): "checked by …" / "restored by …" under the version, "replaced: …" under the line a
//! restore charted, whether the version is still to check, and whether the opened chart may
//! settle it (its own chart is the opened one; never a linked chart's, spec §2).
use crate::commands::read_chart_of;
use crate::state::AppState;
use cairn_gui_tab_medications::{
    settle_words, under_line, version_line, versions_under, MedListRowView, SettleLine, VersionView,
};
use cairn_medication_view::{MedicationRow, PatientMedicationList};
use cairn_node::medication::set_aside_read::{read_set_asides, SetAsideRow};
use cairn_node::medication::version_settle_read::{read_settlements, Settlement, SettlementRow};
use chrono::{DateTime, Local};
use uuid::Uuid;

/// The set-aside versions over a chart set and how each was settled, read together.
pub(crate) type VersionsRead = (Vec<SetAsideRow>, Vec<SettlementRow>);

/// Above the medication list when the set-aside versions could not be read: the current lines
/// still show, and "no version under a line" must never read as "none was set aside".
pub const VERSIONS_NOT_READ: &str =
    "Set-aside versions could not be read: none is shown under the lines";

/// The chart (as [`read_chart_of`]) and the set-aside versions over its chart set with their
/// settlements, from ONE snapshot. Fixture mode has no versions. A failed versions read is
/// `Err(`[`VERSIONS_NOT_READ`]`)` beside a list that still opens (availability over
/// consistency, as every other secondary read of the pane): it runs under a savepoint, so its
/// failure aborts only itself, never the snapshot the list was read in. The settlements share
/// it: a version must never show as unsettled because only its settlements failed to read.
pub(crate) async fn read_chart_and_versions(
    state: &AppState,
    patient: Uuid,
) -> anyhow::Result<(PatientMedicationList, Result<VersionsRead, String>)> {
    let Some(db) = state.db.as_ref() else {
        return Ok((read_chart_of(state, patient).await?, Ok((vec![], vec![]))));
    };
    let mut db = db.lock().await?;
    cairn_node::medication::snapshot::in_snapshot(&mut db, async |tx| {
        let list = cairn_node::medication::read::list_patient_medications(tx, patient).await?;
        tx.batch_execute("SAVEPOINT set_aside_read").await?;
        let charts = list.charts.members();
        let read = async {
            anyhow::Ok((
                read_set_asides(tx, charts).await?,
                read_settlements(tx, charts).await?,
            ))
        };
        let versions = match read.await {
            Ok(v) => Ok(v),
            Err(e) => {
                eprintln!("the set-aside versions could not be read: {e:#}");
                tx.batch_execute("ROLLBACK TO SAVEPOINT set_aside_read")
                    .await?;
                Err(VERSIONS_NOT_READ.to_string())
            }
        };
        Ok((list, versions))
    })
    .await
    .map_err(|e| db.explain(e))
}

/// Put each version under its line, with its settlements, and each restore's "replaced:" under
/// the line it charted: `shown` is `build_view`'s rows, in `rows`' order (as
/// `chart_set::chart_pane` zips them). `opened` is the chart the window opened: only its own
/// versions may be settled from here. Pure.
pub(crate) fn attach_versions(
    shown: &mut [MedListRowView],
    rows: &[MedicationRow],
    (versions, settlements): &VersionsRead,
    opened: Uuid,
    now: &DateTime<Local>,
) {
    let views: Vec<(Uuid, VersionView)> = versions
        .iter()
        .map(|v| (v.alternative_to, version_view(v, settlements, opened, now)))
        .collect();
    // A restore's "replaced:" goes under the line it charted (the restore's own thread).
    let replaced: Vec<(Uuid, String)> = settlements
        .iter()
        .filter_map(|s| match &s.settlement {
            Settlement::Restored {
                line,
                replaced_line,
            } => {
                let words = SettleLine::Replaced {
                    line: replaced_line,
                };
                Some((*line, settle_words(words, s.who.as_deref(), &s.at, now)))
            }
            Settlement::Checked { .. } => None,
        })
        .collect();
    for (shown, row) in shown.iter_mut().zip(rows) {
        shown.versions = versions_under(row, &views);
        shown.replaced = under_line(row, &replaced);
    }
}

/// One version as the line shows it: slice 2a's line, then each settlement of THIS version on
/// ITS chart (`version_id` is not unique across charts), oldest first as read. `open` is left to
/// `versions_under`, which knows whether its line is current. Pure.
fn version_view(
    v: &SetAsideRow,
    settlements: &[SettlementRow],
    opened: Uuid,
    now: &DateTime<Local>,
) -> VersionView {
    let own = settlements
        .iter()
        .filter(|s| s.version_id == v.version_id && s.patient_id == v.patient_id);
    let mut restored = false;
    let mut settled = vec![];
    for s in own {
        let words = match &s.settlement {
            Settlement::Checked { source, why } => SettleLine::Checked {
                source,
                why: why.as_deref(),
            },
            Settlement::Restored { .. } => {
                restored = true;
                SettleLine::Restored
            }
        };
        settled.push(settle_words(words, s.who.as_deref(), &s.at, now));
    }
    VersionView {
        line: version_line(&v.source, &v.drug_line, v.why.as_deref()),
        version_id: v.version_id.to_string(),
        settled,
        open: false,
        can_settle: v.patient_id == opened && !restored,
        restored,
    }
}
