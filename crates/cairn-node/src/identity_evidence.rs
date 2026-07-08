//! §5.4 TEXT identity-evidence author path (marks / belongings / EMS-context). The non-photo
//! sibling of `photo_evidence.rs`: the observation is free text in the payload, so there is no
//! blob and no attachment — the whole author path is ONE `submit_event` call. Splits into pure
//! helpers (unit-tested here) and an async orchestrator (e2e-tested in
//! tests/identity_evidence_text.rs).

use cairn_event::identity_evidence::{
    parse_text_evidence_kind, render_text_evidence_twin, text_evidence_body,
    IDENTITY_EVIDENCE_EVENT_TYPE, IDENTITY_EVIDENCE_SCHEMA_VERSION,
};
use cairn_event::{sign, EventBody, Hlc, SigningKey};
use tokio_postgres::Client;
use uuid::Uuid;

/// PURE: enforce the honest-content requirement (§5.4 / principle 4) — an evidence assertion
/// must say what was observed, so an empty or whitespace-only description is refused. This lives
/// in the library (not only the CLI edge) so EVERY caller — a future UI backend included —
/// inherits the guarantee. (Whether a UI silently supplies a default is soft policy above this
/// floor, per principle 12; the floor's job is only to refuse a genuinely empty claim.)
pub fn validate_description(description: &str) -> anyhow::Result<()> {
    if description.trim().is_empty() {
        anyhow::bail!("identity-evidence description must be non-empty (§5.4/principle 4: say what was observed)");
    }
    Ok(())
}

/// PURE: assemble the signed `EventBody` for a TEXT identity-evidence event. `kind` must already
/// be a canonical constant (the caller validates via `parse_text_evidence_kind`). The payload
/// carries the clinical framing (kind/provenance/description/basis); the twin is authored from
/// the same text; `attachments` stays empty (there is nothing to attach). The sole contributor
/// is the recording actor with role `recorded` — additive, no attestation.
pub fn build_text_evidence_body(
    event_id: Uuid,
    patient_id: Uuid,
    kid: &str,
    hlc: Hlc,
    kind: &str,
    description: &str,
    basis: Option<&str>,
) -> EventBody {
    let twin = render_text_evidence_twin(kind, description, basis);
    EventBody {
        event_id: event_id.to_string(),
        patient_id: patient_id.to_string(),
        event_type: IDENTITY_EVIDENCE_EVENT_TYPE.into(),
        schema_version: IDENTITY_EVIDENCE_SCHEMA_VERSION.into(),
        hlc,
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: text_evidence_body(kind, description, basis),
        attachments: vec![],
        plaintext_twin: Some(twin),
    }
}

/// Author a TEXT identity-evidence event on an existing chart. Validates the description and the
/// kind FIRST so a bad call never reaches the DB (single source of truth for both checks). No
/// blob tier and a single statement, so no explicit transaction is needed — `submit_event` is
/// itself atomic. Ticks the HLC once (self-committing, like `john_doe.rs`/`photo_evidence.rs`).
/// Returns the new event id.
#[allow(clippy::too_many_arguments)] // signer + node context + the kind/description/basis inputs
pub async fn assert_text_evidence(
    client: &Client,
    sk: &SigningKey,
    kid: &str,
    node_origin: &str,
    patient_id: Uuid,
    kind: &str,
    description: &str,
    basis: Option<&str>,
) -> anyhow::Result<Uuid> {
    validate_description(description)?;
    let canonical_kind = parse_text_evidence_kind(kind)
        .ok_or_else(|| anyhow::anyhow!("unknown identity-evidence kind {kind:?}; expected one of mark|belongings|ems-context"))?;

    let hlc = crate::db::next_hlc(client, node_origin).await?;
    let event_id = Uuid::now_v7();
    let body = build_text_evidence_body(event_id, patient_id, kid, hlc, canonical_kind, description, basis);
    let signed = sign(&body, sk)?;

    client.execute("SELECT submit_event($1)", &[&signed.signed_bytes]).await?;
    Ok(event_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hlc() -> Hlc { Hlc { wall: 7, counter: 0, node_origin: "n".into() } }

    #[test]
    fn validate_description_refuses_empty_and_whitespace_only() {
        // The honest-content floor lives in the library, so a caller bypassing the CLI
        // (a UI backend) still cannot author an evidence assertion that says nothing.
        assert!(validate_description("").is_err(), "empty refused");
        assert!(validate_description("   \t\n").is_err(), "whitespace-only refused");
        assert!(validate_description("scar on left forearm").is_ok(), "real description accepted");
    }

    #[test]
    fn body_is_the_identity_evidence_event_with_text_payload_and_empty_attachments() {
        let pid = Uuid::now_v7();
        let eid = Uuid::now_v7();
        let body = build_text_evidence_body(
            eid, pid, "kid", hlc(), "mark", "scar on left forearm ~5cm", Some("primary survey"));

        assert_eq!(body.event_type, IDENTITY_EVIDENCE_EVENT_TYPE);
        assert_eq!(body.schema_version, IDENTITY_EVIDENCE_SCHEMA_VERSION);
        assert_eq!(body.patient_id, pid.to_string());
        assert_eq!(body.payload["kind"], "mark");
        assert_eq!(body.payload["description"], "scar on left forearm ~5cm");
        assert_eq!(body.payload["provenance"], "clinician-observed");
        assert_eq!(body.payload["basis"], "primary survey");
        // No attachment for a text kind — the empty vec preserves content-address identity.
        assert!(body.attachments.is_empty(), "text evidence carries no attachment");
        // additive event → recorded role, no attestation demanded
        assert_eq!(body.contributors[0]["role"], "recorded");
        // authored, legible twin naming the kind and description
        let twin = body.plaintext_twin.as_deref().unwrap();
        assert_eq!(twin, &render_text_evidence_twin("mark", "scar on left forearm ~5cm", Some("primary survey")));
        assert!(twin.contains("scar on left forearm"));
    }

    #[test]
    fn body_omits_basis_and_still_renders_a_twin_when_basis_absent() {
        let body = build_text_evidence_body(
            Uuid::now_v7(), Uuid::now_v7(), "kid", hlc(), "belongings", "blue wallet, €40, keys", None);
        assert!(body.payload.get("basis").is_none(), "absent basis omitted, never null");
        assert!(body.plaintext_twin.as_deref().unwrap().contains("blue wallet"));
    }
}
