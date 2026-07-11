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

use cairn_event::identity::{identify_assertion_body, render_identify_twin, IdentifyAssertion};
use cairn_event::{EventBody, Hlc};
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
