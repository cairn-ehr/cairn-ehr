//! Link or unlink two charts as a HUMAN judgement (ADR-0076 decisions 4 and 5; repair path
//! R2a, for #681's gesture).
//!
//! Two paper folders the clinician has laid side by side are either clipped together
//! ("same person" → an attested `identity.link.asserted`) or marked as two people ("not
//! the same person" → an attested `identity.unlink.asserted`). Both are events in the
//! closed §5.7 identity algebra; both are reversible by the other; neither erases anything.
//! "Different people" is an unlink on a pair that may never have been linked (decision 4).
//!
//! WHY ATTESTED: db/018 ranks an attested assertion above an un-attested one (decision 5),
//! so on an upgraded node no machine assertion about the same pair can undo this judgement.
//! And an attested link is exactly the human decision a hard veto (§5.13) forces: the veto
//! refuses only an UN-attested link at the local door.
//!
//! Split, per house rule 4: pure body assembly (unit-tested here), one in-transaction core
//! (`assert_link_in_tx`) that `apply_proposal::apply_accepted_proposal` also uses, and the
//! two public entry points that add the pre-checks and the transaction.
//!
//! Identity events are CLEAR (db/005 refuses a sealed non-clinical body), so this signs and
//! attests directly and submits through the 3-argument `submit_event` door — never the
//! medication seal path.

use crate::db_diagnosis::{deliberate_refusal, node_state_refusal, LocalDbFault};
use anyhow::Context;
use cairn_event::identity::{
    link_assertion_body, render_link_twin, render_unlink_twin, unlink_assertion_body, LinkAssertion,
};
use cairn_event::{event_address, sign, sign_attestation, EventBody, Hlc, SigningKey};
use cairn_medication_view::ChartSet;
use uuid::Uuid;

/// Which judgement the human made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkVerb {
    /// "Same person" — clip the folders together.
    Link,
    /// "Not the same person" — whether or not they were ever linked.
    Unlink,
}

impl LinkVerb {
    /// The registered event type (db/018).
    pub fn event_type(self) -> &'static str {
        match self {
            LinkVerb::Link => "identity.link.asserted",
            LinkVerb::Unlink => "identity.unlink.asserted",
        }
    }

    /// The body's schema version (the convention the C1 tests established).
    pub fn schema_version(self) -> &'static str {
        match self {
            LinkVerb::Link => "identity.link/1",
            LinkVerb::Unlink => "identity.unlink/1",
        }
    }

    /// The `patient_link.state` this judgement asserts (db/018).
    pub fn state(self) -> &'static str {
        match self {
            LinkVerb::Link => "link",
            LinkVerb::Unlink => "unlink",
        }
    }

    /// The `match_proposal.status` an OPEN proposal for the pair moves to when a human
    /// decides it this way.
    pub fn resolved_status(self) -> &'static str {
        match self {
            LinkVerb::Link => "applied",
            LinkVerb::Unlink => "rejected",
        }
    }
}

/// `(least, greatest)` — the order `patient_link` and `match_proposal` store a pair in, so
/// a caller naming the charts either way round finds the same rows.
pub fn canonical_pair(a: Uuid, b: Uuid) -> (Uuid, Uuid) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

/// The §4.1 provenance of a judgement made while reviewing a chart. Non-empty by
/// construction (the db/018 floor requires it); names the act and the human.
pub fn compose_review_provenance(verb: LinkVerb, human_kid: &str) -> String {
    match verb {
        LinkVerb::Link => format!("chart-review linked-by:{human_kid}"),
        LinkVerb::Unlink => format!("chart-review unlinked-by:{human_kid}"),
    }
}

/// The attested link or unlink body. Pure: the caller supplies `event_id` and `hlc`.
/// The human is the sole contributor and carries `responsibility` — which is what makes
/// both write doors demand a verified human attestation token for this event.
///
/// `about` is the chart the event's ENVELOPE (`patient_id`) is filed under, and must be
/// `low` or `high` — [`assert_link_in_tx`] refuses anything else before calling this (the
/// db/018 floor does not check it, and a wrong `about` would file an identity event in an
/// unrelated patient's stream). The C1 convention is `low`; an unlink naming a chart not held here
/// (see [`admit_judgement`]) is filed under the HELD chart instead, because db/005 step 8b refuses a local event
/// about a chart with no history on this node. The payload's subjects stay canonical
/// either way — db/018 reads the pair from them, never from the envelope.
#[allow(clippy::too_many_arguments)]
pub fn build_attested_assertion_body(
    verb: LinkVerb,
    event_id: Uuid,
    low: Uuid,
    high: Uuid,
    about: Uuid,
    provenance: &str,
    confidence: Option<&str>,
    human_kid: &str,
    hlc: Hlc,
) -> EventBody {
    let low_s = low.to_string();
    let high_s = high.to_string();
    let la = LinkAssertion {
        subject_a: &low_s,
        subject_b: &high_s,
        provenance,
        confidence,
    };
    let (payload, twin) = match verb {
        LinkVerb::Link => (link_assertion_body(&la), render_link_twin(&la)),
        LinkVerb::Unlink => (unlink_assertion_body(&la), render_unlink_twin(&la)),
    };
    EventBody {
        event_id: event_id.to_string(),
        patient_id: about.to_string(),
        event_type: verb.event_type().into(),
        schema_version: verb.schema_version().into(),
        hlc,
        t_effective: None,
        signer_key_id: human_kid.into(),
        // ADR-0051 wire shape: responsibility = {held_by}, held_by = the verified attester
        // (the #195 binding chain).
        contributors: serde_json::json!([
            {"actor_id": human_kid, "role": "attested",
             "responsibility": {"held_by": human_kid}}
        ]),
        payload,
        attachments: vec![],
        plaintext_twin: Some(twin),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    }
}

/// The human making the judgement: the unlocked signing key and its key id. A judgement
/// about who a person is belongs to a human (ADR-0053) — there is no node-key fallback.
pub struct Reviewer<'a> {
    pub human_sk: &'a SigningKey,
    pub human_kid: &'a str,
}

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
/// (see [`Asserted::agrees`]). `other_in_record`: the other chart reads as part of the
/// filed-under chart's record.
pub fn link_effect(verb: LinkVerb, agrees: bool, other_in_record: bool) -> LinkEffect {
    match (verb, agrees, other_in_record) {
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
    /// [`admit_judgement`]), whichever order the caller named the two charts in.
    pub filed_under: Uuid,
    /// The chart set of [`LinkOutcome::filed_under`], read inside the judgement's own
    /// transaction: what that chart reads as, now that the judgement is recorded.
    pub charts: ChartSet,
    /// What the judgement did to the record — see [`LinkEffect`].
    pub effect: LinkEffect,
}

/// What [`assert_link_in_tx`] wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Asserted {
    /// The attested identity event.
    pub event_id: Uuid,
    /// Whether an OPEN `match_proposal` for the pair moved.
    pub proposal_resolved: bool,
    /// Whether the pair's standing `patient_link` assertion says what this judgement says —
    /// its own event, or a later one that agrees. db/018 admits an assertion that loses the
    /// overlay, so a successful submit does not imply this. (Deliberately not "is it OUR
    /// event": a judgement a later AGREEING one outranks has nothing left to do, and must
    /// not be reported as a disagreement.)
    pub agrees: bool,
}

/// The pair's STANDING `patient_link` row — the assertion that currently wins the overlay
/// (db/018) — as far as a caller deciding "did my event take effect?" needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StandingLink {
    /// The winning event's content address.
    pub content_address: Vec<u8>,
    /// `link` or `unlink`.
    pub state: String,
    /// Whether the winner is a human's vouched judgement (db/018's one definition, stored).
    pub attested: bool,
}

impl StandingLink {
    /// Is the event with this content address the one standing? The check both judgement
    /// paths make right after submitting: db/018 ADMITS an assertion that loses the
    /// overlay (it is a real event, and replicates), so "submitted" is not "took effect".
    pub fn is(&self, content_address: &[u8]) -> bool {
        self.content_address == content_address
    }
}

/// Read the standing winner for the canonical pair, inside the caller's transaction (so it
/// sees the caller's own just-submitted event). `None`: no assertion about the pair exists.
pub async fn standing_link(
    client: &(impl tokio_postgres::GenericClient + Sync),
    low: Uuid,
    high: Uuid,
) -> anyhow::Result<Option<StandingLink>> {
    let row = client
        .query_opt(
            "SELECT content_address, state, attested FROM patient_link \
             WHERE low = $1::text::uuid AND high = $2::text::uuid",
            &[&low.to_string(), &high.to_string()],
        )
        .await
        .map_err(|e| LocalDbFault::new("reading the pair's standing link", e))?;
    Ok(row.map(|r| StandingLink {
        content_address: r.get(0),
        state: r.get(1),
        attested: r.get(2),
    }))
}

/// The proposal statuses a human judgement resolves. Every other status is CLOSED and is
/// left exactly as it is: `applied`/`auto_applied` carry an `applied_event_id` that db/019's
/// invariant ties to them, `rejected` is already decided, `retracted` was withdrawn by the
/// matcher. What stands is `patient_link`'s business, not the proposal row's; the row only
/// records how an open proposal was first answered.
const OPEN_PROPOSAL_STATUSES: [&str; 3] = ["pending", "accepted", "review"];

/// Sign, attest and submit one judgement inside the caller's transaction, then move an
/// OPEN proposal for the pair and read back whether the event stands. See [`Asserted`].
///
/// Shared with `apply_proposal::apply_accepted_proposal`, so the matcher-proposal path
/// and the chart-review path cannot drift. `low`/`high` must be canonical (`low < high`)
/// and `about` one of them (see [`build_attested_assertion_body`]); both are checked here,
/// before anything is locked or signed, because a reversed pair would lock and move no
/// proposal row and a wrong `about` would misfile the event — and neither is visible to
/// the database floor.
///
/// Locks the pair's `match_proposal` row (if any) FIRST, before signing or submitting
/// anything — see the inline comment at the top of the body for why: it keeps this
/// function's lock order (row, then db/018's CARNLK advisory lock) the same as every
/// other path that touches both, so two judgements on the same pair cannot deadlock.
///
/// Errors roll the caller's transaction back when it drops: nothing is written and the
/// proposal does not move (the db/005 gate refuses a non-human attester, db/018 a
/// self-link or empty provenance).
#[allow(clippy::too_many_arguments)]
pub async fn assert_link_in_tx(
    tx: &tokio_postgres::Transaction<'_>,
    verb: LinkVerb,
    low: Uuid,
    high: Uuid,
    about: Uuid,
    provenance: &str,
    confidence: Option<&str>,
    reviewer: &Reviewer<'_>,
    hlc: Hlc,
) -> anyhow::Result<Asserted> {
    anyhow::ensure!(
        low < high,
        "a judgement's pair must be canonical and distinct: ({low}, {high})"
    );
    anyhow::ensure!(
        about == low || about == high,
        "a judgement about ({low}, {high}) cannot be filed under chart {about}"
    );
    // LOCK ORDER: db/018's `patient_link_apply` trigger —
    // run inside `submit_event` below — takes the GLOBAL advisory lock
    // `pg_advisory_xact_lock(x'4341524E4C4B')` ('CARNLK') and holds it until this
    // transaction commits or rolls back. Both `auto_apply.rs::apply_auto_candidate`
    // and `apply_proposal.rs::apply_accepted_proposal` lock the pair's
    // `match_proposal` row `FOR UPDATE` FIRST and only submit (taking CARNLK) SECOND. If
    // this function instead submitted first and updated `match_proposal` second, two
    // transactions judging the same pair could deadlock: one holding CARNLK and wanting
    // the row, the other holding the row and wanting CARNLK — Postgres detects the cycle
    // and aborts one with 40P01, which a clinician would see as a random failure.
    //
    // Locking the row here FIRST — even though the actual UPDATE happens later, below —
    // establishes ONE order for every path: row, then CARNLK. `query_opt`: a pair with no
    // `match_proposal` row at all has nothing to lock — a genuine no-op, with nothing to
    // serialize against. A CLOSED row is locked too, harmlessly: the status-filtered UPDATE
    // below leaves it unchanged. When the caller (e.g.
    // `apply_accepted_proposal`) already holds the row FOR UPDATE from its own earlier
    // read, re-locking it here in the SAME transaction is a harmless no-op re-acquire.
    tx.query_opt(
        "SELECT 1 FROM match_proposal \
         WHERE patient_low = $1::text::uuid AND patient_high = $2::text::uuid FOR UPDATE",
        &[&low.to_string(), &high.to_string()],
    )
    .await
    .map_err(|e| LocalDbFault::new("locking the pair's match proposal", e))?;

    let event_id = Uuid::now_v7();
    let body = build_attested_assertion_body(
        verb,
        event_id,
        low,
        high,
        about,
        provenance,
        confidence,
        reviewer.human_kid,
        hlc,
    );
    // The human both authors (signs) and vouches (attests) — the token is what makes the
    // event "attested" in db/018's one definition, and so what ranks it (decision 5).
    let signed = sign(&body, reviewer.human_sk)?;
    let ca = event_address(&signed.signed_bytes);
    let token = sign_attestation(&ca, reviewer.human_kid, "attested", reviewer.human_sk)?;
    let attester_vk = reviewer.human_sk.verifying_key().to_bytes().to_vec();
    tx.execute(
        "SELECT submit_event($1,$2,$3)",
        &[&signed.signed_bytes, &token, &attester_vk],
    )
    .await
    .map_err(|e| LocalDbFault::new("submitting the judgement through the floor", e))?;

    // Move an OPEN proposal for the pair. A link records its event (db/019: applied ⇔
    // applied_event_id set); a rejection records none.
    let applied_event: Option<String> = match verb {
        LinkVerb::Link => Some(event_id.to_string()),
        LinkVerb::Unlink => None,
    };
    let open: Vec<String> = OPEN_PROPOSAL_STATUSES
        .iter()
        .map(|s| s.to_string())
        .collect();
    let moved = tx
        .execute(
            "UPDATE match_proposal \
                SET status = $3, applied_event_id = $4::text::uuid, updated_at = clock_timestamp() \
              WHERE patient_low = $1::text::uuid AND patient_high = $2::text::uuid \
                AND status = ANY($5::text[])",
            &[
                &low.to_string(),
                &high.to_string(),
                &verb.resolved_status(),
                &applied_event,
                &open,
            ],
        )
        .await
        .map_err(|e| LocalDbFault::new("resolving the pair's open match proposal", e))?;

    // Does the record now say what the human said? Read inside this transaction, so it sees
    // our own event.
    let agrees = standing_link(tx, low, high)
        .await?
        .is_some_and(|w| w.state == verb.state());
    Ok(Asserted {
        event_id,
        proposal_resolved: moved > 0,
        agrees,
    })
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
    judge(client, LinkVerb::Link, a, b, reviewer, node_origin).await
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
/// Errors: as [`link_charts`] — only "commit outcome unknown for event …" may have written.
pub async fn unlink_charts(
    client: &mut tokio_postgres::Client,
    a: Uuid,
    b: Uuid,
    reviewer: &Reviewer<'_>,
    node_origin: &str,
) -> anyhow::Result<LinkOutcome> {
    judge(client, LinkVerb::Unlink, a, b, reviewer, node_origin).await
}

/// Whether a judgement may be made on this pair, given what this node holds — and if so,
/// which chart its event is filed under. **Pure**, so the rule is unit-testable apart from
/// the database.
///
/// - LINK needs BOTH charts held (a `patient_chart` row — see
///   `patient::person::ChartIdentity::held`). The floor admits a link naming a chart that
///   has not synced yet, correctly (offline-first); a human's deliberate act from this node
///   has no such excuse, and a typo would otherwise attach a stranger's future chart to
///   this person.
/// - UNLINK attaches nothing, so that risk does not apply. It needs one chart held (the
///   one the clinician has open) and the other either held too or already part of that
///   chart's record here (`shared_record`) — the member line R1 displays for a chart whose
///   registration has not reached this node. A never-linked stranger is still refused.
///
/// `Ok(about)`: the chart to file the event under — `low` by the C1 convention when both are
/// held, else the one held chart (db/005 step 8b refuses a local event about a chart with
/// no history here). `Err(text)`: the refusal, naming the chart(s) at fault.
pub fn admit_judgement(
    verb: LinkVerb,
    (a, a_held): (Uuid, bool),
    (b, b_held): (Uuid, bool),
    shared_record: bool,
) -> Result<Uuid, String> {
    let (low, _) = canonical_pair(a, b);
    let rule = "a link needs both charts held on this node; an unlink needs the chart you \
                have open held here, and the other held too or already read as part of its \
                record";
    match (verb, a_held, b_held) {
        (_, true, true) => Ok(low),
        (_, false, false) => Err(format!(
            "neither chart {a} nor chart {b} is held on this node — {rule}"
        )),
        (LinkVerb::Unlink, true, false) if shared_record => Ok(a),
        (LinkVerb::Unlink, false, true) if shared_record => Ok(b),
        // Exactly one chart unheld, and not admitted above. Say only what is true: for an
        // unlink that means it is also outside the other's record; for a link, whether it
        // is inside is beside the point.
        (_, a_held, _) => {
            let (unheld, other) = if a_held { (b, a) } else { (a, b) };
            let outside = match verb {
                LinkVerb::Unlink => format!(" and is not part of chart {other}'s record here"),
                LinkVerb::Link => String::new(),
            };
            Err(format!(
                "chart {unheld} is not held on this node{outside} — {rule}"
            ))
        }
    }
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
/// once the commit succeeds there is nothing left that can fail.
async fn judge(
    client: &mut tokio_postgres::Client,
    verb: LinkVerb,
    a: Uuid,
    b: Uuid,
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
    // A verdict about this NODE's state: the identical call succeeds once the chart (or the
    // record joining them) has arrived here. Marked, so a surface words it as a verdict and
    // never as an outage to retry (#702).
    let about = admit_judgement(verb, (a, a_held), (b, b_held), shared_record)
        .map_err(node_state_refusal)?;
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
    let asserted = assert_link_in_tx(
        &tx,
        verb,
        low,
        high,
        about,
        &provenance,
        None,
        reviewer,
        hlc,
    )
    .await?;
    // What the filed-under chart (always one held here) now reads as — read BEFORE the
    // commit, in the same transaction, so a failure here rolls the judgement back rather
    // than leaving a committed event behind an error.
    let charts = crate::patient::person::person_charts(&tx, about)
        .await
        .context("reading the chart set the judgement leaves")?;
    let other = if about == a { b } else { a };
    let effect = link_effect(verb, asserted.agrees, charts.contains(&other));

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
        charts,
        effect,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hlc() -> Hlc {
        Hlc {
            wall: 7,
            counter: 0,
            node_origin: "n".into(),
        }
    }

    fn pair() -> (Uuid, Uuid) {
        let lo = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let hi = Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
        (lo, hi)
    }

    #[test]
    fn canonical_pair_orders_either_way_round() {
        let (lo, hi) = pair();
        assert_eq!(canonical_pair(hi, lo), (lo, hi));
        assert_eq!(canonical_pair(lo, hi), (lo, hi));
    }

    #[test]
    fn each_verb_names_its_type_schema_and_resolution() {
        assert_eq!(LinkVerb::Link.event_type(), "identity.link.asserted");
        assert_eq!(LinkVerb::Unlink.event_type(), "identity.unlink.asserted");
        assert_eq!(LinkVerb::Link.schema_version(), "identity.link/1");
        assert_eq!(LinkVerb::Unlink.schema_version(), "identity.unlink/1");
        assert_eq!(LinkVerb::Link.state(), "link");
        assert_eq!(LinkVerb::Unlink.state(), "unlink");
        assert_eq!(LinkVerb::Link.resolved_status(), "applied");
        assert_eq!(LinkVerb::Unlink.resolved_status(), "rejected");
    }

    #[test]
    fn review_provenance_names_the_act_and_the_human() {
        let l = compose_review_provenance(LinkVerb::Link, "kidH");
        let u = compose_review_provenance(LinkVerb::Unlink, "kidH");
        assert!(l.contains("linked-by:kidH") && !l.contains("unlinked"));
        assert!(u.contains("unlinked-by:kidH"));
    }

    #[test]
    fn the_body_carries_the_responsibility_that_demands_a_human_token() {
        let (lo, hi) = pair();
        for verb in [LinkVerb::Link, LinkVerb::Unlink] {
            let eid = Uuid::now_v7();
            let b =
                build_attested_assertion_body(verb, eid, lo, hi, lo, "prov", None, "kidH", hlc());
            assert_eq!(b.event_type, verb.event_type());
            assert_eq!(b.schema_version, verb.schema_version());
            assert_eq!(b.event_id, eid.to_string());
            assert_eq!(
                b.patient_id,
                lo.to_string(),
                "an identity event is about subject_a = low"
            );
            assert_eq!(b.payload["subject_a"], lo.to_string());
            assert_eq!(b.payload["subject_b"], hi.to_string());
            assert_eq!(b.payload["provenance"], "prov");
            assert!(
                b.payload.get("confidence").is_none(),
                "absent, never null (principle 4)"
            );
            assert_eq!(b.contributors[0]["responsibility"]["held_by"], "kidH");
            assert!(!b.plaintext_twin.as_deref().unwrap().trim().is_empty());
            // Filed under the HIGH chart: only the envelope moves; the pair stays canonical.
            let h =
                build_attested_assertion_body(verb, eid, lo, hi, hi, "prov", None, "kidH", hlc());
            assert_eq!(h.patient_id, hi.to_string());
            assert_eq!(h.payload["subject_a"], lo.to_string());
        }
    }

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

    #[test]
    fn a_standing_link_knows_its_own_event() {
        let w = StandingLink {
            content_address: vec![1, 2, 3],
            state: "unlink".into(),
            attested: true,
        };
        assert!(w.is(&[1, 2, 3]));
        assert!(!w.is(&[1, 2, 4]));
    }

    #[test]
    fn a_link_needs_both_charts_held() {
        let (lo, hi) = pair();
        assert_eq!(
            admit_judgement(LinkVerb::Link, (hi, true), (lo, true), false),
            Ok(lo)
        );
        // Even a chart already in the record: a link must not reach past this node.
        let refusal = admit_judgement(LinkVerb::Link, (lo, true), (hi, false), true).unwrap_err();
        assert!(refusal.contains(&hi.to_string()), "{refusal}");
        assert!(
            !refusal.contains("not part of"),
            "true of this chart, so unsaid: {refusal}"
        );
    }

    #[test]
    fn an_unlink_may_name_a_displayed_member_not_held_here_but_not_a_stranger() {
        let (lo, hi) = pair();
        // Filed under whichever chart IS held, in either argument position.
        assert_eq!(
            admit_judgement(LinkVerb::Unlink, (hi, true), (lo, false), true),
            Ok(hi)
        );
        assert_eq!(
            admit_judgement(LinkVerb::Unlink, (lo, false), (hi, true), true),
            Ok(hi)
        );
        let refusal =
            admit_judgement(LinkVerb::Unlink, (hi, true), (lo, false), false).unwrap_err();
        assert!(
            refusal.contains(&lo.to_string()),
            "names the stranger: {refusal}"
        );
        let refusal =
            admit_judgement(LinkVerb::Unlink, (lo, false), (hi, false), true).unwrap_err();
        assert!(refusal.contains(&lo.to_string()) && refusal.contains(&hi.to_string()));
    }
}
