//! The public entry points of a human's link/unlink judgement — [`link_charts`] and
//! [`unlink_charts`], and their "as shown" variants for a judgement made from a comparison — and
//! what they report back ([`LinkOutcome`], [`LinkEffect`]).
//!
//! Lives apart from `chart_link.rs` (house rule 4: files under 500 lines); every item is
//! re-exported by the parent, so `chart_link::unlink_charts` and friends keep their paths. The
//! flow: `judge` makes the legible pre-checks (is each chart held? may the judgement be made,
//! and filed under which chart — the pure rule in `admit.rs`?), then opens ONE transaction that
//! signs and submits through the parent's [`assert_link_in_tx`] (which re-checks a third-chart
//! filing under the link lock (CARNLK)) and reads back what the judgement did, before committing.

use super::{
    admit_judgement, assert_link_in_tx, canonical_pair, compose_review_provenance,
    record_holds_both, AdmitRefusal, LinkVerb, OpenedChart, RecordsShown, Reviewer,
};
use crate::db_diagnosis::{
    deliberate_refusal, node_state_refusal, stale_picture_refusal, LocalDbFault, RefusalScope,
};
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
    /// direct edge still stands as that other judgement says. Judging again is NOT a
    /// no-op: the clinical sync door (db/020, the only door an identity event arrives
    /// through) merges the peer's HLC into this node's clock, so the next local judgement is
    /// stamped above the peer's and would NORMALLY outrank it — overruling the colleague
    /// without settling anything. "Normally", because that door clamps its merge at
    /// `cairn_max_hlc_drift_ms()` (24 h) of drift: a peer further ahead than that is still
    /// admitted with its full wall clock and keeps outranking a local retry until this node's
    /// own clock reaches the peer's. It is a disagreement between humans for a human to settle.
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
        (LinkVerb::Unlink, true, false) => LinkEffect::TookEffect,
        (LinkVerb::Link, true, true) => LinkEffect::TookEffect,
        // A standing link that does not join its own two charts contradicts db/018's component
        // recompute (both run under CARNLK, in this transaction). Named rather than hidden in a
        // wildcard; the link's own assertion stands, which is all this judgement can say.
        (LinkVerb::Link, true, false) => LinkEffect::TookEffect,
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
    /// transaction, after its event applied and under the link lock (CARNLK): what that chart
    /// reads as, now that the judgement is recorded — the set THIS judgement left (#797).
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
    judge(
        client,
        LinkVerb::Link,
        a,
        b,
        None,
        None,
        reviewer,
        node_origin,
    )
    .await
}

/// "Same person", judged from a COMPARISON: as [`link_charts`], but signed only while each
/// chart's record still reads as `shown` — the two records the clinician compared (#708). A
/// record that changed since (a peer's link or unlink landed) refuses the judgement as stale
/// input, before anything is signed; see [`RecordsShown`].
pub async fn link_charts_as_shown(
    client: &mut tokio_postgres::Client,
    a: Uuid,
    b: Uuid,
    shown: RecordsShown<'_>,
    reviewer: &Reviewer<'_>,
    node_origin: &str,
) -> anyhow::Result<LinkOutcome> {
    judge(
        client,
        LinkVerb::Link,
        a,
        b,
        None,
        Some(shown),
        reviewer,
        node_origin,
    )
    .await
}

/// "Same person — confirm" a link INSIDE one record (R6, #716): a link that already stands
/// without a clinician's confirmation, or — the bridge — two charts a clinician once recorded as
/// different people while other links still join them. As [`link_charts`], but signed only while
/// the record still reads as `record` AND the pair's standing assertion is still `standing`, the
/// one the panel showed (see [`RecordsShown::pair`]). Both charts must be held here.
///
/// Why the pair, not just the record: inside one record a third chart can keep A and B joined, so
/// a colleague's attested unlink of A–B landing meanwhile leaves `record` unchanged — and our
/// confirm, stamped later by this node's clock, would silently overrule it. The pair's content
/// address is what changes, so that is what is re-checked under CARNLK before anything is signed.
pub async fn confirm_link_as_shown(
    client: &mut tokio_postgres::Client,
    a: Uuid,
    b: Uuid,
    record: &ChartSet,
    standing: &[u8],
    reviewer: &Reviewer<'_>,
    node_origin: &str,
) -> anyhow::Result<LinkOutcome> {
    let shown = RecordsShown {
        this: record,
        other: record,
        pair: Some(standing),
    };
    judge(
        client,
        LinkVerb::Link,
        a,
        b,
        None,
        Some(shown),
        reviewer,
        node_origin,
    )
    .await
}

/// "Not the same person" about two SEPARATE records the clinician compared ("Different people"
/// on a possible duplicate): as [`unlink_charts`] with no `opened` chart, but signed only while
/// each chart's record still reads as `shown` (#708). A colleague's link of the pair landing
/// mid-judgement is refused here rather than silently overruled; see [`RecordsShown`].
pub async fn unlink_charts_as_shown(
    client: &mut tokio_postgres::Client,
    a: Uuid,
    b: Uuid,
    shown: RecordsShown<'_>,
    reviewer: &Reviewer<'_>,
    node_origin: &str,
) -> anyhow::Result<LinkOutcome> {
    judge(
        client,
        LinkVerb::Unlink,
        a,
        b,
        None,
        Some(shown),
        reviewer,
        node_origin,
    )
    .await
}

/// "Not the same person — unlink" a link INSIDE one record, from the review panel (R6, #716): as
/// [`unlink_charts`] judged from `opened` (the chart on screen), but signed only while the record
/// still reads as `record` AND the pair's standing assertion is still `standing`, the one the
/// panel showed (see [`RecordsShown::pair`]).
///
/// Why the pair, not just the record — the mirror of [`confirm_link_as_shown`]: inside one record
/// a third chart can keep A and B joined, so a colleague's attested LINK of A–B (their confirm)
/// landing while our clinician reads leaves `record` unchanged. Our unlink is attested too and,
/// stamped later by this node's clock, would win the attested-vs-attested tie-break — silently
/// overruling the colleague's confirm and reporting an ordinary `StillJoined`. The pair's content
/// address is what changes, so it is re-checked under CARNLK (row lock first, then CARNLK, as
/// every judgement) before anything is signed; a changed pair is refused as stale input ("review
/// it again"). The CLI and every other caller of [`unlink_charts`] judge without a panel and keep
/// that function's behaviour.
#[allow(clippy::too_many_arguments)]
pub async fn unlink_charts_as_shown_from(
    client: &mut tokio_postgres::Client,
    a: Uuid,
    b: Uuid,
    opened: Uuid,
    record: &ChartSet,
    standing: &[u8],
    reviewer: &Reviewer<'_>,
    node_origin: &str,
) -> anyhow::Result<LinkOutcome> {
    let shown = RecordsShown {
        this: record,
        other: record,
        pair: Some(standing),
    };
    judge(
        client,
        LinkVerb::Unlink,
        a,
        b,
        Some(opened),
        Some(shown),
        reviewer,
        node_origin,
    )
    .await
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
        None,
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

/// Mark an [`admit_judgement`] refusal with the scope it names, so a surface offers the right
/// way forward: wait for this node (`NodeState`), re-read (`Stale`: the record changed since it
/// was read), or nothing at all (`Input`: no retry, by anyone, changes it).
fn refusal_error(refusal: AdmitRefusal) -> anyhow::Error {
    match refusal.scope {
        RefusalScope::NodeState => node_state_refusal(refusal.text),
        RefusalScope::Input => deliberate_refusal(refusal.text),
        RefusalScope::Stale => stale_picture_refusal(refusal.text),
    }
}

/// The shared body of every entry point above: pre-checks (legible refusals before anything is
/// signed), then ONE transaction that writes the judgement AND reads back what it did — so
/// once the commit succeeds there is nothing left that can fail. `opened`: the chart the
/// judgement is made from, if the caller named one (see [`unlink_charts`]). `shown`: the
/// records the judgement was made over, if the caller compared them, and — for a verdict inside
/// one record — the pair's standing assertion as the panel read it (see [`RecordsShown`]).
#[allow(clippy::too_many_arguments)]
async fn judge(
    client: &mut tokio_postgres::Client,
    verb: LinkVerb,
    a: Uuid,
    b: Uuid,
    opened: Option<Uuid>,
    shown: Option<RecordsShown<'_>>,
    reviewer: &Reviewer<'_>,
    node_origin: &str,
) -> anyhow::Result<LinkOutcome> {
    if a == b {
        // A verdict about the INPUT: no retry, by anyone, ever changes it.
        return Err(deliberate_refusal(format!(
            "{a} and {b} are the same chart — a chart cannot be linked to itself"
        )));
    }
    if shown.and_then(|s| s.pair).is_some_and(<[u8]>::is_empty) {
        // Also about the INPUT: a panel that read no standing judgement has nothing to sign
        // over. Without this the pair check would refuse it anyway (no real address is empty),
        // but as "changed while you were judging" — untrue; the window's `require_standing`
        // says the same sentence, so a non-window caller gets the true reason too.
        return Err(deliberate_refusal(
            "the comparison did not read which judgement stands on this pair — nothing was \
             done; review it again",
        ));
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
    // AGAIN by `assert_link_in_tx`, under the link lock (CARNLK), before anything is signed.
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
    // A verdict — about this node's state (a chart not held here: sync may deliver it) or about
    // the picture judged from (an open chart whose record lacks the pair: re-read it). Marked
    // either way, so a surface words it as a verdict and never as an outage to retry (#702).
    let filed = admit_judgement(verb, (a, a_held), (b, b_held), shared_record, opened_chart)
        .map_err(refusal_error)?;
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
    // A third-chart filing's record (the pre-check above) is re-read by `assert_link_in_tx`
    // under the link lock (CARNLK), before anything is signed — see the check there.
    let asserted = assert_link_in_tx(
        &tx,
        verb,
        low,
        high,
        filed,
        shown,
        &provenance,
        None,
        reviewer,
        hlc,
    )
    .await?;
    // The record the clinician judged FROM (the open chart), else the filed-under chart —
    // what the caller shows next. Read BEFORE the commit, in the same transaction, so a
    // failure here rolls the judgement back rather than leaving a committed event behind an
    // error. And read AFTER the event applied, while this transaction still holds db/018's link
    // lock (CARNLK, taken by the event's own apply): no other link or unlink can land between
    // the apply and this read, so this is exactly the set THIS judgement left. The window relies
    // on that (#797): it re-reads against this set, so the clinician's own change is never
    // announced back to them while a colleague's, committed after, still is. A read after the
    // commit could already include the colleague's change and silence it. The order is pinned by
    // `judge_tests.rs`'s `the_set_left_is_read_after_the_apply_and_before_the_commit`.
    let record_of = opened.unwrap_or(about);
    let charts = crate::patient::person::person_charts(&tx, record_of)
        .await
        .context("reading the chart set the judgement leaves")?;
    // "Still joined?" is a question about the two SUBJECTS — do they still read as one
    // record? — asked of the subjects themselves, never of the filed-under or open chart: once
    // an unlink may be filed under a THIRD chart (#699 (a)), "is a subject in that chart's
    // record" is true after a successful split whenever the subject asked about is the near one
    // (the far link of A–B–C, judged from A, leaves B in A's record), so the answer would turn
    // on argument order. Read in this transaction, like `charts`.
    let joined = crate::patient::person::person_charts(&tx, low)
        .await
        .context("reading whether the two charts still read as one record")?
        .contains(&high);
    let effect = link_effect(verb, asserted.agrees, joined);

    // The one failure that may have written: a connection lost DURING the commit leaves
    // its outcome unknown. Name the event so the operator looks before retrying.
    tx.commit().await.map_err(|e| {
        // A server that ANSWERED the COMMIT with an error (it carries a SQLSTATE) rolled the
        // transaction back: nothing was written, and that is an ordinary failure. Only a commit
        // the server never answered — the connection lost mid-flight — may have landed, and is
        // marked (#713), so a surface re-reads before offering this judgement again rather than
        // treating it as an outage: a commit that DID land as `Outranked` leaves the record
        // reading as before, and a second press would overrule the colleague it lost to.
        let answered = e.as_db_error().is_some();
        let fault = anyhow::Error::new(LocalDbFault::new("committing the judgement", e));
        if answered {
            fault
        } else {
            crate::db_diagnosis::commit_outcome_unknown(asserted.event_id, fault)
        }
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
#[path = "judge_tests.rs"]
mod tests;
