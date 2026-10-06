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
//! (`assert_link_in_tx`) that `apply_proposal::apply_accepted_proposal` also uses, the pure
//! admission rule (`admit.rs`), and the two public entry points that add the pre-checks and
//! the transaction (`judge.rs`). Both submodules are re-exported, so every public item is
//! reached as `chart_link::…`.
//!
//! Identity events are CLEAR (db/005 refuses a sealed non-clinical body), so this signs and
//! attests directly and submits through the 3-argument `submit_event` door — never the
//! medication seal path.

use crate::db_diagnosis::{deliberate_refusal, LocalDbFault};
use anyhow::Context;
use cairn_event::identity::{
    link_assertion_body, render_link_twin, render_unlink_twin, unlink_assertion_body, LinkAssertion,
};
use cairn_event::{event_address, sign, sign_attestation, EventBody, Hlc, SigningKey};
use uuid::Uuid;

pub mod admit;
pub use admit::*;
pub mod judge;
pub use judge::*;

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
/// `about` is the chart the envelope is filed under — a subject, or for an unlink the chart
/// the judgement was made from; see [`FiledUnder`]. [`assert_link_in_tx`] refuses a wrong one
/// before calling this (the db/018 floor does not check it, and a wrong `about` would file an
/// identity event in an unrelated patient's stream). The C1 convention is `low`; an unlink
/// naming a chart not held here (see [`admit_judgement`]) is filed under the HELD chart
/// instead, because db/005 step 8b refuses a local event about a chart with no history on
/// this node. The payload's subjects stay canonical either way — db/018 reads the pair from
/// them, never from the envelope.
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
///
/// db/057's view `match_proposal_open` lists exactly these, so the banner and
/// `assert_link_in_tx`'s status move agree on what "open" means (`tests/match_proposal_open.rs`
/// pins the composed SQL).
pub const OPEN_PROPOSAL_STATUSES: [&str; 3] = ["pending", "accepted", "review"];

/// The one fact a third-chart filing ([`FiledUnder::RecordOf`]) rests on — `opened`'s record
/// holds both subjects — checked where it cannot go stale before the commit.
///
/// A plain re-read inside a READ COMMITTED transaction is not enough: a peer's unlink applied
/// through the sync door could commit between the read and this judgement's submit, and the
/// event would then be filed (and graded, db/048) under a chart whose record no longer holds
/// either subject. So CARNLK — db/018's global link lock, which every link/unlink apply (the only
/// writer of a record's membership, `person_member`) takes and holds until commit — is taken
/// FIRST, and the record read after it. The caller has
/// already locked the proposal row, so the order stays row-then-CARNLK; `submit_event`'s own
/// acquisition later in this transaction is a no-op re-acquire (advisory locks stack).
async fn refuse_unless_record_holds_both(
    tx: &tokio_postgres::Transaction<'_>,
    opened: Uuid,
    low: Uuid,
    high: Uuid,
) -> anyhow::Result<()> {
    tx.execute("SELECT pg_advisory_xact_lock(x'4341524E4C4B'::bigint)", &[])
        .await
        .map_err(|e| {
            LocalDbFault::new(
                "taking the link lock (CARNLK) before re-reading the record",
                e,
            )
        })?;
    let record = crate::patient::person::person_charts(tx, opened)
        .await
        .context("re-reading the open chart's record under the link lock (CARNLK)")?;
    if record_holds_both(&record, low, high) {
        return Ok(());
    }
    // A changed record is stale INPUT (the caller judged from a picture that no longer holds),
    // not a not-yet node state: the same call refuses the same way until the caller re-reads.
    Err(deliberate_refusal(format!(
        "chart {opened}'s record no longer reads both {low} and {high} as part of it — the \
         record changed while you were judging — nothing was done; reload the chart and judge \
         again"
    )))
}

/// Sign, attest and submit one judgement inside the caller's transaction, then move an
/// OPEN proposal for the pair and read back whether the event stands. See [`Asserted`].
///
/// Shared with `apply_proposal::apply_accepted_proposal`, so the matcher-proposal path
/// and the chart-review path cannot drift. `low`/`high` must be canonical (`low < high`)
/// and `filed` naming `low` or `high` — or, for an UNLINK only, the held chart whose record
/// contains both ([`FiledUnder::RecordOf`], #699 (a)); both are checked here, before anything
/// is locked or signed, because a reversed pair would lock and move no proposal row and a
/// wrong filing would misfile the event — and neither is visible to the database floor.
///
/// Locks the pair's `match_proposal` row (if any) FIRST, before signing or submitting
/// anything — see the inline comment at the top of the body for why: it keeps this
/// function's lock order (row, then db/018's CARNLK advisory lock) the same as every
/// other path that touches both, so two judgements on the same pair cannot deadlock.
///
/// A `RecordOf` filing then takes CARNLK itself and re-reads that chart's record: the filing
/// is only honest while the record holds both subjects, and CARNLK is what every link/unlink
/// apply — the only writer of a record's membership, a peer's unlink arriving through the sync
/// door included — holds until it commits, so the record read after taking it cannot change
/// before this judgement commits. A record that no
/// longer holds both is refused as a verdict about the input (reload and judge again).
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
    filed: FiledUnder,
    provenance: &str,
    confidence: Option<&str>,
    reviewer: &Reviewer<'_>,
    hlc: Hlc,
) -> anyhow::Result<Asserted> {
    anyhow::ensure!(
        low < high,
        "a judgement's pair must be canonical and distinct: ({low}, {high})"
    );
    // Checked before anything is locked or signed: a wrong envelope chart is invisible to the
    // database floor (db/018 reads the pair from the payload). See `admit::filing_for`.
    let about = filing_for(verb, low, high, filed).map_err(anyhow::Error::msg)?;
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
    if let FiledUnder::RecordOf(opened) = filed {
        refuse_unless_record_holds_both(tx, opened, low, high).await?;
    }

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
                "filed under subject_a = low, by the C1 convention"
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
    fn a_standing_link_knows_its_own_event() {
        let w = StandingLink {
            content_address: vec![1, 2, 3],
            state: "unlink".into(),
            attested: true,
        };
        assert!(w.is(&[1, 2, 3]));
        assert!(!w.is(&[1, 2, 4]));
    }
}
