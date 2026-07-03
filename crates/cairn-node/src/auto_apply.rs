//! §5.2/§5.7 C2b — auto-apply of the matcher's `auto_candidate` band. Sibling of
//! `apply_proposal.rs` (the human-accepted C2 seam). Here the MATCHER authors the link
//! un-attested (contributor role `suggested`, no `responsibility`), so `submit_event`
//! requires NO attestation token (db/018: an identity link is additive +
//! targets_other_author=FALSE). Recallability comes for free: the matcher is a real
//! per-epoch `agent` actor (see `matcher_actor.rs`), so the db/006 recall surface can
//! recall a bad config's auto-links precisely.
//!
//! Split: pure body/provenance assembly (unit-tested, no DB) + IO functions — one
//! proposal (`apply_auto_candidate`) and the batch driver (`apply_auto_candidates`).

use crate::matcher_actor::resolve_matcher_actor;
use cairn_event::identity::{link_assertion_body, render_link_twin, LinkAssertion};
use cairn_event::{sign, EventBody, Hlc, SigningKey};
use std::collections::HashMap;
use std::path::Path;
use tokio_postgres::Client;
use uuid::Uuid;

/// schema_version for a link event (mirrors the C1/C2 convention).
const LINK_SCHEMA_VERSION: &str = "identity.link/1";

/// Compose the §4.1 provenance for a matcher-AUTO-applied link. Distinct from C2's
/// `matcher:{v} accepted-by:{kid}`: there is NO human, so it reads `matcher:{v} auto` —
/// legible that the link was applied by the matcher alone (no human vouched).
pub fn compose_auto_provenance(matcher_version: &str) -> String {
    format!("matcher:{matcher_version} auto")
}

/// Assemble the un-attested `identity.link.asserted` EventBody the matcher will sign.
/// Pure: `event_id` is supplied by the caller (deterministic/testable). `low`/`high` are
/// the canonical pair (low < high); subject_a := low. The SOLE contributor is the matcher
/// with role `suggested` (ADR-0028 contributory, non-bearing) and NO `responsibility` key
/// — this keeps the event off the db/005 attestation gate.
pub fn build_suggested_link_body(
    event_id: Uuid,
    low: Uuid,
    high: Uuid,
    provenance: &str,
    confidence: Option<&str>,
    matcher_kid: &str,
    hlc: Hlc,
) -> EventBody {
    let low_s = low.to_string();
    let high_s = high.to_string();
    let la = LinkAssertion { subject_a: &low_s, subject_b: &high_s, provenance, confidence };
    EventBody {
        event_id: event_id.to_string(),
        patient_id: low_s.clone(), // C1 convention: an identity event is "about" subject_a
        event_type: "identity.link.asserted".into(),
        schema_version: LINK_SCHEMA_VERSION.into(),
        hlc,
        t_effective: None,
        signer_key_id: matcher_kid.into(),
        // Authorship present (the matcher suggested the link), accountability ABSENT (no
        // `responsibility`) — principle 10 on the auto path. No responsibility ->
        // submit_event demands no attestation.
        contributors: serde_json::json!([
            {"actor_id": matcher_kid, "role": "suggested"}
        ]),
        payload: link_assertion_body(&la),
        attachments: vec![],
        plaintext_twin: Some(render_link_twin(&la)),
    }
}

/// The result of attempting to auto-apply one proposal.
pub enum AutoOutcome {
    /// A link event was appended; carries its event_id.
    Applied(Uuid),
    /// A veto appeared since propose; the proposal was kicked to human `review`.
    VetoedToReview,
    /// Not eligible (not auto_candidate, not pending, or absent); nothing changed.
    Skipped(String),
}

/// Apply ONE proposal: read it `FOR UPDATE`, require band='auto_candidate' AND
/// status='pending', RE-CHECK the db/016 veto (any severity) — a veto that appeared since
/// propose kicks the pair to human `review` instead of auto-linking — else build + sign an
/// un-attested link with the matcher's key, submit through the 1-arg `submit_event` door,
/// and mark the proposal 'auto_applied'. All in ONE transaction: any rejection rolls back,
/// so no event is written and the proposal stays 'pending' to retry (atomicity =
/// idempotency).
///
/// The pair may be passed in either order; it is canonicalized to `(least, greatest)` to
/// match match_proposal's `CHECK (patient_low < patient_high)`.
pub async fn apply_auto_candidate(
    client: &mut Client,
    low: Uuid,
    high: Uuid,
    matcher_sk: &SigningKey,
    matcher_kid: &str,
    hlc: Hlc,
) -> anyhow::Result<AutoOutcome> {
    let (low, high) = if low <= high { (low, high) } else { (high, low) };
    let (low_s, high_s) = (low.to_string(), high.to_string());
    let tx = client.transaction().await?;

    // 1. Lock the row; require it is an auto_candidate still awaiting disposition.
    let row = tx
        .query_opt(
            "SELECT band, status, score_total, matcher_version FROM match_proposal \
             WHERE patient_low=$1::text::uuid AND patient_high=$2::text::uuid FOR UPDATE",
            &[&low_s, &high_s],
        )
        .await?;
    let Some(row) = row else {
        return Ok(AutoOutcome::Skipped(format!("no proposal for ({low}, {high})")));
    };
    let band: String = row.get(0);
    let status: String = row.get(1);
    let score: f64 = row.get(2);
    let matcher_version: String = row.get(3);
    if band != "auto_candidate" || status != "pending" {
        return Ok(AutoOutcome::Skipped(format!(
            "band='{band}' status='{status}' — not an actionable auto_candidate"
        )));
    }

    // 2. Re-check the veto floor (no human backstop on this path). ANY veto (hard_veto or
    //    degrade_hold) forbids an auto-link — mirrors banding.py. A since-vetoed pair is
    //    kicked to a human, never auto-linked over.
    let vetoed: bool = tx
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM cairn_match_veto($1::text::uuid, $2::text::uuid))",
            &[&low_s, &high_s],
        )
        .await?
        .get(0);
    if vetoed {
        tx.execute(
            "UPDATE match_proposal SET status='review', updated_at=clock_timestamp() \
             WHERE patient_low=$1::text::uuid AND patient_high=$2::text::uuid",
            &[&low_s, &high_s],
        )
        .await?;
        tx.commit().await?;
        return Ok(AutoOutcome::VetoedToReview);
    }

    // 3. Build + sign the un-attested matcher link.
    let provenance = compose_auto_provenance(&matcher_version);
    let confidence = format!("{score:.3}");
    let event_id = Uuid::now_v7();
    let body = build_suggested_link_body(
        event_id,
        low,
        high,
        &provenance,
        Some(&confidence),
        matcher_kid,
        hlc,
    );
    let signed = sign(&body, matcher_sk)?;

    // 4. Submit through the 1-arg (un-attested) door. The db/018 identity floor +
    //    patient_link_apply trigger run here.
    tx.execute("SELECT submit_event($1)", &[&signed.signed_bytes]).await?;

    // 5. Mark the proposal auto_applied (distinct from C2's human 'applied').
    let event_id_s = event_id.to_string();
    tx.execute(
        "UPDATE match_proposal SET status='auto_applied', applied_event_id=$3::text::uuid, updated_at=clock_timestamp() \
         WHERE patient_low=$1::text::uuid AND patient_high=$2::text::uuid",
        &[&low_s, &high_s, &event_id_s],
    )
    .await?;

    tx.commit().await?;
    Ok(AutoOutcome::Applied(event_id))
}

/// Batch outcome counts for the operator's summary line.
pub struct AutoSummary {
    pub applied: usize,
    pub vetoed_to_review: usize,
    pub skipped: usize,
}

/// Tick the node HLC once (the same door node authoring uses) and stamp `node_origin`.
/// Authoring is single-threaded on a node, so tick->sign->submit per event is safe here.
async fn next_hlc(client: &Client, node_origin: &str) -> anyhow::Result<Hlc> {
    let row = client.query_one("SELECT wall, counter FROM node_hlc_tick()", &[]).await?;
    Ok(Hlc { wall: row.get(0), counter: row.get(1), node_origin: node_origin.into() })
}

/// Auto-apply EVERY pending auto_candidate proposal. Resolves the matcher actor once per
/// distinct epoch (cached), then applies each pair in its own transaction so one bad pair
/// never rolls back the batch (skip-and-report, mirroring pipeline/sweep.py). Owner-run:
/// `resolve_matcher_actor` enrolls actors, which the runtime role may not.
pub async fn apply_auto_candidates(
    client: &mut Client,
    keystore_dir: &Path,
    secret: Option<&str>,
    node_origin: &str,
) -> anyhow::Result<AutoSummary> {
    // Snapshot the worklist first (a read), then act — so we never hold a cursor across
    // the per-pair transactions.
    let rows = client
        .query(
            "SELECT patient_low::text, patient_high::text, matcher_version \
             FROM match_proposal WHERE band='auto_candidate' AND status='pending' \
             ORDER BY patient_low, patient_high",
            &[],
        )
        .await?;

    let mut keys: HashMap<String, (SigningKey, String)> = HashMap::new();
    let mut summary = AutoSummary { applied: 0, vetoed_to_review: 0, skipped: 0 };

    for r in rows {
        let low: Uuid = r.get::<_, String>(0).parse()?;
        let high: Uuid = r.get::<_, String>(1).parse()?;
        let version: String = r.get(2);

        // Resolve (and cache) the matcher key/actor for this epoch. Enrollment happens
        // BEFORE the event is submitted, so the event's admission-time stamp attributes
        // it 'pinned' to this epoch (db/006), not 'pre-registration'.
        if !keys.contains_key(&version) {
            let resolved = resolve_matcher_actor(client, keystore_dir, secret, &version).await?;
            keys.insert(version.clone(), resolved);
        }
        // Clone out of the cache so no immutable borrow of `client`/`keys` is held across
        // the `&mut client` apply call below.
        let (sk, kid) = {
            let (sk, kid) = keys.get(&version).unwrap();
            (sk.clone(), kid.clone())
        };

        let hlc = next_hlc(client, node_origin).await?;
        match apply_auto_candidate(client, low, high, &sk, &kid, hlc).await {
            Ok(AutoOutcome::Applied(_)) => summary.applied += 1,
            Ok(AutoOutcome::VetoedToReview) => summary.vetoed_to_review += 1,
            Ok(AutoOutcome::Skipped(_)) => summary.skipped += 1,
            Err(e) => {
                eprintln!("auto-apply ({low},{high}): {e}");
                summary.skipped += 1;
            }
        }
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids() -> (Uuid, Uuid, Uuid) {
        let a = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let b = Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
        let eid = Uuid::parse_str("22222222-0000-0000-0000-000000000000").unwrap();
        (eid, a, b)
    }

    #[test]
    fn provenance_names_version_and_auto_not_human() {
        let p = compose_auto_provenance("0.3.0+abc");
        assert!(p.contains("0.3.0+abc"));
        assert!(p.contains("auto"));
        assert!(!p.contains("accepted-by"), "the auto path has no human voucher");
    }

    #[test]
    fn body_contributor_is_suggested_with_no_responsibility() {
        let (eid, a, b) = ids();
        let body = build_suggested_link_body(
            eid,
            a,
            b,
            "matcher:x auto",
            None,
            "mkid",
            Hlc { wall: 5, counter: 0, node_origin: "n".into() },
        );
        let c = &body.contributors[0];
        assert_eq!(c["actor_id"], "mkid");
        assert_eq!(c["role"], "suggested");
        assert!(
            c.get("responsibility").is_none(),
            "the matcher bears NO responsibility -> no attestation required"
        );
    }

    #[test]
    fn body_is_a_link_event_with_authored_twin_and_canonical_subjects() {
        let (eid, a, b) = ids();
        let body = build_suggested_link_body(
            eid,
            a,
            b,
            "matcher:x auto",
            Some("0.950"),
            "mkid",
            Hlc { wall: 5, counter: 0, node_origin: "n".into() },
        );
        assert_eq!(body.event_type, "identity.link.asserted");
        assert_eq!(body.payload["subject_a"], a.to_string());
        assert_eq!(body.payload["subject_b"], b.to_string());
        assert_eq!(body.payload["confidence"], "0.950");
        assert!(
            body.plaintext_twin.as_deref().unwrap().starts_with("link: "),
            "authored twin required by the db/018 floor"
        );
    }
}
