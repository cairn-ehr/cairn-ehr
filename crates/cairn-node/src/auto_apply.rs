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

use crate::db_diagnosis::{operator_chain, LocalDbFault};
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
    let la = LinkAssertion {
        subject_a: &low_s,
        subject_b: &high_s,
        provenance,
        confidence,
    };
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
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
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
    let (low, high) = if low <= high {
        (low, high)
    } else {
        (high, low)
    };
    let (low_s, high_s) = (low.to_string(), high.to_string());
    // Every postgres call in this function names the operation it was performing, so a
    // failure on the §5.7 auto-apply ceremony says which step met the database and with
    // what SQLSTATE (#477). `LocalDbFault` renders legibly AND keeps the
    // `tokio_postgres::Error` reachable as `source()`.
    let tx = client
        .transaction()
        .await
        .map_err(|e| LocalDbFault::new("opening the auto-apply transaction", e))?;

    // 1. Lock the row; require it is an auto_candidate still awaiting disposition.
    let row = tx
        .query_opt(
            "SELECT band, status, score_total, matcher_version FROM match_proposal \
             WHERE patient_low=$1::text::uuid AND patient_high=$2::text::uuid FOR UPDATE",
            &[&low_s, &high_s],
        )
        .await
        .map_err(|e| LocalDbFault::new("locking the match proposal", e))?;
    let Some(row) = row else {
        return Ok(AutoOutcome::Skipped(format!(
            "no proposal for ({low}, {high})"
        )));
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
        .await
        .map_err(|e| LocalDbFault::new("re-checking the db/016 veto floor", e))?
        .get(0);
    if vetoed {
        tx.execute(
            "UPDATE match_proposal SET status='review', updated_at=clock_timestamp() \
             WHERE patient_low=$1::text::uuid AND patient_high=$2::text::uuid",
            &[&low_s, &high_s],
        )
        .await
        .map_err(|e| LocalDbFault::new("kicking a vetoed pair to human review", e))?;
        tx.commit()
            .await
            .map_err(|e| LocalDbFault::new("committing the veto-to-review update", e))?;
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
    tx.execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await
        .map_err(|e| LocalDbFault::new("submitting the matcher link through the floor", e))?;

    // 5. Mark the proposal auto_applied (distinct from C2's human 'applied').
    let event_id_s = event_id.to_string();
    tx.execute(
        "UPDATE match_proposal SET status='auto_applied', applied_event_id=$3::text::uuid, updated_at=clock_timestamp() \
         WHERE patient_low=$1::text::uuid AND patient_high=$2::text::uuid",
        &[&low_s, &high_s, &event_id_s],
    )
    .await
    .map_err(|e| LocalDbFault::new("marking the proposal auto_applied", e))?;

    tx.commit()
        .await
        .map_err(|e| LocalDbFault::new("committing the auto-applied link", e))?;
    Ok(AutoOutcome::Applied(event_id))
}

/// Batch outcome counts for the operator's summary line.
pub struct AutoSummary {
    pub applied: usize,
    pub vetoed_to_review: usize,
    pub skipped: usize,
    /// Pairs that hit a HARD error (matcher-actor resolve failed, or the apply txn errored).
    /// Kept separate from the benign `skipped` bucket so a systematic failure (a floor
    /// change rejecting every submit, a sealed-but-unopenable key, a revoked epoch) can
    /// never masquerade as a healthy quiet run — the CLI turns any `errored` into a
    /// non-zero exit.
    pub errored: usize,
}

/// Session advisory-lock key for the auto-apply ceremony. Two-int form (namespace
/// 0x4341524E = "CARN", slot 2) so it occupies a DIFFERENT lock space from the
/// single-bigint `db::test_serial_guard` and can never collide with it.
const AUTO_APPLY_LOCK_NS: i32 = 0x4341524E;
const AUTO_APPLY_LOCK_SLOT: i32 = 2;

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
    // Serialize concurrent owner ceremonies. Two apply-auto-candidates runs racing on a
    // brand-new epoch would BOTH see no key file and BOTH mint+enroll it (a TOCTOU on the
    // per-epoch key file -> divergent on-disk keys and duplicate enroll rows). A session
    // advisory lock makes the ceremony single-writer; it auto-releases when this
    // short-lived connection closes.
    let got_lock: bool = client
        .query_one(
            "SELECT pg_try_advisory_lock($1, $2)",
            &[&AUTO_APPLY_LOCK_NS, &AUTO_APPLY_LOCK_SLOT],
        )
        .await
        .map_err(|e| LocalDbFault::new("taking the auto-apply advisory lock", e))?
        .get(0);
    if !got_lock {
        anyhow::bail!(
            "another apply-auto-candidates run holds the auto-apply lock — retry once it finishes"
        );
    }

    // Run the ceremony body, then release the lock on EVERY exit — success or error
    // (#213: the body's `?` returns previously kept the session lock held, so on a
    // long-lived owner connection one failed run blocked every later run until
    // disconnect). Disconnect still releases it as the backstop; best-effort unlock
    // (an unlock failure means the connection is dying, which releases it anyway).
    let result = ceremony_locked(client, keystore_dir, secret, node_origin).await;
    let _ = client
        .execute(
            "SELECT pg_advisory_unlock($1, $2)",
            &[&AUTO_APPLY_LOCK_NS, &AUTO_APPLY_LOCK_SLOT],
        )
        .await;
    result
}

/// The operator line for an epoch whose matcher actor could not be resolved. **Pure.**
///
/// # Why `operator_chain` and not `{e}` (issue #477)
///
/// `anyhow`'s plain `Display` prints only the OUTERMOST message. Before this change every
/// postgres call below reached the database with a bare `?`, so the outermost error WAS a
/// `tokio_postgres::Error` — whose own `Display` is the literal string `db error`, and
/// that was the whole content of this line on the §5.7 identity auto-apply ceremony.
///
/// It matters more here than the two words suggest. The caller counts the failure and
/// CONTINUES, so a run where every pair failed for one missing grant looked exactly like a
/// run where every pair failed for a different reason — and this path is the one that
/// links two patient charts together without a human in the loop.
///
/// Both halves of the fix are needed, and it is worth being exact about which does what:
/// `resolve_matcher_actor`'s three registry calls now name their operation, so the
/// outermost `Display` is already legible; `operator_chain` is what keeps that true when a
/// caller adds a context layer, and what still renders a call somebody forgets to name.
fn resolve_failure_line(version: &str, e: &anyhow::Error) -> String {
    format!(
        "auto-apply resolve epoch '{version}': {}",
        operator_chain(e)
    )
}

/// The operator line for one pair whose apply transaction failed. **Pure.**
///
/// [`resolve_failure_line`]'s sibling, and the one that fires once per pair. Same defect,
/// same fix; the pair is named because the summary counts alone cannot say WHICH charts
/// were left unlinked.
///
/// The same exactness applies: `apply_auto_candidate`'s ten postgres calls are wrapped, so
/// the outermost `Display` is legible on its own. `operator_chain` earns its place on the
/// paths that are NOT database errors at all — a signing failure, a future unwrapped call —
/// and on any context layer a caller adds above.
fn apply_failure_line(low: Uuid, high: Uuid, e: &anyhow::Error) -> String {
    format!("auto-apply ({low},{high}): {}", operator_chain(e))
}

/// The auto-apply ceremony body. MUST only run under the session advisory lock taken
/// by [`apply_auto_candidates`] — factored out so the lock/unlock bracket wraps every
/// early `?` return here by construction.
async fn ceremony_locked(
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
        .await
        .map_err(|e| LocalDbFault::new("reading the auto-candidate worklist", e))?;

    let mut keys: HashMap<String, (SigningKey, String)> = HashMap::new();
    // Epochs whose resolve already failed this run — so we neither re-resolve nor re-log
    // for every remaining pair of a broken/revoked epoch, but STILL count each affected
    // pair as errored (the operator sees the true blast radius, not one line).
    let mut failed_versions: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut summary = AutoSummary {
        applied: 0,
        vetoed_to_review: 0,
        skipped: 0,
        errored: 0,
    };

    for r in rows {
        let low: Uuid = r.get::<_, String>(0).parse()?;
        let high: Uuid = r.get::<_, String>(1).parse()?;
        let version: String = r.get(2);

        if failed_versions.contains(&version) {
            summary.errored += 1;
            continue;
        }

        // Resolve (and cache) the matcher key/actor for this epoch. Enrollment happens
        // BEFORE the event is submitted, so the event's admission-time stamp attributes
        // it 'pinned' to this epoch (db/006), not 'pre-registration'. A resolve FAILURE
        // (sealed key with no secret, or a revoked epoch we refuse to resurrect) must NOT
        // abort the batch — skip this epoch's pairs, count them errored, keep going, so one
        // bad epoch never strands healthy pairs of other epochs (the skip-and-report the
        // doc-comment promises).
        if !keys.contains_key(&version) {
            match resolve_matcher_actor(client, keystore_dir, secret, &version).await {
                Ok(resolved) => {
                    keys.insert(version.clone(), resolved);
                }
                Err(e) => {
                    eprintln!("{}", resolve_failure_line(&version, &e));
                    failed_versions.insert(version.clone());
                    summary.errored += 1;
                    continue;
                }
            }
        }
        // Clone out of the cache so no immutable borrow of `client`/`keys` is held across
        // the `&mut client` apply call below.
        let (sk, kid) = {
            let (sk, kid) = keys.get(&version).unwrap();
            (sk.clone(), kid.clone())
        };

        let hlc = crate::db::next_hlc(client, node_origin).await?;
        match apply_auto_candidate(client, low, high, &sk, &kid, hlc).await {
            Ok(AutoOutcome::Applied(_)) => summary.applied += 1,
            Ok(AutoOutcome::VetoedToReview) => summary.vetoed_to_review += 1,
            Ok(AutoOutcome::Skipped(_)) => summary.skipped += 1,
            Err(e) => {
                eprintln!("{}", apply_failure_line(low, high, &e));
                summary.errored += 1;
            }
        }
    }

    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real `tokio_postgres::Error` with a live `source()`, built with NO database:
    /// `Config`'s own parser is the only way to get one by hand, because `DbError` cannot
    /// be constructed outside `tokio-postgres`. The same fixture `db_diagnosis` uses.
    fn a_real_pg_error() -> tokio_postgres::Error {
        "host=localhost port=not-a-number"
            .parse::<tokio_postgres::Config>()
            .expect_err("a non-numeric port is not a parseable connection string")
    }

    /// Issue #477: both operator lines on the §5.7 auto-apply ceremony said `db error`.
    ///
    /// `apply_auto_candidate` returns `anyhow::Result` and reaches the database with a
    /// bare `?`, so the outermost error IS the `tokio_postgres::Error` — and `anyhow`'s
    /// plain `Display` prints only the outermost message, which is that literal string.
    /// The line then increments `summary.errored` and continues, so a run where EVERY
    /// pair failed for one missing grant is indistinguishable from one where every pair
    /// failed for a different reason.
    #[test]
    fn a_failed_resolve_names_the_epoch_and_the_diagnosis() {
        let e = anyhow::Error::from(a_real_pg_error());
        let line = resolve_failure_line("2026.07.1", &e);

        assert!(line.contains("2026.07.1"), "names the epoch: {line}");
        assert!(
            line.contains("port"),
            "…and the diagnosis, not the kind: {line}"
        );
        assert!(!line.ends_with("db error"), "#477's species: {line}");
        assert!(!line.contains('\n'), "one line per event: {line}");
    }

    /// The sibling line, and the one that fires once per pair.
    #[test]
    fn a_failed_apply_names_the_pair_and_the_diagnosis() {
        let (a, b, _) = ids();
        // The shape production builds since this change: a named operation over the
        // database error, with a caller's context layer on top.
        let e = anyhow::Error::from(LocalDbFault::new(
            "locking the match proposal",
            a_real_pg_error(),
        ))
        .context("auto-applying a matcher link");
        let line = apply_failure_line(a, b, &e);

        assert!(line.contains(&a.to_string()), "names the pair: {line}");
        assert!(line.contains(&b.to_string()), "names the pair: {line}");
        assert!(
            line.contains("locking the match proposal"),
            "…the operation that failed: {line}"
        );
        assert!(line.contains("port"), "…and the diagnosis: {line}");
        assert_eq!(
            line.matches("invalid value for option `port`").count(),
            1,
            "said exactly once: {line}"
        );
    }

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
        assert!(
            !p.contains("accepted-by"),
            "the auto path has no human voucher"
        );
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
            Hlc {
                wall: 5,
                counter: 0,
                node_origin: "n".into(),
            },
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
            Hlc {
                wall: 5,
                counter: 0,
                node_origin: "n".into(),
            },
        );
        assert_eq!(body.event_type, "identity.link.asserted");
        assert_eq!(body.payload["subject_a"], a.to_string());
        assert_eq!(body.payload["subject_b"], b.to_string());
        assert_eq!(body.payload["confidence"], "0.950");
        assert!(
            body.plaintext_twin
                .as_deref()
                .unwrap()
                .starts_with("link: "),
            "authored twin required by the db/018 floor"
        );
    }
}
