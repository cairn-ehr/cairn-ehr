//! The stat orders under the charted lines (charting slice 2, design §3.6): each stat order with
//! its gives and their strikes, read in the SAME snapshot as the chart
//! (`chart_versions::read_chart_and_versions`), so a line is never shown "not yet given" beside a
//! list from a moment after it was given.
//!
//! [`read_stat`] is the read. Putting each order's words under its line (`attach_stat`, the
//! drawing half) belongs here too, beside it; until then the pane carries only whether the read
//! failed ([`STAT_NOT_READ`], `ChartPane::stat_error`).
use cairn_node::medication::stat_read::{read_stat_orders, StatOrder};
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
