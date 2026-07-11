//! §5.4 finisher 3 — resolve a John-Doe chart: record WHO the patient is
//! (`identity.identify.asserted`, flipping the chart to *confirmed*) and, when that
//! person already has a prior chart, OPTIONALLY join the two (`identity.link.asserted`).
//!
//! Accountability (see the design doc): the identify is **device-additive** — authored
//! with the node key exactly like `register_john_doe`, no attestation. The optional link
//! MERGES a chart into a prior identity — a real human attribution — so it is signed AND
//! attested by a **human** key supplied at the CLI. This module owns the authoring seam;
//! it changes no floor and re-uses `apply_proposal::build_attested_link_body` verbatim.
//!
//! Split: pure body assembly (unit-tested, no DB) + one async orchestrator that authors
//! the identify and the optional link in ONE transaction (atomic — never confirmed-but-
//! half-linked when a link was intended).

// `build_attested_link_body` lives in the sibling apply_proposal module; import it directly
// so the reuse is explicit and no link body is ever re-serialized here.
use crate::apply_proposal::build_attested_link_body;
use cairn_event::identity::{identify_assertion_body, render_identify_twin, IdentifyAssertion};
use cairn_event::{event_address, sign, sign_attestation, EventBody, Hlc, SigningKey};
use uuid::Uuid;

/// schema_version for the identify marker (mirrors `john_doe.rs`'s per-type constants).
const IDENTIFY_SCHEMA_VERSION: &str = "identity.identify.asserted/1";

/// Assemble the device-additive `identity.identify.asserted` `EventBody`. Pure:
/// `event_id`, `hlc`, and the resolved strings are supplied so the body is fully
/// testable. The sole contributor is the registering node actor with role `recorded`
/// (it recorded the identification) — additive, so no attestation is demanded. `method`
/// is §5.7's "method recorded"; the db/024 floor rejects it empty.
pub fn build_identify_body(
    event_id: Uuid,
    patient: Uuid,
    method: &str,
    node_kid: &str,
    hlc: Hlc,
) -> EventBody {
    let pid = patient.to_string();
    let a = IdentifyAssertion {
        subject: &pid,
        method,
    };
    EventBody {
        event_id: event_id.to_string(),
        patient_id: pid.clone(), // an identity-state assertion is "about" its subject's chart
        event_type: "identity.identify.asserted".into(),
        schema_version: IDENTIFY_SCHEMA_VERSION.into(),
        hlc,
        t_effective: None,
        signer_key_id: node_kid.into(),
        contributors: serde_json::json!([{"actor_id": node_kid, "role": "recorded"}]),
        payload: identify_assertion_body(&a),
        attachments: vec![],
        plaintext_twin: Some(render_identify_twin(&a)),
    }
}

/// Compose the §4.1 provenance for a link authored while resolving a John-Doe chart.
/// Non-empty by construction (the db/018 floor requires it) and legible: it records that
/// this link came from a John-Doe identification and names the vouching human.
pub fn compose_identify_link_provenance(human_kid: &str) -> String {
    format!("john-doe-identify linked-by:{human_kid}")
}

/// The optional prior-chart link inputs. Threaded explicitly (rather than read from
/// globals) so `identify_patient` stays a pure function of its arguments and is easy to
/// reason about. The link is signed AND attested by this human key.
pub struct LinkParams<'a> {
    pub prior: Uuid,
    pub human_sk: &'a SigningKey,
    pub human_kid: &'a str,
}

/// What `identify_patient` wrote: the identify event id, and the link event id when a link
/// was requested. Lets the CLI print an honest, specific confirmation.
pub struct IdentifyOutcome {
    pub identify_event_id: Uuid,
    pub link_event_id: Option<Uuid>,
}

/// Resolve a John-Doe chart: author the device-additive identify and, when `link` is given,
/// a human-attested link to a prior chart — in ONE transaction (atomic: never
/// confirmed-but-half-linked when a link was intended; if the link is refused, the identify
/// rolls back too and the chart stays *pending* to be retried).
///
/// HLC ticks (one per event) run before the transaction and self-commit; if the submit txn
/// rolls back the clock has merely advanced with no matching events, which is fine — the HLC
/// is monotonic and gaps are allowed (the same shape `register_john_doe` uses).
///
/// No cross-existence check on `patient`/`prior`: the offline-first floor (db/018/db/024)
/// does none — a pending marker or the prior chart may not have synced yet. The db/018 floor
/// rejects only a self-link (a == b) and an empty provenance.
pub async fn identify_patient(
    client: &mut tokio_postgres::Client,
    node_sk: &SigningKey,
    node_kid: &str,
    node_origin: &str,
    patient: Uuid,
    method: &str,
    link: Option<LinkParams<'_>>,
) -> anyhow::Result<IdentifyOutcome> {
    // 1. Build + sign the device-additive identify (node authors).
    let h1 = crate::db::next_hlc(client, node_origin).await?;
    let identify_event_id = Uuid::now_v7();
    let identify_body = build_identify_body(identify_event_id, patient, method, node_kid, h1);
    let identify_signed = sign(&identify_body, node_sk)?;

    // 2. If linking, build + sign + attest the human link BEFORE opening the txn.
    let link_prepared = match &link {
        Some(lp) => {
            let h2 = crate::db::next_hlc(client, node_origin).await?;
            let (low, high) = if patient <= lp.prior {
                (patient, lp.prior)
            } else {
                (lp.prior, patient)
            };
            let provenance = compose_identify_link_provenance(lp.human_kid);
            let link_event_id = Uuid::now_v7();
            // confidence: None — a human's direct assertion, not a matcher score.
            let link_body = build_attested_link_body(
                link_event_id,
                low,
                high,
                &provenance,
                None,
                lp.human_kid,
                h2,
            );
            let link_signed = sign(&link_body, lp.human_sk)?;
            let ca = event_address(&link_signed.signed_bytes);
            let token = sign_attestation(&ca, lp.human_kid, "attested", lp.human_sk)?;
            let attester_vk = lp.human_sk.verifying_key().to_bytes().to_vec();
            Some((link_event_id, link_signed.signed_bytes, token, attester_vk))
        }
        None => None,
    };

    // 3. One transaction: identify through the 1-arg door, link (if any) through the 3-arg
    //    door. Atomicity is the guarantee — a link rejection rolls the identify back too.
    let tx = client.transaction().await?;
    tx.execute("SELECT submit_event($1)", &[&identify_signed.signed_bytes])
        .await?;
    let link_event_id = match &link_prepared {
        Some((eid, bytes, token, vk)) => {
            tx.execute("SELECT submit_event($1,$2,$3)", &[bytes, token, vk])
                .await?;
            Some(*eid)
        }
        None => None,
    };
    tx.commit().await?;

    Ok(IdentifyOutcome {
        identify_event_id,
        link_event_id,
    })
}

/// Advisory CLI pre-check: does `attester_kid` resolve to a `kind='human'` actor? Human-ness
/// is read from the `actor_current` VIEW — mirroring the db/005 attestation gate itself (see
/// `db/005_submit.sql`'s `actor_current WHERE signing_key_id = ... AND kind = 'human'` check),
/// so a revoked/superseded human key is refused by BOTH this pre-check and the floor: a
/// faithful preview, not a confusing pass-then-refuse. (The ADR-0043 history-vs-current
/// discipline governs a DIFFERENT gate — the suppression-author check — and does not apply
/// here.)
///
/// This is a LEGIBILITY aid only: it lets the CLI reject a wrong `--attester-key` with a clear
/// message BEFORE authoring anything. The real, unbypassable enforcement is the db/005
/// attestation gate inside `submit_event` (defense in depth); never rely on this check for
/// safety — a raw-SQL client that skips it still cannot attest a link with a non-human key.
pub async fn attester_is_enrolled_human(
    client: &tokio_postgres::Client,
    attester_kid: &str,
) -> anyhow::Result<bool> {
    let ok: bool = client
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM actor_current \
             WHERE signing_key_id = $1 AND kind = 'human')",
            &[&attester_kid],
        )
        .await?
        .get(0);
    Ok(ok)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids() -> (Uuid, Uuid) {
        let eid = Uuid::parse_str("33333333-0000-0000-0000-000000000000").unwrap();
        let pid = Uuid::parse_str("00000000-0000-0000-0000-0000000000ab").unwrap();
        (eid, pid)
    }

    #[test]
    fn identify_body_is_device_additive_with_authored_twin() {
        let (eid, pid) = ids();
        let body = build_identify_body(
            eid,
            pid,
            "driver's licence + family confirmation",
            "nodekid",
            Hlc {
                wall: 7,
                counter: 0,
                node_origin: "n".into(),
            },
        );
        assert_eq!(body.event_type, "identity.identify.asserted");
        assert_eq!(body.patient_id, pid.to_string());
        assert_eq!(body.payload["subject"], pid.to_string());
        assert_eq!(
            body.payload["method"],
            "driver's licence + family confirmation"
        );
        assert!(
            body.payload.get("basis").is_none(),
            "an identify carries no basis"
        );
        // Device-additive: a `recorded` contributor with NO responsibility marker, so the
        // db/005 attestation gate demands nothing (matches john_doe.rs's registration events).
        let c = &body.contributors[0];
        assert_eq!(c["role"], "recorded");
        assert!(
            c.get("responsibility").is_none(),
            "identify demands no attestation"
        );
        // The db/024 floor HARD-requires a non-empty authored twin.
        assert!(!body.plaintext_twin.as_deref().unwrap().trim().is_empty());
    }

    #[test]
    fn link_provenance_is_nonempty_and_names_the_human() {
        let p = compose_identify_link_provenance("humankid");
        assert!(p.contains("humankid"));
        assert!(
            !p.trim().is_empty(),
            "the db/018 floor requires a non-empty provenance"
        );
    }
}
