//! The rung-3 audited crypto-shred ceremony (ADR-0005 / ADR-0052).
//!
//! WHY the tombstone is PLAINTEXT: "event X existed — destroyed, basis Z" must stay
//! legible FOREVER, including after the very DEK that once protected X's clinical
//! content is gone. If the tombstone itself were sealed, its own key could later rot
//! and the one durable record of an erasure decision would go dark exactly when it
//! matters most — that defeats the whole point of an AUDITED erasure ladder. So
//! `erasure.shred.asserted` is classified `additive` and deliberately NOT `clinical.*`
//! (db/037): it is exempt from the born-sealed floor and rides the plain
//! `submit_event` door, never `medication::sealed_submit`.
//!
//! WHY the heavy lifting lives in the door, not here: `cairn_execute_shred` (db/037)
//! runs INSIDE `submit_event`'s erasure arm (and inside the sync door's lenient
//! erasure leg), so a raw-SQL client that skips this module entirely still cannot
//! destroy custody without ALSO leaving a signed, legible tombstone behind — the audit
//! trail is unbypassable, not merely a UI convention (principle 12). This module's
//! whole job is: name the target, explain why (`basis`), sign, submit.
use anyhow::Context;
use cairn_event::{event_address, sign, sign_attestation, EventBody, Hlc, SigningKey};
use uuid::Uuid;

use crate::medication;

/// `schema_version` for the shred tombstone — mirrors the wire constant already pinned
/// by the db/037 floor and by the `shred_body` test helper in `tests/seal_apply.rs`.
const SHRED_SCHEMA_VERSION: &str = "erasure.shred/1";

/// Advisory Rust-side guard mirroring the DB floor: refuse a blank basis with a
/// clinical (not just technical) message before any key is unsealed or event authored.
/// `cairn_check_erasure_shred` (db/037) is the real, unbypassable enforcement — this
/// only makes the common mistake ("forgot --basis") fail fast and legibly.
pub fn validate_basis(basis: &str) -> anyhow::Result<()> {
    if basis.trim().is_empty() {
        anyhow::bail!(
            "--basis must not be empty: record WHY this event was destroyed — the ADR-0005 \
             audited \"why\" is what makes rung-3 erasure an AUDITED ceremony, not a silent \
             deletion"
        );
    }
    Ok(())
}

/// Assemble the `erasure.shred.asserted` tombstone `EventBody`. Pure: `event_id`/`hlc`
/// are supplied by the caller so this stays deterministic and unit-testable without a
/// DB. `target`/`basis` become the payload the db/037 floor demands
/// (`cairn_check_erasure_shred`); the authored twin restates both in prose so the
/// tombstone reads as a sentence even to a human who never sees the JSON.
///
/// `attester_kid` chooses which of two shapes this tombstone takes:
///   - `None` -> device-additive: the NODE signs, sole contributor `recorded`, no
///     `responsibility` marker (mirrors `identify.rs`'s device-additive shape and the
///     `shred_body` helper in `tests/seal_apply.rs` — the everyday, un-vouched path).
///   - `Some(human_kid)` -> the event is authored AND signed by the human — that is
///     what makes it "attested": sole contributor `attested` with
///     `responsibility: {held_by}` (mirrors
///     `apply_proposal::build_attested_link_body`), which trips the db/005 attestation
///     gate. A human is taking PERSONAL responsibility for the erasure decision
///     itself, not merely recording that it happened.
pub fn build_shred_body(
    event_id: Uuid,
    patient: Uuid,
    target: Uuid,
    basis: &str,
    node_kid: &str,
    attester_kid: Option<&str>,
    hlc: Hlc,
) -> EventBody {
    let signer_kid = attester_kid.unwrap_or(node_kid);
    let contributors = match attester_kid {
        Some(human_kid) => serde_json::json!([
            {"actor_id": human_kid, "role": "attested",
             "responsibility": {"held_by": human_kid}}
        ]),
        None => serde_json::json!([{"actor_id": node_kid, "role": "recorded"}]),
    };
    EventBody {
        event_id: event_id.to_string(),
        patient_id: patient.to_string(),
        event_type: "erasure.shred.asserted".into(),
        schema_version: SHRED_SCHEMA_VERSION.into(),
        hlc,
        t_effective: None,
        signer_key_id: signer_kid.into(),
        contributors,
        payload: serde_json::json!({
            "target_event_id": target.to_string(),
            "basis": basis,
        }),
        attachments: vec![],
        plaintext_twin: Some(format!("shredded event {target} — basis: {basis}")),
    }
}

/// Author, sign, and submit the audited crypto-shred tombstone for `target`.
///
/// 1. Reads `target`'s `patient_id` so the tombstone lands in the SAME chart as the
///    event it describes (an orphaned tombstone would be unfindable from the record it
///    is about). Refuses legibly — "nothing to shred" — when `target` is not present
///    locally; the strict door (`cairn_check_erasure_shred`/`cairn_execute_shred`,
///    db/037, wired through db/005's erasure arm) double-checks this itself, so this
///    pre-check is a legibility aid, not the enforcement (defense in depth — the same
///    discipline `resolve_attester` uses for `--attest-as`).
/// 2. Builds the tombstone body: device-additive when `attest` is `None`, or authored
///    and signed by the human when `attest` is `Some` (see `build_shred_body`).
/// 3. Signs and submits PLAINTEXT. `erasure.*` is exempt from the born-sealed floor
///    (db/037), so — unlike every `clinical.*` verb in `medication/` — there is no DEK
///    to seal, no unwrap key to register, and no `sealed_submit` door to go through:
///    a plain 1-arg (device-additive) or 3-arg (attested) `submit_event` call.
///
/// Returns the tombstone's own event id (NOT `target` — that event is untouched at the
/// wire level; only its custody and derived plaintext are scrubbed).
pub async fn shred_event(
    client: &mut tokio_postgres::Client,
    node_sk: &SigningKey,
    node_kid: &str,
    node_origin: &str,
    target: Uuid,
    basis: &str,
    attest: Option<&medication::AttestParams<'_>>,
) -> anyhow::Result<Uuid> {
    validate_basis(basis)?;

    // 1. The tombstone lands in the target's own chart — read patient_id off the
    //    target's event_log row. `query_opt` (not `query_one`) so an unknown target is
    //    a clean `None`, not a driver error, letting us give the legible refusal below.
    let target_s = target.to_string();
    let patient_s: Option<String> = client
        .query_opt(
            "SELECT patient_id::text FROM event_log WHERE event_id = $1::text::uuid",
            &[&target_s],
        )
        .await?
        .map(|row| row.get(0));
    let patient: Uuid = patient_s
        .ok_or_else(|| {
            anyhow::anyhow!(
                "nothing to shred: event {target} is not present on this node — sync it \
                 first, or double-check the event id"
            )
        })?
        .parse()
        .with_context(|| format!("event {target}'s patient_id is not a valid uuid"))?;

    // HLC + event id are minted up front (self-committing, like every other verb in
    // this crate): a rolled-back submit just leaves an HLC gap, which is allowed.
    let hlc = crate::db::next_hlc(client, node_origin).await?;
    let event_id = Uuid::now_v7();

    match attest {
        None => {
            // Device-additive: the node authors and signs; no attestation token needed
            // (no contributor claims `responsibility`), so the plain 1-arg door suffices.
            let body = build_shred_body(event_id, patient, target, basis, node_kid, None, hlc);
            let signed = sign(&body, node_sk)?;
            client
                .execute("SELECT submit_event($1)", &[&signed.signed_bytes])
                .await?;
        }
        Some(params) => {
            // Attested: the HUMAN authors and signs the tombstone itself (they are the
            // one taking responsibility for the erasure decision), and a matching
            // attestation token rides alongside so the db/005 gate can verify it.
            let body = build_shred_body(
                event_id,
                patient,
                target,
                basis,
                node_kid,
                Some(params.human_kid),
                hlc,
            );
            let signed = sign(&body, params.human_sk)?;
            let ca = event_address(&signed.signed_bytes);
            let token = sign_attestation(&ca, params.human_kid, "attested", params.human_sk)?;
            let attester_vk = params.human_sk.verifying_key().to_bytes().to_vec();
            client
                .execute(
                    "SELECT submit_event($1,$2,$3)",
                    &[&signed.signed_bytes, &token, &attester_vk],
                )
                .await?;
        }
    }
    Ok(event_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids() -> (Uuid, Uuid, Uuid) {
        let eid = Uuid::parse_str("55555555-0000-0000-0000-000000000000").unwrap();
        let pid = Uuid::parse_str("00000000-0000-0000-0000-0000000000cd").unwrap();
        let target = Uuid::parse_str("00000000-0000-0000-0000-0000000000ef").unwrap();
        (eid, pid, target)
    }

    fn hlc() -> Hlc {
        Hlc {
            wall: 9,
            counter: 0,
            node_origin: "n".into(),
        }
    }

    #[test]
    fn device_additive_body_carries_recorded_contributor_and_legible_twin() {
        let (eid, pid, target) = ids();
        let body = build_shred_body(
            eid,
            pid,
            target,
            "retention ceiling",
            "nodekid",
            None,
            hlc(),
        );
        assert_eq!(body.event_type, "erasure.shred.asserted");
        assert_eq!(body.schema_version, "erasure.shred/1");
        assert_eq!(body.signer_key_id, "nodekid");
        assert_eq!(body.payload["target_event_id"], target.to_string());
        assert_eq!(body.payload["basis"], "retention ceiling");
        let c = &body.contributors[0];
        assert_eq!(c["actor_id"], "nodekid");
        assert_eq!(c["role"], "recorded");
        assert!(
            c.get("responsibility").is_none(),
            "device-additive shred demands no attestation"
        );
        // The db/037 floor HARD-requires a non-empty authored twin naming target+basis.
        let twin = body.plaintext_twin.as_deref().unwrap();
        assert!(twin.contains(&target.to_string()));
        assert!(twin.contains("retention ceiling"));
    }

    #[test]
    fn attested_body_is_authored_and_signed_by_the_human() {
        let (eid, pid, target) = ids();
        let body = build_shred_body(
            eid,
            pid,
            target,
            "GDPR erasure request",
            "nodekid",
            Some("humankid"),
            hlc(),
        );
        // The human — not the node — is the signer: this is what makes it "attested".
        assert_eq!(body.signer_key_id, "humankid");
        let c = &body.contributors[0];
        assert_eq!(c["actor_id"], "humankid");
        assert_eq!(c["role"], "attested");
        assert!(
            c.get("responsibility").is_some(),
            "an attested shred must carry a responsibility marker to trip the db/005 gate"
        );
    }

    #[test]
    fn validate_basis_accepts_real_text_and_rejects_blank() {
        assert!(validate_basis("retention ceiling").is_ok());
        assert!(validate_basis("").is_err());
        assert!(validate_basis("   ").is_err());
    }
}
