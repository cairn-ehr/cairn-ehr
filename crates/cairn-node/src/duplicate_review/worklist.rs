//! The possible-duplicate worklist's node reads (repair path R5b, #680; design page "R5b — the
//! worklist, designed 2026-10-08"): the front door's tray, "Possible duplicates (N)".
//!
//! Every row comes from db/057's `match_proposal_open` — the ONE "still needs a human" predicate
//! the banner shares — and an entry is a PAIR OF RECORDS, not a proposal row: two charts of one
//! person proposed against one other person are one entry, counted once. A record's key is
//! `COALESCE(person_member.person_id, chart)` (a never-linked chart is a record of one), the same
//! `person_member` notion db/057 uses for "same record", so the count, the list and the view can
//! never disagree about who is one person.
//!
//! Two reads, split by cost: [`worklist_count`] is ONE statement (the front door pays for it on
//! every show); [`worklist`] reads the rows, groups them here ([`group_by_record_pair`], pure),
//! and reads the two records of only the entries it will show.
use super::{is_another_record, DISPUTED_SQL};
use crate::patient::person::person_charts;
use anyhow::Context;
use cairn_medication_view::ChartSet;
use std::cmp::Reverse;
use std::collections::BTreeMap;
use tokio_postgres::GenericClient;
use uuid::Uuid;

/// One open proposal, as the list query reads it: both charts, the record each belongs to, and
/// the flags the tray shows. Canonical order: `low < high`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposalRow {
    pub low: Uuid,
    pub high: Uuid,
    /// `COALESCE(person_member.person_id, low)` — the record `low` belongs to.
    pub low_record: Uuid,
    pub high_record: Uuid,
    pub band: String,
    pub status: String,
    /// The db/016 veto floor finds a disagreement between the two charts NOW.
    pub vetoed: bool,
    /// An un-attested unlink stands for this pair (`DISPUTED_SQL`, ADR-0078).
    pub disputed: bool,
    /// When the proposal was written, epoch milliseconds — ordering only.
    pub created_ms: i64,
}

/// One tray entry: every open proposal between ONE pair of records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorklistEntry {
    /// The record holding the most recently registered chart; Review opens it.
    pub newer_record: Uuid,
    pub older_record: Uuid,
    /// The newer record's side of the entry's newest proposal — the chart Review opens.
    pub open_chart: Uuid,
    /// The other side of that same proposal — what Review compares against.
    pub older_chart: Uuid,
    /// Every open pair this entry stands for, canonical `(low, high)`, sorted, no duplicates.
    pub pairs: Vec<(Uuid, Uuid)>,
    /// The strongest band over the pairs (`auto_candidate` over `review`).
    pub band: String,
    pub vetoed: bool,
    pub disputed: bool,
    pub accepted: bool,
    /// The newest of the pairs' `created_ms` — orders the tray, newest first.
    pub newest_ms: i64,
}

/// An entry with both records read in full (`person_charts`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorklistItem {
    pub entry: WorklistEntry,
    pub newer: ChartSet,
    pub older: ChartSet,
}

/// The tray: the newest entries read in full, and the count of ALL entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worklist {
    pub items: Vec<WorklistItem>,
    pub total: usize,
}

/// The millisecond a UUIDv7 chart id was minted — its registration time, as the registering
/// node's clock claimed it. Used only to decide which record Review opens; never shown.
pub fn registered_ms(id: Uuid) -> u64 {
    (id.as_u128() >> 80) as u64
}

/// The band's strength, for "the strongest of the pairs".
fn band_rank(band: &str) -> u8 {
    match band {
        "auto_candidate" => 2,
        "review" => 1,
        _ => 0,
    }
}

/// Group open proposals into entries, one per pair of RECORDS; newest proposal first. **Pure.**
///
/// - A row whose two record keys are equal is no entry (they read as one record — db/057 already
///   drops it; kept here so a direct caller cannot be wrong).
/// - **Newer record**: the record holding the chart with the latest UUIDv7 time among the charts
///   the entry's pairs name; a tie goes to the SMALLER chart id.
/// - **`open_chart`**: the newer record's side of the entry's NEWEST proposal (what Review opens);
///   `older_chart` its other side. A `created_ms` tie picks the smaller `(low, high)`.
/// - `band` the strongest; `vetoed` / `disputed` / `accepted` if ANY pair is.
/// - Ordering: `newest_ms` descending, then `(newer_record, older_record)` ascending — so the same
///   rows in any order give the same entries.
pub fn group_by_record_pair(rows: &[ProposalRow]) -> Vec<WorklistEntry> {
    let mut groups: BTreeMap<(Uuid, Uuid), Vec<&ProposalRow>> = BTreeMap::new();
    for r in rows.iter().filter(|r| r.low_record != r.high_record) {
        let key = (
            r.low_record.min(r.high_record),
            r.low_record.max(r.high_record),
        );
        groups.entry(key).or_default().push(r);
    }
    let mut entries: Vec<WorklistEntry> = groups.into_values().map(entry_of).collect();
    entries.sort_by(|a, b| {
        b.newest_ms
            .cmp(&a.newest_ms)
            .then_with(|| (a.newer_record, a.older_record).cmp(&(b.newer_record, b.older_record)))
    });
    entries
}

/// One entry from its (non-empty) group of rows. **Pure.**
fn entry_of(group: Vec<&ProposalRow>) -> WorklistEntry {
    // Every chart the pairs name, with its record.
    let charts = group
        .iter()
        .flat_map(|r| [(r.low, r.low_record), (r.high, r.high_record)]);
    let (_, newer_record) = charts
        .max_by_key(|(chart, _)| (registered_ms(*chart), Reverse(*chart)))
        .expect("a group has at least one row");
    let newest = group
        .iter()
        .max_by_key(|r| (r.created_ms, Reverse((r.low, r.high))))
        .expect("a group has at least one row");
    let (open_chart, older_chart, older_record) = if newest.low_record == newer_record {
        (newest.low, newest.high, newest.high_record)
    } else {
        (newest.high, newest.low, newest.low_record)
    };
    let mut pairs: Vec<(Uuid, Uuid)> = group.iter().map(|r| (r.low, r.high)).collect();
    pairs.sort();
    pairs.dedup();
    WorklistEntry {
        newer_record,
        older_record,
        open_chart,
        older_chart,
        pairs,
        band: group
            .iter()
            .max_by_key(|r| band_rank(&r.band))
            .map(|r| r.band.clone())
            .unwrap_or_default(),
        vetoed: group.iter().any(|r| r.vetoed),
        disputed: group.iter().any(|r| r.disputed),
        accepted: group.iter().any(|r| r.status == "accepted"),
        newest_ms: newest.created_ms,
    }
}

/// The open proposals with both sides' record keys — the ONE spelling of a record key, shared
/// by the count and the list. A record key is `COALESCE(person_member.person_id, chart)`.
const RECORDS_FROM: &str = "\
      FROM match_proposal_open \
      LEFT JOIN person_member a ON a.patient_id = patient_low \
      LEFT JOIN person_member b ON b.patient_id = patient_high";
const LOW_RECORD: &str = "COALESCE(a.person_id, patient_low)";
const HIGH_RECORD: &str = "COALESCE(b.person_id, patient_high)";

/// "Possible duplicates (N)": the number of distinct pairs of RECORDS with an open proposal.
/// ONE statement, no per-chart reads, and deliberately NO veto or dispute column — the front
/// door pays for this on every show, and `cairn_match_veto` per row is the list's cost, never
/// the count's.
pub async fn worklist_count(client: &(impl GenericClient + Sync)) -> anyhow::Result<usize> {
    let sql = format!(
        "SELECT count(*) FROM (SELECT DISTINCT LEAST(lr, hr), GREATEST(lr, hr) \
           FROM (SELECT {LOW_RECORD} AS lr, {HIGH_RECORD} AS hr {RECORDS_FROM}) r \
          WHERE lr <> hr) s"
    );
    let n: i64 = client
        .query_one(&sql, &[])
        .await
        .context("counting the open possible duplicates")?
        .get(0);
    Ok(n as usize)
}

/// The worklist: every entry grouped, the newest `limit` read in full (both records).
///
/// `total` is the number of entries at read time. An entry whose two records have meanwhile
/// come to share a chart (a link landing between this function's reads) is not shown; `total`
/// still counts it, so "N more" can be one high in that race — the next read is exact.
pub async fn worklist(
    client: &(impl GenericClient + Sync),
    limit: usize,
) -> anyhow::Result<Worklist> {
    let sql = format!(
        "SELECT patient_low::text AS low, patient_high::text AS high, \
                ({LOW_RECORD})::text AS low_record, ({HIGH_RECORD})::text AS high_record, \
                band, status, \
                EXISTS (SELECT 1 FROM cairn_match_veto(patient_low, patient_high)) AS vetoed, \
                {DISPUTED_SQL} AS disputed, \
                (extract(epoch FROM created_at) * 1000)::bigint AS created_ms \
         {RECORDS_FROM} \
         ORDER BY created_at DESC, patient_low, patient_high"
    );
    let rows = client
        .query(&sql, &[])
        .await
        .context("reading the open possible duplicates")?;
    let parse = |r: &tokio_postgres::Row, col: &str| -> anyhow::Result<Uuid> {
        Ok(r.get::<_, String>(col).parse()?)
    };
    let rows: Vec<ProposalRow> = rows
        .iter()
        .map(|r| {
            Ok(ProposalRow {
                low: parse(r, "low")?,
                high: parse(r, "high")?,
                low_record: parse(r, "low_record")?,
                high_record: parse(r, "high_record")?,
                band: r.get("band"),
                status: r.get("status"),
                vetoed: r.get("vetoed"),
                disputed: r.get("disputed"),
                created_ms: r.get("created_ms"),
            })
        })
        .collect::<anyhow::Result<_>>()?;
    let entries = group_by_record_pair(&rows);
    let total = entries.len();
    let mut items = Vec::new();
    for entry in entries.into_iter().take(limit) {
        let newer = person_charts(client, entry.open_chart).await?;
        let older = person_charts(client, entry.older_chart).await?;
        if is_another_record(&older, &newer) {
            items.push(WorklistItem {
                entry,
                newer,
                older,
            });
        }
    }
    Ok(Worklist { items, total })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A chart id whose UUIDv7 time is `ms`, with `n` telling ids of one millisecond apart.
    fn at(ms: u64, n: u128) -> Uuid {
        Uuid::from_u128(((ms as u128) << 80) | n)
    }
    fn row(low: Uuid, high: Uuid, lr: Uuid, hr: Uuid, created_ms: i64) -> ProposalRow {
        ProposalRow {
            low: low.min(high),
            high: low.max(high),
            low_record: if low < high { lr } else { hr },
            high_record: if low < high { hr } else { lr },
            band: "review".into(),
            status: "pending".into(),
            vetoed: false,
            disputed: false,
            created_ms,
        }
    }

    #[test]
    fn registered_ms_reads_the_uuidv7_time() {
        assert_eq!(registered_ms(at(1_700_000_000_000, 7)), 1_700_000_000_000);
    }

    #[test]
    fn two_members_against_one_record_are_one_entry_with_both_pairs() {
        let (a1, a2, b) = (at(10, 1), at(11, 2), at(50, 3));
        let rec_a = a1; // a1 and a2 are one record keyed a1
        let rows = vec![row(a1, b, rec_a, b, 100), row(a2, b, rec_a, b, 200)];
        let got = group_by_record_pair(&rows);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].pairs.len(), 2);
        assert_eq!(got[0].newest_ms, 200);
    }

    #[test]
    fn the_newer_record_holds_the_most_recently_registered_chart_and_review_opens_it() {
        let (old, new) = (at(10, 1), at(99, 2));
        let got = group_by_record_pair(&[row(old, new, old, new, 5)]);
        assert_eq!(got[0].newer_record, new);
        assert_eq!(got[0].older_record, old);
        assert_eq!((got[0].open_chart, got[0].older_chart), (new, old));
        // Either orientation of the input row gives the same answer.
        assert_eq!(group_by_record_pair(&[row(new, old, new, old, 5)]), got);
    }

    #[test]
    fn a_registration_time_tie_goes_to_the_smaller_id() {
        let (x, y) = (at(10, 1), at(10, 2));
        let got = group_by_record_pair(&[row(x, y, x, y, 5)]);
        assert_eq!(got[0].newer_record, x);
    }

    #[test]
    fn a_pair_inside_one_record_is_not_an_entry() {
        let (x, y) = (at(10, 1), at(11, 2));
        assert!(group_by_record_pair(&[row(x, y, x, x, 5)]).is_empty());
    }

    #[test]
    fn flags_and_band_are_the_strongest_over_the_pairs() {
        let (a1, a2, b) = (at(10, 1), at(11, 2), at(50, 3));
        let mut r1 = row(a1, b, a1, b, 1);
        r1.band = "auto_candidate".into();
        r1.disputed = true;
        let mut r2 = row(a2, b, a1, b, 2);
        r2.status = "accepted".into();
        r2.vetoed = true;
        let got = &group_by_record_pair(&[r1, r2])[0];
        assert_eq!(got.band, "auto_candidate");
        assert!(got.vetoed && got.disputed && got.accepted);
    }

    #[test]
    fn entries_are_newest_first_and_input_order_does_not_matter() {
        let (p, q, r, s) = (at(1, 1), at(2, 2), at(3, 3), at(4, 4));
        let rows = vec![row(p, q, p, q, 10), row(r, s, r, s, 20)];
        let got = group_by_record_pair(&rows);
        assert_eq!(got[0].newest_ms, 20);
        let mut rev = rows.clone();
        rev.reverse();
        assert_eq!(group_by_record_pair(&rev), got);
    }
}
