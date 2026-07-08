//! §5.4 identity evidence — clinician-observed evidence about an unidentified patient that
//! is NOT a demographic field: a photograph today; distinguishing marks / belongings / EMS
//! pickup context as future `kind` values. A distinct, non-demographic event type
//! (`identity.evidence.asserted`) whose attachment (if any) rides the top-level
//! `EventBody.attachments`. The twin renders from the descriptor, never the bytes (§3.14).

use crate::attachment::{render_attachment_twin, Attachment};
use serde_json::{json, Value};

/// The event type for clinician-observed identity evidence. Non-demographic: the db/015
/// twin floor carries its authored twin verbatim (no floor branch needed).
pub const IDENTITY_EVIDENCE_EVENT_TYPE: &str = "identity.evidence.asserted";
/// schema_version for identity-evidence assertions.
pub const IDENTITY_EVIDENCE_SCHEMA_VERSION: &str = "identity.evidence.asserted/1";
/// The `kind` discriminator for a photograph (future kinds: mark, belongings, ems-context).
pub const PHOTO_EVIDENCE_KIND: &str = "photo";

/// Build the payload for a photo identity-evidence event: `{ kind, provenance, basis? }`.
/// The photograph itself is NOT in the payload — it rides the top-level
/// `EventBody.attachments` as an `Attachment` (ADR-0042); the payload is the clinical
/// framing. `basis` (how/why observed) is optional and omitted entirely when None
/// (principle 4: never manufacture a basis).
pub fn photo_evidence_body(basis: Option<&str>) -> Value {
    let mut body = json!({
        "kind": PHOTO_EVIDENCE_KIND,
        "provenance": crate::evidence::CLINICIAN_OBSERVED_PROVENANCE,
    });
    if let Some(b) = basis {
        body["basis"] = json!(b);
    }
    body
}

/// Render the authored §3.13/§4.5 twin for an identity-evidence event: names the kind,
/// then the attachment's own descriptor-derived twin (never the bytes), then the basis if
/// stated. This is a pure mechanical derivation (ADR-0039 pattern) the db/015 floor carries
/// verbatim for this non-demographic type.
pub fn render_identity_evidence_twin(kind: &str, basis: Option<&str>, attachment: &Attachment) -> String {
    let mut out = format!("identity evidence ({kind}): {}", render_attachment_twin(attachment));
    if let Some(b) = basis {
        out.push_str(&format!(" — {b}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attachment::Rendition;

    #[test]
    fn photo_body_is_a_clinician_observed_photo_kind_with_optional_basis() {
        let with = photo_evidence_body(Some("photographed on arrival for identification"));
        assert_eq!(with["kind"], "photo");
        assert_eq!(with["provenance"], "clinician-observed");
        assert_eq!(with["basis"], "photographed on arrival for identification");

        let without = photo_evidence_body(None);
        assert_eq!(without["kind"], "photo");
        assert!(without.get("basis").is_none(), "absent basis is omitted, never null");
    }

    #[test]
    fn twin_is_legible_from_descriptor_and_names_the_kind_never_bytes() {
        let r = Rendition::reference("original", b"PIXELS", "image/jpeg");
        let att = Attachment::single("frontal face photograph", r);
        let twin = render_identity_evidence_twin("photo", Some("on arrival"), &att);
        assert!(twin.contains("photo"), "twin names the kind: {twin}");
        assert!(twin.contains("frontal face photograph"), "descriptor: {twin}");
        assert!(twin.contains("image/jpeg"));
        assert!(twin.contains("on arrival"), "basis when present: {twin}");
        assert!(!twin.contains("PIXELS"));
        assert!(!twin.trim().is_empty());
    }

    #[test]
    fn twin_omits_the_basis_clause_when_absent() {
        let r = Rendition::reference("original", b"x", "image/jpeg");
        let att = Attachment::single("photo", r);
        let twin = render_identity_evidence_twin("photo", None, &att);
        assert!(!twin.contains(" — "), "no trailing basis separator when basis is None: {twin}");
    }
}
