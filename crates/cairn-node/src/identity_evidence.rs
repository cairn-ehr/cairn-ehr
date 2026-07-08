//! §5.4 identity-evidence author path. Two disjoint evidence shapes share one CLI command:
//! a content-addressed photograph (the bytes ride `EventBody.attachments` — authored by
//! `photo_evidence.rs`) and a free-text observation (marks / belongings / EMS-context, whose
//! observation IS the payload text — authored here in ONE `submit_event` call, no blob). This
//! module owns the shared `route_identity_evidence` gate that decides which shape a `--kind`
//! selects, plus the TEXT path's pure helpers (unit-tested here) and its async orchestrator
//! (e2e-tested in tests/identity_evidence_text.rs).

use cairn_event::identity_evidence::{
    parse_text_evidence_kind, render_text_evidence_twin, text_evidence_body,
    IDENTITY_EVIDENCE_EVENT_TYPE, IDENTITY_EVIDENCE_SCHEMA_VERSION, PHOTO_EVIDENCE_KIND,
};
use cairn_event::{sign, EventBody, Hlc, SigningKey};
use std::path::PathBuf;
use tokio_postgres::Client;
use uuid::Uuid;

/// The validated shape of an `assert-identity-evidence` invocation: a `--kind` resolved to
/// exactly the flags that kind needs. §5.4 identity evidence comes in two disjoint shapes —
/// a content-addressed photograph (bytes) or a free-text observation (mark/belongings/
/// ems-context) — and one CLI command carries both. Resolving the flag combination in ONE pure,
/// tested place keeps the "--file iff --kind photo" rule off the CLI match arm and lets a future
/// UI backend reuse the same gate instead of re-deriving it.
pub enum EvidenceRoute {
    /// `--kind photo`: a content-addressed image at `file` (`media_type`), described by `descriptor`.
    Photo {
        file: PathBuf,
        media_type: String,
        descriptor: String,
        basis: Option<String>,
    },
    /// A text kind (mark/belongings/ems-context) with a free-text `description`; `kind` is the
    /// canonical `&'static str` (already run through `parse_text_evidence_kind`).
    Text {
        kind: &'static str,
        description: String,
        basis: Option<String>,
    },
}

/// PURE: resolve a raw `assert-identity-evidence` invocation into the one evidence shape its
/// `--kind` selects, rejecting any mismatched flag combination BEFORE any DB or file I/O. Photo
/// and text carry disjoint flags, so this is the single gate that stops them crossing:
/// `--kind photo` needs `--file`/`--media-type`/`--descriptor` and forbids `--description`; a
/// text kind needs `--description` and forbids the photo flags; anything else is an unknown kind.
/// (Content checks — non-empty descriptor/description — stay in the library floors downstream,
/// the single source of truth for principle 4; this gate is only about flag *presence*/pairing.)
pub fn route_identity_evidence(
    kind: &str,
    file: Option<PathBuf>,
    media_type: Option<String>,
    descriptor: Option<String>,
    description: Option<String>,
    basis: Option<String>,
) -> anyhow::Result<EvidenceRoute> {
    if kind == PHOTO_EVIDENCE_KIND {
        if description.is_some() {
            anyhow::bail!("--description is for text kinds; --kind photo describes the image with --descriptor");
        }
        let file = file.ok_or_else(|| anyhow::anyhow!("--kind photo requires --file"))?;
        let media_type =
            media_type.ok_or_else(|| anyhow::anyhow!("--kind photo requires --media-type"))?;
        let descriptor =
            descriptor.ok_or_else(|| anyhow::anyhow!("--kind photo requires --descriptor"))?;
        return Ok(EvidenceRoute::Photo {
            file,
            media_type,
            descriptor,
            basis,
        });
    }
    if let Some(canonical) = parse_text_evidence_kind(kind) {
        if file.is_some() || media_type.is_some() || descriptor.is_some() {
            anyhow::bail!("--file/--media-type/--descriptor are for --kind photo; a text kind uses --description");
        }
        let description =
            description.ok_or_else(|| anyhow::anyhow!("--kind {kind} requires --description"))?;
        return Ok(EvidenceRoute::Text {
            kind: canonical,
            description,
            basis,
        });
    }
    anyhow::bail!("unknown --kind {kind:?}; expected photo|mark|belongings|ems-context")
}

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

/// Author a TEXT identity-evidence event on an existing chart. Validates the kind then the
/// description FIRST so a bad call never reaches the DB (single source of truth for both checks;
/// same discriminator-first order as `route_identity_evidence`). No blob tier and a single
/// statement, so no explicit transaction is needed — `submit_event` is itself atomic. Ticks the
/// HLC once (self-committing, like `john_doe.rs`/`photo_evidence.rs`). Returns the new event id.
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
    let canonical_kind = parse_text_evidence_kind(kind).ok_or_else(|| {
        anyhow::anyhow!(
            "unknown identity-evidence kind {kind:?}; expected one of mark|belongings|ems-context"
        )
    })?;
    validate_description(description)?;

    let hlc = crate::db::next_hlc(client, node_origin).await?;
    let event_id = Uuid::now_v7();
    let body = build_text_evidence_body(
        event_id,
        patient_id,
        kid,
        hlc,
        canonical_kind,
        description,
        basis,
    );
    let signed = sign(&body, sk)?;

    client
        .execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await?;
    Ok(event_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_event::identity_evidence::MARK_EVIDENCE_KIND;

    fn hlc() -> Hlc {
        Hlc {
            wall: 7,
            counter: 0,
            node_origin: "n".into(),
        }
    }

    #[test]
    fn route_photo_requires_its_flags_and_rejects_the_text_flag() {
        // Happy path: all photo flags present, no --description → Photo route carrying them.
        let r = route_identity_evidence(
            "photo",
            Some(PathBuf::from("f.jpg")),
            Some("image/jpeg".into()),
            Some("frontal face".into()),
            None,
            Some("on arrival".into()),
        )
        .unwrap();
        match r {
            EvidenceRoute::Photo {
                file,
                media_type,
                descriptor,
                basis,
            } => {
                assert_eq!(file, PathBuf::from("f.jpg"));
                assert_eq!(media_type, "image/jpeg");
                assert_eq!(descriptor, "frontal face");
                assert_eq!(basis.as_deref(), Some("on arrival"));
            }
            _ => panic!("expected a photo route"),
        }
        // Each required photo flag missing → refused (no partial photo authored).
        assert!(
            route_identity_evidence(
                "photo",
                None,
                Some("image/jpeg".into()),
                Some("d".into()),
                None,
                None
            )
            .is_err(),
            "missing --file"
        );
        assert!(
            route_identity_evidence(
                "photo",
                Some(PathBuf::from("f")),
                None,
                Some("d".into()),
                None,
                None
            )
            .is_err(),
            "missing --media-type"
        );
        assert!(
            route_identity_evidence(
                "photo",
                Some(PathBuf::from("f")),
                Some("image/jpeg".into()),
                None,
                None,
                None
            )
            .is_err(),
            "missing --descriptor"
        );
        // --description on a photo is a crossed-shape error.
        assert!(route_identity_evidence(
            "photo",
            Some(PathBuf::from("f")),
            Some("image/jpeg".into()),
            Some("d".into()),
            Some("oops".into()),
            None
        )
        .is_err());
    }

    #[test]
    fn route_text_kind_requires_description_and_rejects_photo_flags() {
        // Happy path: only --description → Text route with the canonical kind.
        let r = route_identity_evidence(
            "mark",
            None,
            None,
            None,
            Some("scar on left forearm".into()),
            None,
        )
        .unwrap();
        match r {
            EvidenceRoute::Text {
                kind, description, ..
            } => {
                assert_eq!(kind, MARK_EVIDENCE_KIND, "canonical kind");
                assert_eq!(description, "scar on left forearm");
            }
            _ => panic!("expected a text route"),
        }
        // The other two text kinds also route as Text.
        assert!(matches!(
            route_identity_evidence("belongings", None, None, None, Some("wallet".into()), None)
                .unwrap(),
            EvidenceRoute::Text { .. }
        ));
        assert!(matches!(
            route_identity_evidence(
                "ems-context",
                None,
                None,
                None,
                Some("bus stop".into()),
                None
            )
            .unwrap(),
            EvidenceRoute::Text { .. }
        ));
        // Missing --description → refused.
        assert!(
            route_identity_evidence("mark", None, None, None, None, None).is_err(),
            "missing --description"
        );
        // Any photo flag on a text kind → crossed-shape error.
        assert!(
            route_identity_evidence(
                "mark",
                Some(PathBuf::from("f")),
                None,
                None,
                Some("d".into()),
                None
            )
            .is_err(),
            "--file on text kind"
        );
        assert!(
            route_identity_evidence(
                "mark",
                None,
                Some("image/jpeg".into()),
                None,
                Some("d".into()),
                None
            )
            .is_err(),
            "--media-type on text kind"
        );
        assert!(
            route_identity_evidence(
                "mark",
                None,
                None,
                Some("desc".into()),
                Some("d".into()),
                None
            )
            .is_err(),
            "--descriptor on text kind"
        );
    }

    #[test]
    fn route_rejects_unknown_and_miscased_kinds() {
        assert!(
            route_identity_evidence("scar", None, None, None, Some("d".into()), None).is_err(),
            "unknown kind"
        );
        assert!(
            route_identity_evidence(
                "Photo",
                Some(PathBuf::from("f")),
                Some("image/jpeg".into()),
                Some("d".into()),
                None,
                None
            )
            .is_err(),
            "case-sensitive"
        );
    }

    #[test]
    fn validate_description_refuses_empty_and_whitespace_only() {
        // The honest-content floor lives in the library, so a caller bypassing the CLI
        // (a UI backend) still cannot author an evidence assertion that says nothing.
        assert!(validate_description("").is_err(), "empty refused");
        assert!(
            validate_description("   \t\n").is_err(),
            "whitespace-only refused"
        );
        assert!(
            validate_description("scar on left forearm").is_ok(),
            "real description accepted"
        );
    }

    #[test]
    fn body_is_the_identity_evidence_event_with_text_payload_and_empty_attachments() {
        let pid = Uuid::now_v7();
        let eid = Uuid::now_v7();
        let body = build_text_evidence_body(
            eid,
            pid,
            "kid",
            hlc(),
            "mark",
            "scar on left forearm ~5cm",
            Some("primary survey"),
        );

        assert_eq!(body.event_type, IDENTITY_EVIDENCE_EVENT_TYPE);
        assert_eq!(body.schema_version, IDENTITY_EVIDENCE_SCHEMA_VERSION);
        assert_eq!(body.patient_id, pid.to_string());
        assert_eq!(body.payload["kind"], "mark");
        assert_eq!(body.payload["description"], "scar on left forearm ~5cm");
        assert_eq!(body.payload["provenance"], "clinician-observed");
        assert_eq!(body.payload["basis"], "primary survey");
        // No attachment for a text kind — the empty vec preserves content-address identity.
        assert!(
            body.attachments.is_empty(),
            "text evidence carries no attachment"
        );
        // additive event → recorded role, no attestation demanded
        assert_eq!(body.contributors[0]["role"], "recorded");
        // authored, legible twin naming the kind and description
        let twin = body.plaintext_twin.as_deref().unwrap();
        assert_eq!(
            twin,
            &render_text_evidence_twin("mark", "scar on left forearm ~5cm", Some("primary survey"))
        );
        assert!(twin.contains("scar on left forearm"));
    }

    #[test]
    fn body_omits_basis_and_still_renders_a_twin_when_basis_absent() {
        let body = build_text_evidence_body(
            Uuid::now_v7(),
            Uuid::now_v7(),
            "kid",
            hlc(),
            "belongings",
            "blue wallet, €40, keys",
            None,
        );
        assert!(
            body.payload.get("basis").is_none(),
            "absent basis omitted, never null"
        );
        assert!(body
            .plaintext_twin
            .as_deref()
            .unwrap()
            .contains("blue wallet"));
    }
}
