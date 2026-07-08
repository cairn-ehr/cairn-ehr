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

/// The `kind` discriminator for a distinguishing bodily mark (scar, tattoo, amputation, ...).
pub const MARK_EVIDENCE_KIND: &str = "mark";
/// The `kind` discriminator for personal belongings found on the patient.
pub const BELONGINGS_EVIDENCE_KIND: &str = "belongings";
/// The `kind` discriminator for the EMS pickup context (where/how the patient was found).
pub const EMS_CONTEXT_EVIDENCE_KIND: &str = "ems-context";

/// The closed set of text-shaped evidence kinds, in one place so the parser below and any
/// future reader share a single source of truth (a test pins it to the three constants).
pub const TEXT_EVIDENCE_KINDS: [&str; 3] = [
    MARK_EVIDENCE_KIND,
    BELONGINGS_EVIDENCE_KIND,
    EMS_CONTEXT_EVIDENCE_KIND,
];

/// Map an input kind string to its canonical `&'static str`, returning `None` for anything
/// outside the closed set. The set is closed at the AUTHOR edge — a typo-drift guard so the
/// event log does not accumulate a mess of near-synonym kinds over time — NOT on the wire: the
/// event type stays additively open per ADR-0012. Returns `Option` (not `anyhow::Result`)
/// because `cairn-event` is dependency-light; the node/CLI layer supplies the error framing.
pub fn parse_text_evidence_kind(kind: &str) -> Option<&'static str> {
    TEXT_EVIDENCE_KINDS.into_iter().find(|k| *k == kind)
}

/// Build the payload for a TEXT identity-evidence event: `{ kind, provenance, description,
/// basis? }`. There is no attachment — the observation IS the `description` text (compare the
/// photo path, where the bytes ride `EventBody.attachments` and the descriptor lives on the
/// attachment). `basis` (how/why observed; for ems-context, the relayed source) is optional and
/// omitted entirely when None (principle 4: never manufacture a basis).
pub fn text_evidence_body(kind: &str, description: &str, basis: Option<&str>) -> Value {
    let mut body = json!({
        "kind": kind,
        "provenance": crate::evidence::CLINICIAN_OBSERVED_PROVENANCE,
        "description": description,
    });
    if let Some(b) = basis {
        body["basis"] = json!(b);
    }
    body
}

/// Render the authored §3.13/§4.5 twin for a TEXT identity-evidence event: names the kind, then
/// the observed description directly (no attachment to defer to), then the basis if stated. A
/// pure mechanical derivation (ADR-0039 pattern) the db/015 floor carries verbatim.
pub fn render_text_evidence_twin(kind: &str, description: &str, basis: Option<&str>) -> String {
    let mut out = format!("identity evidence ({kind}): {description}");
    if let Some(b) = basis {
        out.push_str(&format!(" — {b}"));
    }
    out
}

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
pub fn render_identity_evidence_twin(
    kind: &str,
    basis: Option<&str>,
    attachment: &Attachment,
) -> String {
    let mut out = format!(
        "identity evidence ({kind}): {}",
        render_attachment_twin(attachment)
    );
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
        assert!(
            without.get("basis").is_none(),
            "absent basis is omitted, never null"
        );
    }

    #[test]
    fn twin_is_legible_from_descriptor_and_names_the_kind_never_bytes() {
        let r = Rendition::reference("original", b"PIXELS", "image/jpeg");
        let att = Attachment::single("frontal face photograph", r);
        let twin = render_identity_evidence_twin("photo", Some("on arrival"), &att);
        assert!(twin.contains("photo"), "twin names the kind: {twin}");
        assert!(
            twin.contains("frontal face photograph"),
            "descriptor: {twin}"
        );
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
        assert!(
            !twin.contains(" — "),
            "no trailing basis separator when basis is None: {twin}"
        );
    }

    #[test]
    fn text_evidence_kinds_parse_to_canonical_constants_and_reject_unknowns() {
        assert_eq!(parse_text_evidence_kind("mark"), Some(MARK_EVIDENCE_KIND));
        assert_eq!(
            parse_text_evidence_kind("belongings"),
            Some(BELONGINGS_EVIDENCE_KIND)
        );
        assert_eq!(
            parse_text_evidence_kind("ems-context"),
            Some(EMS_CONTEXT_EVIDENCE_KIND)
        );
        assert_eq!(
            parse_text_evidence_kind("photo"),
            None,
            "photo is the attachment path, not a text kind"
        );
        assert_eq!(
            parse_text_evidence_kind("Mark"),
            None,
            "case-sensitive: canonical spelling only"
        );
        assert_eq!(parse_text_evidence_kind(""), None);
        // The closed set and the constants cannot drift apart.
        assert_eq!(
            TEXT_EVIDENCE_KINDS,
            [
                MARK_EVIDENCE_KIND,
                BELONGINGS_EVIDENCE_KIND,
                EMS_CONTEXT_EVIDENCE_KIND
            ]
        );
    }

    #[test]
    fn text_body_carries_kind_provenance_description_and_optional_basis() {
        let with = text_evidence_body("mark", "scar on left forearm ~5cm", Some("primary survey"));
        assert_eq!(with["kind"], "mark");
        assert_eq!(with["provenance"], "clinician-observed");
        assert_eq!(with["description"], "scar on left forearm ~5cm");
        assert_eq!(with["basis"], "primary survey");

        let without = text_evidence_body("belongings", "blue wallet, €40, keys", None);
        assert_eq!(without["kind"], "belongings");
        assert_eq!(without["description"], "blue wallet, €40, keys");
        assert!(
            without.get("basis").is_none(),
            "absent basis is omitted, never null"
        );
    }

    #[test]
    fn text_twin_is_legible_names_the_kind_and_appends_basis_only_when_present() {
        let with = render_text_evidence_twin(
            "ems-context",
            "found unconscious at bus stop",
            Some("reported by paramedic"),
        );
        assert!(with.contains("ems-context"), "twin names the kind: {with}");
        assert!(
            with.contains("found unconscious at bus stop"),
            "description: {with}"
        );
        assert!(
            with.contains("reported by paramedic"),
            "basis when present: {with}"
        );
        assert!(with.contains(" — "), "basis is set off by an em-dash");

        let without =
            render_text_evidence_twin("mark", "tattoo of an anchor, right shoulder", None);
        assert!(without.contains("tattoo of an anchor"), "{without}");
        assert!(
            !without.contains(" — "),
            "no trailing basis separator when basis is None: {without}"
        );
        assert!(!without.trim().is_empty());
    }
}
