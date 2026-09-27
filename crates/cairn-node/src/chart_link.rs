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
//! so a machine's later link can never undo this judgement, and a hard veto (§5.13) — which
//! refuses an un-attested link at the local door — is exactly the human decision it forces.
//!
//! Split, per house rule 4: pure body assembly (unit-tested here), one in-transaction core
//! (`assert_link_in_tx`) that `apply_proposal::apply_accepted_proposal` also uses, and the
//! two public entry points that add the pre-checks and the transaction.
//!
//! Identity events are CLEAR (db/005 refuses a sealed non-clinical body), so this signs and
//! attests directly and submits through the 3-argument `submit_event` door — never the
//! medication seal path.

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

    /// The body's schema version (the convention the C1 tests and `apply_proposal` use).
    pub fn schema_version(self) -> &'static str {
        match self {
            LinkVerb::Link => "identity.link/1",
            LinkVerb::Unlink => "identity.unlink/1",
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
/// `low` or `high`. The C1 convention is `low`; an unlink naming a chart not held here
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
    debug_assert!(
        about == low || about == high,
        "the envelope names one of the pair"
    );
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

/// What a judgement wrote, and what the chart now is.
#[derive(Debug)]
pub struct LinkOutcome {
    /// The attested identity event.
    pub event_id: Uuid,
    /// Whether an OPEN `match_proposal` for the pair moved (`applied` / `rejected`).
    pub proposal_resolved: bool,
    /// The chart set of the first chart named, read after commit — what a window reopens.
    pub charts: ChartSet,
    /// An UNLINK was recorded, but the second chart still reads as part of the first's
    /// record through ANOTHER link (A–C–B: unlinking A from B leaves A–C and C–B
    /// standing). The judgement is real and replicates (ADR-0076 decision 4) — it just
    /// cannot split the record on its own. The remedy is to unlink that other link too;
    /// the machine never picks which edge is wrong (principle 2), so this is reported,
    /// never auto-resolved. Always `false` for a link.
    pub still_joined: bool,
}

/// The proposal statuses a human judgement resolves. Every other status is CLOSED and is
/// left exactly as it is: `applied`/`auto_applied` carry an `applied_event_id` that db/019's
/// invariant ties to them, `rejected` is already decided, `retracted` was withdrawn by the
/// matcher. What stands is `patient_link`'s business, not the proposal row's; the row only
/// records how an open proposal was first answered.
const OPEN_PROPOSAL_STATUSES: [&str; 3] = ["pending", "accepted", "review"];

/// Sign, attest and submit one judgement inside the caller's transaction, then move an
/// OPEN proposal for the pair. Returns (event id, whether a proposal moved).
///
/// Shared with `apply_proposal::apply_accepted_proposal`, so the matcher-proposal path
/// and the chart-review path cannot drift. `low`/`high` must already be canonical;
/// `about` is the envelope's chart (see [`build_attested_assertion_body`]).
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
) -> anyhow::Result<(Uuid, bool)> {
    // LOCK ORDER (controller ruling, R2a Task 4): db/018's `patient_link_apply` trigger —
    // run inside `submit_event` below — takes the GLOBAL advisory lock
    // `pg_advisory_xact_lock(x'4341524E4C4B')` ('CARNLK') and holds it until this
    // transaction commits or rolls back. Both `auto_apply.rs::apply_auto_candidate`
    // (~:122) and `apply_proposal.rs::apply_accepted_proposal` lock the pair's
    // `match_proposal` row `FOR UPDATE` FIRST and only submit (taking CARNLK) SECOND. If
    // this function instead submitted first and updated `match_proposal` second, two
    // transactions judging the same pair could deadlock: one holding CARNLK and wanting
    // the row, the other holding the row and wanting CARNLK — Postgres detects the cycle
    // and aborts one with 40P01, which a clinician would see as a random failure.
    //
    // Locking the row here FIRST — even though the actual UPDATE happens later, below —
    // establishes ONE order for every path: row, then CARNLK. `query_opt`: a pair with no
    // open proposal has no row to lock, which is fine — there is nothing to serialize
    // against, and this is then a genuine no-op. When the caller (e.g.
    // `apply_accepted_proposal`) already holds the row FOR UPDATE from its own earlier
    // read, re-locking it here in the SAME transaction is a harmless no-op re-acquire.
    tx.query_opt(
        "SELECT 1 FROM match_proposal \
         WHERE patient_low = $1::text::uuid AND patient_high = $2::text::uuid FOR UPDATE",
        &[&low.to_string(), &high.to_string()],
    )
    .await?;

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
    .await?;

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
        .await?;
    Ok((event_id, moved > 0))
}

/// "Same person": link two charts as the reviewer's attested judgement. Both charts must
/// be held here.
///
/// An `Err` whose message says "recorded as event …" means the judgement IS committed —
/// only the read-back of the chart set failed. Do not retry it: a retry mints a second
/// event. Every other `Err` means nothing was written.
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
/// edge but cannot split the record — [`LinkOutcome::still_joined`] says so, and the other
/// link must be unlinked too.
///
/// One chart may be a member this node does not hold (R1 shows it; a peer's link named
/// it), provided it reads as part of the other chart's record here.
///
/// Errors: as [`link_charts`] — "recorded as event …" means the judgement is committed.
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
        .await?
        .get(0))
}

/// The shared body of both entry points: pre-checks (legible refusals before anything is
/// signed), one transaction, and the resulting chart set.
async fn judge(
    client: &mut tokio_postgres::Client,
    verb: LinkVerb,
    a: Uuid,
    b: Uuid,
    reviewer: &Reviewer<'_>,
    node_origin: &str,
) -> anyhow::Result<LinkOutcome> {
    if a == b {
        anyhow::bail!("{a} and {b} are the same chart — a chart cannot be linked to itself");
    }
    let (a_held, b_held) = (is_held(client, a).await?, is_held(client, b).await?);
    // Only asked when it can change the answer: an unlink with exactly one chart unheld.
    let shared_record = verb == LinkVerb::Unlink
        && a_held != b_held
        && crate::patient::person::person_charts(&*client, a)
            .await?
            .contains(&b);
    let about = admit_judgement(verb, (a, a_held), (b, b_held), shared_record)
        .map_err(anyhow::Error::msg)?;
    // Legibility only; the db/005 gate is the enforcement (a raw-SQL client skipping this
    // still cannot attest with a non-human key).
    if !crate::identify::attester_is_enrolled_human(client, reviewer.human_kid).await? {
        anyhow::bail!(
            "key {} is not an enrolled human actor — linking or unlinking charts is a human \
             judgement (unlock a clinician's key)",
            reviewer.human_kid
        );
    }

    let (low, high) = canonical_pair(a, b);
    let provenance = compose_review_provenance(verb, reviewer.human_kid);
    // The tick self-commits before the transaction; a rolled-back judgement leaves only a
    // clock gap, which the HLC allows (the identify_patient shape).
    let hlc = crate::db::next_hlc(client, node_origin).await?;
    let tx = client.transaction().await?;
    let (event_id, proposal_resolved) = assert_link_in_tx(
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
    tx.commit().await?;

    // From here the judgement is DURABLE. A failed read-back must not look like a failed
    // judgement — an operator who retries would mint a second event — so the error names
    // the committed event.
    let charts = crate::patient::person::person_charts(&*client, a)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "the judgement is recorded as event {event_id}; could not re-read the chart \
                 set afterwards: {e}"
            )
        })?;
    // An unlink that left b in a's record: joined through another link (see the field).
    let still_joined = verb == LinkVerb::Unlink && charts.contains(&b);
    Ok(LinkOutcome {
        event_id,
        proposal_resolved,
        charts,
        still_joined,
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
    fn a_link_needs_both_charts_held() {
        let (lo, hi) = pair();
        assert_eq!(
            admit_judgement(LinkVerb::Link, (hi, true), (lo, true), false),
            Ok(lo)
        );
        // Even a chart already in the record: a link must not reach past this node.
        let err = admit_judgement(LinkVerb::Link, (lo, true), (hi, false), true).unwrap_err();
        assert!(err.contains(&hi.to_string()), "{err}");
        assert!(
            !err.contains("not part of"),
            "true of this chart, so unsaid: {err}"
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
        let err = admit_judgement(LinkVerb::Unlink, (hi, true), (lo, false), false).unwrap_err();
        assert!(err.contains(&lo.to_string()), "names the stranger: {err}");
        let err = admit_judgement(LinkVerb::Unlink, (lo, false), (hi, false), true).unwrap_err();
        assert!(err.contains(&lo.to_string()) && err.contains(&hi.to_string()));
    }
}
