//! §5.2/§5.7 C2 apply seam: turn a human-ACCEPTED match_proposal (db/017) into a
//! human-ATTESTED `identity.link.asserted` event through the existing submit_event
//! door. This module owns the seam; it changes no floor. The link event is *additive*
//! but carries a responsibility-bearing contributor (the accepting human), which trips
//! the existing db/005 attestation gate — so submit_event requires a valid human token
//! bound to this event. The event construction lives here (Rust, §9 safety-critical
//! tier) and reuses cairn-event's serialization verbatim — never re-serialized elsewhere.
//!
//! Split: pure body-assembly (unit-testable, no DB; now a one-line delegate to
//! `chart_link::build_attested_assertion_body`) + one IO function that reads the
//! proposal (and requires it 'accepted') and then delegates the write — sign, attest,
//! submit, and mark the proposal applied, all in one transaction — to the shared
//! `chart_link::assert_link_in_tx` core, so this C2 matcher-accepted path and the R2a
//! chart-review path (`chart_link::link_charts`/`unlink_charts`) cannot drift apart.

use cairn_event::{EventBody, Hlc, SigningKey};
use uuid::Uuid;

/// Compose the §4.1 provenance string for a matcher-proposed, human-accepted link.
/// Non-empty by construction (the db/018 floor requires it) and legible: it records
/// both the ADR-0014 matcher config digest AND that a specific human vouched.
pub fn compose_provenance(matcher_version: &str, human_kid: &str) -> String {
    format!("matcher:{matcher_version} accepted-by:{human_kid}")
}

/// Assemble the `identity.link.asserted` EventBody for an accepted proposal. Pure:
/// `event_id` is supplied by the caller (so this stays deterministic and testable, and
/// the caller can reuse the same id as match_proposal.applied_event_id). `low`/`high`
/// are the canonical pair (low < high); subject_a := low, subject_b := high. The
/// accepting human is the sole contributor and carries a `responsibility` marker — this
/// is what makes submit_event demand a valid human attestation token.
///
/// Delegates to `chart_link::build_attested_assertion_body` — one body builder for every
/// attested identity judgement, so this C2 accepted-proposal path and the R2a
/// chart-review path share the exact same wire shape.
pub fn build_attested_link_body(
    event_id: Uuid,
    low: Uuid,
    high: Uuid,
    provenance: &str,
    confidence: Option<&str>,
    human_kid: &str,
    hlc: Hlc,
) -> EventBody {
    crate::chart_link::build_attested_assertion_body(
        crate::chart_link::LinkVerb::Link,
        event_id,
        low,
        high,
        provenance,
        confidence,
        human_kid,
        hlc,
    )
}

/// Apply one human-ACCEPTED match_proposal: read it here (requiring 'accepted'), then
/// delegate the write — build, sign, attest, submit the link event through the existing
/// 3-arg submit_event door, and mark the proposal applied — to the shared
/// `chart_link::assert_link_in_tx` core, all in ONE transaction. This IO function's own
/// job is now just: read + validate the proposal, compose its provenance/confidence, and
/// hand off; `chart_link::assert_link_in_tx` is the one place either an accepted-proposal
/// apply or a human chart-review judgement writes an attested link/unlink event.
///
/// Atomicity is the idempotency guarantee: if submit_event rejects (e.g. a non-human
/// attester) or any step fails, the whole transaction rolls back, so no link event is
/// written and the proposal stays 'accepted' to be retried. On success the event and
/// the 'applied' transition commit together, and a re-run finds no 'accepted' row.
///
/// Concurrency: the proposal row is read `FOR UPDATE` here, so two callers racing on the
/// same pair serialize — the second blocks on the row lock, then re-reads the now-
/// 'applied' status and bails. Without the lock both would read 'accepted' under READ
/// COMMITTED and each append its own link event. `assert_link_in_tx` also (re-)locks the
/// row before it submits — a harmless no-op re-acquire here, since this function already
/// holds it — which is what keeps this path's lock order (row, then db/018's CARNLK
/// advisory lock) the same as `auto_apply.rs`'s, so the two can never deadlock on the
/// same pair.
///
/// The pair may be passed in either order: it is canonicalized to `(least, greatest)` to
/// match match_proposal's `CHECK (patient_low < patient_high)` storage, so a caller passing
/// `(high, low)` still finds the accepted proposal rather than silently missing it.
///
/// Errors (Err, transaction rolled back) if the proposal is absent or its status is not
/// 'accepted' (only a human's acceptance applies), or if the in-DB floor refuses.
pub async fn apply_accepted_proposal(
    client: &mut tokio_postgres::Client,
    low: Uuid,
    high: Uuid,
    human_sk: &SigningKey,
    human_kid: &str,
    hlc: Hlc,
) -> anyhow::Result<Uuid> {
    let tx = client.transaction().await?;

    // Canonicalize to (least, greatest) so the pair matches match_proposal's
    // `CHECK (patient_low < patient_high)` storage regardless of the order the caller
    // supplied. `chart_link::assert_link_in_tx` then also receives the canonical pair
    // (subject_a := low), matching the C1 edge overlay's canonical (low, high) key.
    let (low, high) = crate::chart_link::canonical_pair(low, high);

    // Text-cast the UUIDs at the binding boundary: this crate's `uuid` dependency has no
    // `postgres`/`with-uuid-1` feature enabled, so `Uuid` does not implement `ToSql` here.
    // `.to_string()` + `$N::text::uuid` in the SQL is the established convention already
    // used throughout this crate (see e.g. `tests/identity_linkage.rs::person_of`).
    let (low_s, high_s) = (low.to_string(), high.to_string());

    // 1. Read the proposal and require status='accepted'. `FOR UPDATE` locks the row so a
    //    concurrent apply of the same pair blocks here and then sees the 'applied' status
    //    instead of racing to append a second link event (see the doc's Concurrency note).
    let row = tx
        .query_opt(
            "SELECT score_total, matcher_version, status FROM match_proposal \
             WHERE patient_low=$1::text::uuid AND patient_high=$2::text::uuid \
             FOR UPDATE",
            &[&low_s, &high_s],
        )
        .await?
        .ok_or_else(|| anyhow::anyhow!("no match_proposal for pair ({low}, {high})"))?;
    let score: f64 = row.get(0);
    let matcher_version: String = row.get(1);
    let status: String = row.get(2);
    if status != "accepted" {
        // Rolls back on drop; nothing was written.
        anyhow::bail!(
            "match_proposal ({low}, {high}) is '{status}', not 'accepted' — refusing to apply"
        );
    }

    // 2. Compose provenance + confidence.
    let provenance = compose_provenance(&matcher_version, human_kid);
    let confidence = format!("{score:.3}");

    // 2–5. Build, sign, attest and submit through the shared core, which also moves this
    //      (open, 'accepted') proposal to 'applied' with its event id — in this transaction.
    let reviewer = crate::chart_link::Reviewer {
        human_sk,
        human_kid,
    };
    let (event_id, moved) = crate::chart_link::assert_link_in_tx(
        &tx,
        crate::chart_link::LinkVerb::Link,
        low,
        high,
        &provenance,
        Some(&confidence),
        &reviewer,
        hlc,
    )
    .await?;
    // The row was read FOR UPDATE as 'accepted' above, so the core's open-status move must
    // have hit it; anything else is a logic error, not a state to commit.
    anyhow::ensure!(
        moved,
        "match_proposal ({low}, {high}) did not move to 'applied'"
    );
    tx.commit().await?;
    Ok(event_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids() -> (Uuid, Uuid, Uuid) {
        // Fixed, ordered UUIDs so low < high is stable and assertions are deterministic.
        let a = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let b = Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
        let eid = Uuid::parse_str("11111111-0000-0000-0000-000000000000").unwrap();
        (eid, a, b)
    }

    #[test]
    fn provenance_is_nonempty_and_names_matcher_and_human() {
        let p = compose_provenance("cfg@abc", "humankid");
        assert!(p.contains("cfg@abc"));
        assert!(p.contains("humankid"));
        assert!(!p.trim().is_empty());
    }

    #[test]
    fn body_carries_responsibility_bearing_human_contributor() {
        let (eid, a, b) = ids();
        let body = build_attested_link_body(
            eid,
            a,
            b,
            "matcher:x accepted-by:h",
            None,
            "h",
            Hlc {
                wall: 5,
                counter: 0,
                node_origin: "n".into(),
            },
        );
        let c = &body.contributors[0];
        assert_eq!(c["actor_id"], "h");
        assert_eq!(c["role"], "attested");
        assert!(
            c.get("responsibility").is_some(),
            "must carry a responsibility marker to trip the attestation gate"
        );
    }

    #[test]
    fn body_is_a_link_event_with_authored_twin_and_canonical_subjects() {
        let (eid, a, b) = ids();
        let body = build_attested_link_body(
            eid,
            a,
            b,
            "matcher:x accepted-by:h",
            Some("0.910"),
            "h",
            Hlc {
                wall: 5,
                counter: 0,
                node_origin: "n".into(),
            },
        );
        assert_eq!(body.event_type, "identity.link.asserted");
        assert_eq!(body.payload["subject_a"], a.to_string());
        assert_eq!(body.payload["subject_b"], b.to_string());
        assert_eq!(body.payload["confidence"], "0.910");
        let twin = body.plaintext_twin.as_deref().unwrap();
        assert!(
            twin.starts_with("link: "),
            "authored twin required by the db/018 floor"
        );
    }
}
