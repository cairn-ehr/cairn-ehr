//! The stat orders under the charted lines (charting slice 2, design §3.6): each stat order with
//! its gives and their strikes, read in the SAME snapshot as the chart
//! (`chart_versions::read_chart_and_versions`), so a line is never shown "not yet given" beside a
//! list from a moment after it was given.
//!
//! [`read_stat`] is the read; [`attach_stat`] puts each order's words under its line, worded in
//! the tab crate (`stat_words`). A failed read is only said above the list ([`STAT_NOT_READ`],
//! `ChartPane::stat_error`): no line then claims to know whether it was given.
use cairn_gui_tab_medications::{
    stat_words, struck_words, under_line, GiveIn, GiveView, MedListRowView, StatView, StatWordsIn,
};
use cairn_medication_view::{MedicationRow, MedicationStatus};
use cairn_node::medication::stat_read::{read_stat_orders, StatOrder};
use chrono::{DateTime, Local};
use tokio_postgres::Transaction;
use uuid::Uuid;

/// Above the medication list when the stat orders could not be read: the list still opens, and
/// a given line still reads stopped, but no line can say whether (or when) it was given.
pub const STAT_NOT_READ: &str = "Stat doses could not be read: no line shows whether it was given";

/// Every stat order over `charts`, read inside the chart's snapshot `tx` under its OWN savepoint:
/// a failure rolls back only this read, never the snapshot the list was read in, and answers
/// `Err(`[`STAT_NOT_READ`]`)` beside a list that still opens (availability over consistency, as
/// the set-aside versions read).
pub(crate) async fn read_stat(
    tx: &Transaction<'_>,
    charts: &[Uuid],
) -> Result<Vec<StatOrder>, String> {
    let read = async {
        tx.batch_execute("SAVEPOINT stat_read").await?;
        match read_stat_orders(tx, charts).await {
            Ok(orders) => Ok(orders),
            Err(e) => {
                tx.batch_execute("ROLLBACK TO SAVEPOINT stat_read").await?;
                Err(e)
            }
        }
    };
    read.await.map_err(|e: anyhow::Error| {
        eprintln!("the stat orders could not be read: {e:#}");
        STAT_NOT_READ.to_string()
    })
}

/// Put each stat order's line under the drug line that holds its thread (`under_line`, on the
/// row's member threads), and word a stopped line with a standing give "given", not "ceased": on
/// paper the stat box is initialled with the time, and that is what ended the order. `shown` is `build_view`'s rows in `rows`' order (as
/// `attach_versions`). `opened` is the chart the window opened: only its own orders and gives
/// are offered a button (never a linked chart's, spec §2); Rust re-derives every one of them on
/// a fresh read before the key anyway. Pure.
pub(crate) fn attach_stat(
    shown: &mut [MedListRowView],
    rows: &[MedicationRow],
    orders: &[StatOrder],
    opened: Uuid,
    now: &DateTime<Local>,
) {
    let views: Vec<(Uuid, StatView)> = orders
        .iter()
        .map(|o| (o.medication_id, stat_view(o, opened, now)))
        .collect();
    for (shown, row) in shown.iter_mut().zip(rows) {
        // shortcut: a row holding several stat orders (reconciled duplicates, ADR-0047: the same
        // order charted at two terminals) shows ONE — the one with a standing give if any, else
        // the first read — and offers Given only when NO order under it has a standing give:
        // Rust re-derives "waiting" per thread, so a Given on the waiting twin would give the dose
        // twice. Follow-up issue: "a row holding several stat orders shows one".
        let under = under_line(row, &views);
        let given = under.iter().any(|s| !s.gives.is_empty());
        let Some(mut stat) = under
            .iter()
            .find(|s| !s.gives.is_empty())
            .or(under.first())
            .cloned()
        else {
            continue;
        };
        let current = row.status == MedicationStatus::Active;
        // A stopped line cannot be given (Rust refuses it: `STAT_NOT_OPEN`), so no button.
        stat.can_give &= current && !given;
        if !current && !stat.gives.is_empty() {
            shown.status_label = "given".into();
        }
        shown.stat = Some(stat);
    }
}

/// One stat order as its line shows it; `can_give` is narrowed by its row in [`attach_stat`].
fn stat_view(o: &StatOrder, opened: Uuid, now: &DateTime<Local>) -> StatView {
    let own = o.patient_id == opened;
    let words = StatWordsIn {
        ordered_at: o.ordered_at,
        orderer: o.orderer.clone(),
        gives: o
            .gives
            .iter()
            .map(|g| GiveIn {
                given_at: g.given_at,
                giver: g.giver.clone(),
                struck: g
                    .struck
                    .as_ref()
                    .map(|s| (s.who.clone(), s.at, s.why.clone())),
            })
            .collect(),
    };
    StatView {
        medication_id: o.medication_id.to_string(),
        line: stat_words(&words, now),
        struck: struck_words(&words, now),
        can_give: own && o.waiting(),
        // The read gives them oldest first; only a standing give can be struck.
        gives: o
            .gives
            .iter()
            .filter(|g| g.struck.is_none())
            .map(|g| GiveView {
                event_id: g.event_id.to_string(),
                can_strike: own,
            })
            .collect(),
    }
}

#[cfg(test)]
#[path = "chart_stat_tests.rs"]
mod tests;
