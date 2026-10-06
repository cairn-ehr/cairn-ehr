//! The possible-duplicate banner's node reads, and its "Different people" judgement (repair
//! path R5a, #680; design page "R5a — the banner, designed 2026-10-06").
//!
//! R4's worker writes `match_proposal` rows (db/017). Which of them still need a human is ONE
//! answer — db/057's view `match_proposal_open` — and everything here reads only that view:
//! - [`possible_duplicates`]: every open proposal between a displayed chart set and a chart
//!   OUTSIDE it, grouped by the other side's RECORD, so the banner shows one entry per other
//!   person however many of their charts were proposed;
//! - [`open_pairs_between`]: the open pairs joining two records — Review's admission (the window
//!   does not widen `AppState::shown`; it asks this, at that moment) and what "Different people"
//!   judges;
//! - `record_different_people` (added by the next slice task): an attested unlink on each of those pairs.
//!
//! Nothing here links, and nothing here writes a proposal status: `chart_link::unlink_charts`
//! writes the event and moves the proposal, in its own transaction, as it does for R2b-2.

#[allow(unused_imports)]
// `unlink_charts`, `LinkOutcome`, `Reviewer` are used by the "Different people" judgement (Task 3).
use crate::chart_link::{canonical_pair, unlink_charts, LinkOutcome, Reviewer};
use crate::patient::person::person_charts;
use anyhow::Context;
use cairn_medication_view::ChartSet;
use std::collections::{BTreeMap, HashMap};
#[allow(unused_imports)] // `Client` is used by Task 3.
use tokio_postgres::{Client, GenericClient};
use uuid::Uuid;

/// One open proposal that crosses a record's boundary: `here` is inside the displayed set,
/// `other` outside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenProposal {
    pub here: Uuid,
    pub other: Uuid,
    /// The matcher recorded veto findings for the pair (shown as a note; Review shows WHICH,
    /// read fresh by the compare panel — never worded here from the stored JSON).
    pub vetoed: bool,
    /// When the proposal was written, epoch milliseconds — ordering only.
    pub created_ms: i64,
}

/// One banner entry: every open proposal between the displayed set and ONE other record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PossibleDuplicate {
    /// The other side's whole record (`person_charts`), so the banner can name every chart of
    /// that person and read their medications as one list.
    pub other_record: ChartSet,
    /// The chart Review compares against: the other side of the NEWEST proposal.
    pub review_chart: Uuid,
    /// Every open pair this entry stands for, canonical `(low, high)`, sorted, no duplicates —
    /// what "Different people" records an unlink on.
    pub pairs: Vec<(Uuid, Uuid)>,
    pub vetoed: bool,
    pub newest_ms: i64,
}

/// `(here, other)` for a pair crossing `charts`'s boundary, or `None` when both or neither
/// side is inside it. **Pure.**
pub fn orient(low: Uuid, high: Uuid, charts: &ChartSet) -> Option<(Uuid, Uuid)> {
    match (charts.contains(&low), charts.contains(&high)) {
        (true, false) => Some((low, high)),
        (false, true) => Some((high, low)),
        _ => None,
    }
}

/// Group proposals by the other side's record; newest entry first. **Pure.**
///
/// Each input carries the record its `other` chart belongs to (read by the caller). Two of my
/// charts proposed against two charts of the same other person are ONE entry: the banner is
/// about people, as the front door is since R3.
pub fn group_by_other_record(found: Vec<(OpenProposal, ChartSet)>) -> Vec<PossibleDuplicate> {
    let mut groups: BTreeMap<Vec<Uuid>, PossibleDuplicate> = BTreeMap::new();
    for (p, record) in found {
        let pair = canonical_pair(p.here, p.other);
        let key = record.members().to_vec();
        match groups.get_mut(&key) {
            Some(g) => {
                g.pairs.push(pair);
                g.vetoed |= p.vetoed;
                if p.created_ms > g.newest_ms {
                    g.newest_ms = p.created_ms;
                    g.review_chart = p.other;
                }
            }
            None => {
                groups.insert(
                    key,
                    PossibleDuplicate {
                        other_record: record,
                        review_chart: p.other,
                        pairs: vec![pair],
                        vetoed: p.vetoed,
                        newest_ms: p.created_ms,
                    },
                );
            }
        }
    }
    let mut entries: Vec<PossibleDuplicate> = groups
        .into_values()
        .map(|mut g| {
            g.pairs.sort();
            g.pairs.dedup();
            g
        })
        .collect();
    entries.sort_by(|a, b| {
        b.newest_ms
            .cmp(&a.newest_ms)
            .then_with(|| a.review_chart.cmp(&b.review_chart))
    });
    entries
}

fn ids(charts: &ChartSet) -> Vec<String> {
    charts.members().iter().map(Uuid::to_string).collect()
}

/// Every open proposal with exactly one side in `charts`, newest first.
pub async fn open_proposals_touching(
    client: &(impl GenericClient + Sync),
    charts: &ChartSet,
) -> anyhow::Result<Vec<OpenProposal>> {
    let rows = client
        .query(
            "SELECT patient_low::text AS low, patient_high::text AS high, \
                    veto_findings <> '[]'::jsonb AS vetoed, \
                    (extract(epoch FROM created_at) * 1000)::bigint AS created_ms \
               FROM match_proposal_open \
              WHERE (patient_low = ANY($1::text[]::uuid[])) \
                 <> (patient_high = ANY($1::text[]::uuid[])) \
              ORDER BY created_at DESC, patient_low, patient_high",
            &[&ids(charts)],
        )
        .await
        .context("reading the open duplicate proposals for this record")?;
    rows.iter()
        .map(|r| {
            let low: Uuid = r.get::<_, String>("low").parse()?;
            let high: Uuid = r.get::<_, String>("high").parse()?;
            let (here, other) = orient(low, high, charts)
                .with_context(|| format!("proposal {low}–{high} does not cross this record"))?;
            Ok(OpenProposal {
                here,
                other,
                vetoed: r.get("vetoed"),
                created_ms: r.get("created_ms"),
            })
        })
        .collect()
}

/// The banner's entries for `charts`: open proposals grouped by the other side's record.
/// One `person_charts` read per distinct other chart.
pub async fn possible_duplicates(
    client: &(impl GenericClient + Sync),
    charts: &ChartSet,
) -> anyhow::Result<Vec<PossibleDuplicate>> {
    let open = open_proposals_touching(client, charts).await?;
    let mut records: HashMap<Uuid, ChartSet> = HashMap::new();
    let mut found = Vec::with_capacity(open.len());
    for p in open {
        let record = match records.get(&p.other) {
            Some(r) => r.clone(),
            None => {
                let r = person_charts(client, p.other).await?;
                records.insert(p.other, r.clone());
                r
            }
        };
        found.push((p, record));
    }
    Ok(group_by_other_record(found))
}

/// The open pairs joining `left` and `right` (either orientation), canonical and sorted.
/// Empty means: no open proposal joins the two records any more.
pub async fn open_pairs_between(
    client: &(impl GenericClient + Sync),
    left: &ChartSet,
    right: &ChartSet,
) -> anyhow::Result<Vec<(Uuid, Uuid)>> {
    let rows = client
        .query(
            "SELECT patient_low::text, patient_high::text FROM match_proposal_open \
              WHERE (patient_low = ANY($1::text[]::uuid[]) AND patient_high = ANY($2::text[]::uuid[])) \
                 OR (patient_low = ANY($2::text[]::uuid[]) AND patient_high = ANY($1::text[]::uuid[])) \
              ORDER BY 1, 2",
            &[&ids(left), &ids(right)],
        )
        .await
        .context("reading the open duplicate proposals between two records")?;
    rows.iter()
        .map(|r| {
            Ok((
                r.get::<_, String>(0).parse()?,
                r.get::<_, String>(1).parse()?,
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }
    fn set(v: &[u128]) -> ChartSet {
        ChartSet::new(v.iter().map(|n| id(*n))).unwrap()
    }
    fn prop(here: u128, other: u128, vetoed: bool, created_ms: i64) -> OpenProposal {
        OpenProposal {
            here: id(here),
            other: id(other),
            vetoed,
            created_ms,
        }
    }

    #[test]
    fn a_pair_is_oriented_by_which_side_the_record_holds() {
        let record = set(&[1, 2]);
        assert_eq!(orient(id(1), id(9), &record), Some((id(1), id(9))));
        assert_eq!(orient(id(2), id(9), &record), Some((id(2), id(9))));
        assert_eq!(orient(id(0), id(1), &record), Some((id(1), id(0))));
        // Both inside or both outside: not a banner pair.
        assert_eq!(orient(id(1), id(2), &record), None);
        assert_eq!(orient(id(8), id(9), &record), None);
    }

    /// Review Focus 2: two of my charts proposed against two charts of ONE other record are one
    /// entry, standing for both pairs; Review compares against the NEWEST proposal's chart.
    #[test]
    fn proposals_against_one_other_record_are_one_entry() {
        let other = set(&[8, 9]);
        let got = group_by_other_record(vec![
            (prop(1, 8, false, 100), other.clone()),
            (prop(2, 9, true, 200), other.clone()),
        ]);
        assert_eq!(got.len(), 1);
        let e = &got[0];
        assert_eq!(e.other_record, other);
        assert_eq!(e.review_chart, id(9), "the newest proposal's other chart");
        assert_eq!(e.pairs, vec![(id(1), id(8)), (id(2), id(9))]);
        assert!(e.vetoed, "any vetoed pair marks the entry");
        assert_eq!(e.newest_ms, 200);
    }

    #[test]
    fn entries_are_newest_first_and_records_stay_apart() {
        let got = group_by_other_record(vec![
            (prop(1, 7, false, 100), set(&[7])),
            (prop(1, 9, false, 300), set(&[9])),
        ]);
        let order: Vec<Uuid> = got.iter().map(|e| e.review_chart).collect();
        assert_eq!(order, vec![id(9), id(7)]);
    }

    #[test]
    fn nothing_found_is_no_entries() {
        assert!(group_by_other_record(vec![]).is_empty());
    }
}
