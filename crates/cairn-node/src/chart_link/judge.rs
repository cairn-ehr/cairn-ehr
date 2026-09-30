//! The two public entry points of a human's link/unlink judgement — [`link_charts`] and
//! [`unlink_charts`] — and what they report back ([`LinkOutcome`], [`LinkEffect`]).
//!
//! Split out of `chart_link.rs` (house rule 4: files under 500 lines) with no change in
//! behaviour; every item is re-exported by the parent, so `chart_link::unlink_charts` and
//! friends keep their paths. The flow: `judge` makes the legible pre-checks (is each chart
//! held? may the judgement be made, and filed under which chart — the pure rule in
//! `admit.rs`?), then opens ONE transaction that signs and submits through the parent's
//! [`assert_link_in_tx`] and reads back what the judgement did, before committing.

use super::{
    admit_judgement, assert_link_in_tx, canonical_pair, compose_review_provenance,
    record_holds_both, FiledUnder, LinkVerb, OpenedChart, Reviewer,
};
use crate::db_diagnosis::{deliberate_refusal, node_state_refusal, LocalDbFault};
use anyhow::Context;
use cairn_medication_view::ChartSet;
use uuid::Uuid;

/// What a recorded judgement did to the record. A judgement is ALWAYS recorded once it is
/// committed — it is a real event and replicates (ADR-0076 decision 4) — but recording it
/// is not the same as it taking effect, and a caller must never report the one as the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkEffect {
    /// The record reads the way it says: a link joined the charts; an unlink left them
    /// apart. Usually its own event is the pair's standing assertion; it may instead be a
    /// later judgement that says the SAME thing (e.g. a peer's, from a clock ahead) —
    /// either way there is nothing left to do.
    TookEffect,
    /// Another assertion about the SAME pair that says the OPPOSITE outranks it — a later
    /// human judgement (higher HLC; e.g. a peer's, from a clock ahead of this node's). The
    /// direct edge still stands as that other judgement says. Judging again changes
    /// nothing; it is a disagreement between humans for a human to settle.
    Outranked,
    /// An UNLINK that stands on its own edge, but the second chart still reads as part of
    /// the first's record through ANOTHER link (A–C–B: unlinking A from B leaves A–C and
    /// C–B standing). It cannot split the record on its own; the remedy is to unlink that
    /// other link too. The machine never picks which edge is wrong (principle 2), so this is
    /// reported, never auto-resolved. Never the effect of a link.
    StillJoined,
}

/// What a judgement did, from two facts read inside its own transaction. **Pure**, so the
/// three outcomes are unit-tested apart from the database.
///
/// `agrees`: the pair's standing `patient_link` assertion says what this judgement says
/// (see [`Asserted::agrees`](super::Asserted::agrees)). `still_joined`: the two subjects still
/// read as one record (`high ∈ person_charts(low)`), read inside the judgement's transaction.
pub fn link_effect(verb: LinkVerb, agrees: bool, still_joined: bool) -> LinkEffect {
    match (verb, agrees, still_joined) {
        (_, false, _) => LinkEffect::Outranked,
        (LinkVerb::Unlink, true, true) => LinkEffect::StillJoined,
        _ => LinkEffect::TookEffect,
    }
}

/// What a judgement wrote, and what the chart now is.
#[derive(Debug)]
pub struct LinkOutcome {
    /// The attested identity event.
    pub event_id: Uuid,
    /// Whether an OPEN `match_proposal` for the pair moved (`applied` / `rejected`).
    pub proposal_resolved: bool,
    /// The chart the event is filed under — always one this node HOLDS (see
    /// [`admit_judgement`]): a subject, or for an unlink judged from an open record, that
    /// chart (#699 (a)) — whichever order the caller named the two charts in.
    pub filed_under: Uuid,
    /// The chart whose record [`LinkOutcome::charts`] is: the chart the judgement was made
    /// from when the caller named one (`unlink_charts`'s `opened`), else
    /// [`LinkOutcome::filed_under`].
    pub record_of: Uuid,
    /// The chart set of [`LinkOutcome::record_of`], read inside the judgement's own
    /// transaction: what that chart reads as, now that the judgement is recorded.
    pub charts: ChartSet,
    /// What the judgement did to the record — see [`LinkEffect`].
    pub effect: LinkEffect,
}

/// "Same person": link two charts as the reviewer's attested judgement. Both charts must
/// be held here. [`LinkOutcome::effect`] says whether it took effect.
///
/// `Err` means nothing was written — with ONE exception, which says so: if the connection
/// fails DURING the commit, the database may have committed it. That error names the
/// event ("commit outcome unknown for event …"); look for the event before retrying, or a
/// retry may record the same judgement twice.
pub async fn link_charts(
    client: &mut tokio_postgres::Client,
    a: Uuid,
    b: Uuid,
    reviewer: &Reviewer<'_>,
    node_origin: &str,
) -> anyhow::Result<LinkOutcome> {
    judge(client, LinkVerb::Link, a, b, None, reviewer, node_origin).await
}

/// "Not the same person": record the reviewer's attested judgement that two charts are two
/// people. On a DIRECTLY linked pair it splits them. On a pair never linked it is the record
/// that they were looked at and are different (decision 4), which no machine link then
/// undoes. On a pair joined only THROUGH other charts (A–C–B) it is recorded on the A–B
/// edge but cannot split the record — [`LinkEffect::StillJoined`] says so, and the other
/// link must be unlinked too.
///
/// One chart may be a member this node does not hold (R1 shows it; a peer's link named
/// it), provided it reads as part of the other chart's record here.
///
/// `opened` is the chart the clinician is judging FROM — the one on screen, whose record
/// [`LinkOutcome::charts`] then is. When it is not one of the pair it must be held here and
/// its record must read both charts as part of it, or the unlink is refused before anything
/// is signed. It carries the filing only when NEITHER chart is held here (#699 (a)): the far
/// link B–C of an A–B–C record, read on a node holding only A; with `None` that case is
/// refused.
///
/// Errors: as [`link_charts`] — only "commit outcome unknown for event …" may have written.
pub async fn unlink_charts(
    client: &mut tokio_postgres::Client,
    a: Uuid,
    b: Uuid,
    opened: Option<Uuid>,
    reviewer: &Reviewer<'_>,
    node_origin: &str,
) -> anyhow::Result<LinkOutcome> {
    judge(
        client,
        LinkVerb::Unlink,
        a,
        b,
        opened,
        reviewer,
        node_origin,
    )
    .await
}

/// Does this node hold `chart` itself (its `patient_chart` row, made by its registration)?
async fn is_held(client: &tokio_postgres::Client, chart: Uuid) -> anyhow::Result<bool> {
    Ok(client
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM patient_chart WHERE patient_id = $1::text::uuid)",
            &[&chart.to_string()],
        )
        .await
        .map_err(|e| LocalDbFault::new("checking whether this node holds the chart", e))?
        .get(0))
}

/// The shared body of both entry points: pre-checks (legible refusals before anything is
/// signed), then ONE transaction that writes the judgement AND reads back what it did — so
/// once the commit succeeds there is nothing left that can fail. `opened`: the chart the
/// judgement is made from, if the caller named one (see [`unlink_charts`]).
async fn judge(
    client: &mut tokio_postgres::Client,
    verb: LinkVerb,
    a: Uuid,
    b: Uuid,
    opened: Option<Uuid>,
    reviewer: &Reviewer<'_>,
    node_origin: &str,
) -> anyhow::Result<LinkOutcome> {
    if a == b {
        // A verdict about the INPUT: no retry, by anyone, ever changes it.
        return Err(deliberate_refusal(format!(
            "{a} and {b} are the same chart — a chart cannot be linked to itself"
        )));
    }
    let (a_held, b_held) = (is_held(client, a).await?, is_held(client, b).await?);
    // Only asked when it can change the answer: an unlink with exactly one chart unheld.
    let shared_record = verb == LinkVerb::Unlink
        && a_held != b_held
        && crate::patient::person::person_charts(&*client, a)
            .await
            .context("reading whether the two charts share a record here")?
            .contains(&b);
    // The chart an unlink is judged FROM, when it is a THIRD chart (not a subject): read so
    // `admit_judgement` can check it — it may carry the filing when neither subject is held
    // (#699 (a)), and whichever chart carries it, its record is what the caller is shown next,
    // so it must be a record that exists here and holds the pair (Ruling R5). Pre-checks for a
    // LEGIBLE refusal; db/005 step 8b is the enforcement for the filing (it refuses an event
    // filed under a chart with no history here), and a third-chart filing's record is read
    // AGAIN inside the transaction below, before anything is signed.
    let opened_chart = match (verb, opened) {
        (LinkVerb::Unlink, Some(o)) if o != a && o != b => {
            let record = crate::patient::person::person_charts(&*client, o)
                .await
                .context("reading the open chart's record")?;
            Some(OpenedChart {
                chart: o,
                held: is_held(client, o).await?,
                holds_both: record_holds_both(&record, a, b),
            })
        }
        _ => None,
    };
    // A verdict about this NODE's state: the identical call succeeds once the chart (or the
    // record joining them) has arrived here. Marked, so a surface words it as a verdict and
    // never as an outage to retry (#702).
    let filed = admit_judgement(verb, (a, a_held), (b, b_held), shared_record, opened_chart)
        .map_err(node_state_refusal)?;
    let about = filed.chart();
    // Legibility only; the db/005 gate is the enforcement (a raw-SQL client skipping this
    // still cannot attest with a non-human key).
    if !crate::identify::attester_is_enrolled_human(client, reviewer.human_kid)
        .await
        .context("checking the reviewer's key is an enrolled human")?
    {
        return Err(node_state_refusal(format!(
            "key {} is not an enrolled human actor — linking or unlinking charts is a human \
             judgement (unlock a clinician's key)",
            reviewer.human_kid
        )));
    }

    let (low, high) = canonical_pair(a, b);
    let provenance = compose_review_provenance(verb, reviewer.human_kid);
    // The tick self-commits before the transaction; a rolled-back judgement leaves only a
    // clock gap, which the HLC allows (the identify_patient shape).
    let hlc = crate::db::next_hlc(client, node_origin).await?;
    let tx = client
        .transaction()
        .await
        .map_err(|e| LocalDbFault::new("opening the judgement's transaction", e))?;
    // A third-chart filing rests on a fact about the RECORD (the open chart's record holds
    // both subjects), and the record can change between the pre-check above and this
    // transaction (a peer's unlink syncing in). Re-read it here, before anything is signed:
    // a refusal now rolls the transaction back with nothing written. `filing_for` cannot
    // check this — it is pure.
    if let FiledUnder::RecordOf(o) = filed {
        let record = crate::patient::person::person_charts(&tx, o)
            .await
            .context("re-reading the open chart's record inside the judgement")?;
        if !record_holds_both(&record, a, b) {
            return Err(node_state_refusal(format!(
                "chart {o}'s record no longer reads both {a} and {b} as part of it — the \
                 record changed while the judgement was being made; open it again"
            )));
        }
    }
    let asserted = assert_link_in_tx(
        &tx,
        verb,
        low,
        high,
        filed,
        &provenance,
        None,
        reviewer,
        hlc,
    )
    .await?;
    // The record the clinician judged FROM (the open chart), else the filed-under chart —
    // what the caller shows next. Read BEFORE the commit, in the same transaction, so a
    // failure here rolls the judgement back rather than leaving a committed event behind an
    // error.
    let record_of = opened.unwrap_or(about);
    let charts = crate::patient::person::person_charts(&tx, record_of)
        .await
        .context("reading the chart set the judgement leaves")?;
    // "Still joined?" is a question about the two SUBJECTS — do they still read as one
    // record? — asked of the subjects themselves, never of the filed-under chart: once an
    // unlink may be filed under a THIRD chart (#699 (a)), "is the other chart in the filed-under
    // chart's record" answers StillJoined for every successful split (the far link of A–B–C,
    // filed under A, leaves B in A's record). Read in this transaction, like `charts`.
    let joined = crate::patient::person::person_charts(&tx, low)
        .await
        .context("reading whether the two charts still read as one record")?
        .contains(&high);
    let effect = link_effect(verb, asserted.agrees, joined);

    // The one failure that may have written: a connection lost DURING the commit leaves
    // its outcome unknown. Name the event so the operator looks before retrying.
    tx.commit().await.map_err(|e| {
        anyhow::Error::new(LocalDbFault::new("committing the judgement", e)).context(format!(
            "commit outcome unknown for event {} — check whether it was recorded before \
             retrying, or the judgement may be recorded twice",
            asserted.event_id
        ))
    })?;
    Ok(LinkOutcome {
        event_id: asserted.event_id,
        proposal_resolved: asserted.proposal_resolved,
        filed_under: about,
        record_of,
        charts,
        effect,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_judgement_says_what_it_did_not_just_that_it_was_recorded() {
        use LinkEffect::*;
        // Outranked whenever the standing assertion says the opposite — whatever the rest of
        // the record reads.
        for verb in [LinkVerb::Link, LinkVerb::Unlink] {
            for joined in [false, true] {
                assert_eq!(link_effect(verb, false, joined), Outranked, "{verb:?}");
            }
        }
        assert_eq!(link_effect(LinkVerb::Link, true, true), TookEffect);
        assert_eq!(link_effect(LinkVerb::Unlink, true, false), TookEffect);
        assert_eq!(link_effect(LinkVerb::Unlink, true, true), StillJoined);
    }
}
