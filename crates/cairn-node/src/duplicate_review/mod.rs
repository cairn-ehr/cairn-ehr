//! The possible-duplicate banner's node reads, and its "Different people" judgement (repair
//! path R5a, #680; design page "R5a — the banner, designed 2026-10-06").
//!
//! R4's worker writes `match_proposal` rows (db/017). Which of them still need a human is ONE
//! answer — db/057's view `match_proposal_open` — and every "is it open?" question here is
//! answered by that view (the rest is reading records, the veto floor, and `unlink_charts`):
//! - [`possible_duplicates`]: every open proposal between a displayed chart set and a chart
//!   OUTSIDE it, grouped by the other side's RECORD, so the banner shows one entry per other
//!   person however many of their charts were proposed;
//! - [`open_pairs_between`]: the open pairs joining two records — Review's admission (the window
//!   does not widen `AppState::shown`; it asks this, at that moment) and what "Different people"
//!   judges;
//! - [`record_different_people`]: an attested unlink on each of those pairs.
//!
//! Nothing here links, and nothing here writes a proposal status: `chart_link::unlink_charts`
//! writes the event and moves the proposal, in its own transaction, as it does for R2b-2.

use crate::chart_link::{canonical_pair, unlink_charts, LinkOutcome, Reviewer};
use crate::patient::person::person_charts;
use anyhow::Context;
use cairn_medication_view::ChartSet;
use std::collections::{BTreeMap, HashMap};
use tokio_postgres::{Client, GenericClient};
use uuid::Uuid;

pub mod worklist;

/// "Another writer recorded these two as different people, without a clinician's confirmation"
/// (ADR-0078): an UN-attested unlink stands for the open proposal's pair. The ONE spelling — the
/// banner (`open_proposals_touching`) and the worklist both select it — written over
/// `match_proposal_open`'s own column names, so it reads the row the query is on.
pub const DISPUTED_SQL: &str = "EXISTS (SELECT 1 FROM patient_link pl \
     WHERE pl.low = patient_low AND pl.high = patient_high \
       AND pl.state = 'unlink' AND NOT pl.attested)";

/// One open proposal that crosses a record's boundary: `here` is inside the displayed set,
/// `other` outside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenProposal {
    pub here: Uuid,
    pub other: Uuid,
    /// The db/016 veto floor finds a disagreement between the two charts NOW
    /// (`cairn_match_veto`, read in the same query — the predicate `auto_apply.rs` re-checks).
    /// Never the proposal's stored `veto_findings`: those are propose-time, so a pair that
    /// became vetoed after it was proposed (which `auto_apply.rs` moves to `review`, leaving the
    /// findings untouched) would show no note, and a corrected fact would leave a false one.
    /// Shown as a note only; Review shows WHICH facts, read fresh by the compare panel.
    pub vetoed: bool,
    /// When the proposal was written, epoch milliseconds — ordering only.
    pub created_ms: i64,
    /// A human already said "same person" through C2 (status `accepted`);
    /// `apply_accepted_proposal` has not yet run (#736). "Different people" must not overrule it.
    pub accepted: bool,
    /// Another writer's un-attested unlink stands for the pair ([`DISPUTED_SQL`], ADR-0078).
    pub disputed: bool,
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
    /// for grouping and inspection. "Different people" NEVER uses these: they are as stale as
    /// the banner, so [`record_different_people`] re-reads the open pairs itself.
    pub pairs: Vec<(Uuid, Uuid)>,
    /// Any of `pairs` is vetoed by the db/016 floor now ([`OpenProposal::vetoed`]).
    pub vetoed: bool,
    /// The newest of `pairs`' `created_ms` — orders the banner's entries, newest first.
    pub newest_ms: i64,
    /// Any of `pairs` is already accepted as the same person ([`OpenProposal::accepted`]).
    pub accepted: bool,
    /// Any of `pairs` is disputed by another writer's unlink ([`OpenProposal::disputed`]).
    pub disputed: bool,
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
///
/// Input-order-independent: on a `created_ms` tie the SMALLER `other` chart is Review's chart,
/// so the same proposals in any order give the same entry (the SQL read orders ties, but this
/// function does not rely on it).
pub fn group_by_other_record(found: Vec<(OpenProposal, ChartSet)>) -> Vec<PossibleDuplicate> {
    let mut groups: BTreeMap<Vec<Uuid>, PossibleDuplicate> = BTreeMap::new();
    for (p, record) in found {
        let pair = canonical_pair(p.here, p.other);
        let key = record.members().to_vec();
        match groups.get_mut(&key) {
            Some(g) => {
                g.pairs.push(pair);
                g.vetoed |= p.vetoed;
                g.accepted |= p.accepted;
                g.disputed |= p.disputed;
                if p.created_ms > g.newest_ms {
                    g.newest_ms = p.created_ms;
                    g.review_chart = p.other;
                } else if p.created_ms == g.newest_ms && p.other < g.review_chart {
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
                        accepted: p.accepted,
                        disputed: p.disputed,
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

/// Whether `record` is a DIFFERENT record from the displayed `charts` — shares none of its
/// charts. **Pure.** A record holding a displayed chart reads as one with it (db/057's own "not
/// open"), which only a link landing between two of [`possible_duplicates`]' reads can produce.
pub fn is_another_record(record: &ChartSet, charts: &ChartSet) -> bool {
    !record.members().iter().any(|c| charts.contains(c))
}

fn ids(charts: &ChartSet) -> Vec<String> {
    charts.members().iter().map(Uuid::to_string).collect()
}

/// Every open proposal with exactly one side in `charts`, newest first. `vetoed` is the db/016
/// floor evaluated in this same query (see [`OpenProposal::vetoed`]).
pub async fn open_proposals_touching(
    client: &(impl GenericClient + Sync),
    charts: &ChartSet,
) -> anyhow::Result<Vec<OpenProposal>> {
    // `DISPUTED_SQL` is inlined, never re-spelled: one spelling of "disputed" for banner and worklist.
    let sql = format!(
        "SELECT patient_low::text AS low, patient_high::text AS high, \
                EXISTS (SELECT 1 FROM cairn_match_veto(patient_low, patient_high)) AS vetoed, \
                (extract(epoch FROM created_at) * 1000)::bigint AS created_ms, \
                status = 'accepted' AS accepted, \
                {DISPUTED_SQL} AS disputed \
           FROM match_proposal_open \
          WHERE (patient_low = ANY($1::text[]::uuid[])) \
             <> (patient_high = ANY($1::text[]::uuid[])) \
          ORDER BY created_at DESC, patient_low, patient_high"
    );
    let rows = client
        .query(&sql, &[&ids(charts)])
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
                accepted: r.get("accepted"),
                disputed: r.get("disputed"),
            })
        })
        .collect()
}

/// The banner's entries for `charts`: open proposals grouped by the other side's record.
/// One `person_charts` read per distinct other chart; a record that has meanwhile come to hold
/// a displayed chart is dropped ([`is_another_record`]) — the next read no longer lists it.
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
        if is_another_record(&record, charts) {
            found.push((p, record));
        }
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

/// The open pairs between two records that a human has already ACCEPTED as the same person
/// (status `accepted`, #736). "Different people" must never overrule one.
pub async fn accepted_pairs_between(
    client: &(impl GenericClient + Sync),
    left: &ChartSet,
    right: &ChartSet,
) -> anyhow::Result<Vec<(Uuid, Uuid)>> {
    let rows = client
        .query(
            "SELECT patient_low::text, patient_high::text FROM match_proposal_open \
              WHERE status = 'accepted' AND \
                ((patient_low = ANY($1::text[]::uuid[]) AND patient_high = ANY($2::text[]::uuid[])) \
              OR (patient_low = ANY($2::text[]::uuid[]) AND patient_high = ANY($1::text[]::uuid[]))) \
              ORDER BY 1, 2",
            &[&ids(left), &ids(right)],
        )
        .await
        .context("reading whether a human already accepted these as the same person")?;
    rows.iter()
        .map(|r| {
            Ok((
                r.get::<_, String>(0).parse()?,
                r.get::<_, String>(1).parse()?,
            ))
        })
        .collect()
}

/// One pair's "Different people" judgement and what it did (or why it was not recorded).
#[derive(Debug)]
pub struct PairJudgement {
    pub low: Uuid,
    pub high: Uuid,
    pub outcome: anyhow::Result<LinkOutcome>,
}

/// What "Different people" did.
#[derive(Debug)]
pub enum DifferentPeople {
    /// No open proposal joins the two records any more (a colleague's judgement, here or by
    /// sync, resolved it since the banner was drawn). Nothing was signed.
    NothingOpen,
    /// A human has already accepted (some of) these pairs as the SAME person (#736); that
    /// judgement awaits linking and is not overruled from here. Nothing was signed.
    AcceptedAsSame,
    /// One attested unlink per open pair, each in its own transaction (`unlink_charts`).
    Judged(Vec<PairJudgement>),
}

/// "Different people": an attested unlink on EVERY open pair between `left` (the displayed
/// record) and `right` (the other record). The PAIRS are read fresh here — never the pairs the
/// banner showed, which may be stale. The two chart SETS are the caller's: it must pass sets it
/// has just read (the window re-reads both and refuses one that changed since the comparison),
/// because a pair joining a chart outside them is not looked for. Each is
/// `unlink_charts(low, high, None, …)`: a proposal's charts are normally both held here, so the
/// judgement files under a subject, and a pair whose in-record side is a linked member — not
/// the chart on screen — is judged the same way (#699 (a)'s third-chart filing is for unlinking
/// a standing link, not a proposal). A proposal CAN name a chart not held here — the matcher
/// scores any chart with identity projections, e.g. demographics synced ahead of the
/// registration — and that pair is refused before anything is signed, carried in its
/// [`PairJudgement`] (pinned by `one_pairs_failure_is_carried_and_the_others_stand`).
///
/// Honest, not atomic: several pairs are several events. A failure on one is carried in its
/// [`PairJudgement`] and the others still stand. A REFUSED pair (nothing signed) stays open
/// and stays on the banner; but `unlink_charts`'s "commit outcome unknown for event …" error
/// may have committed — that pair may already be judged and off the banner, and only a re-read
/// says which. `Err` only when the open pairs could not be read — nothing was signed.
///
/// CONTRACT the window relies on: `Err` happens only BEFORE the first signature. The window words
/// every `Err` "nothing was done", so a fallible step added after the loop starts must carry its
/// failure in a [`PairJudgement`], never return it with `?`.
pub async fn record_different_people(
    client: &mut Client,
    left: &ChartSet,
    right: &ChartSet,
    reviewer: &Reviewer<'_>,
    node_origin: &str,
) -> anyhow::Result<DifferentPeople> {
    let pairs = open_pairs_between(&*client, left, right).await?;
    if pairs.is_empty() {
        return Ok(DifferentPeople::NothingOpen);
    }
    // Before the first signature, so the "Err only before signing" contract holds.
    if !accepted_pairs_between(&*client, left, right)
        .await?
        .is_empty()
    {
        return Ok(DifferentPeople::AcceptedAsSame);
    }
    let mut judged = Vec::with_capacity(pairs.len());
    for (low, high) in pairs {
        let outcome = unlink_charts(client, low, high, None, reviewer, node_origin).await;
        judged.push(PairJudgement { low, high, outcome });
    }
    Ok(DifferentPeople::Judged(judged))
}

#[cfg(test)]
mod tests;
