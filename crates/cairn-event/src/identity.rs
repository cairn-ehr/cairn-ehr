//! Identity linkage assertion builders (spec §5.1/§5.7 — matcher piece C1). Pure:
//! explicit inputs, no I/O, no DB. The safety-critical structural floor and the
//! connected-component projection live in the database (db/018); these functions
//! only shape and render the event a node will sign. Mirrors `demographics.rs`.

use serde_json::{json, Value};

/// One §5.7 link/unlink assertion between two immortal patient UUIDs. `subject_a`
/// and `subject_b` are the two UUIDs whose linkage is asserted; the event_type
/// (link vs unlink) — not the payload — carries the direction. The in-DB floor
/// (db/018) rejects a self-link (a == b) and an empty provenance.
pub struct LinkAssertion<'a> {
    pub subject_a: &'a str,  // §5.7 — one immortal subject UUID (string form)
    pub subject_b: &'a str,  // §5.7 — the other immortal subject UUID
    pub provenance: &'a str, // §4.1 provenance ladder — required-present, value-open
    pub confidence: Option<&'a str>, // acknowledged uncertainty (principle 4); omitted when None
}

/// Shared payload shape for link and unlink (identical; the event_type distinguishes
/// them). `confidence` is omitted entirely when absent — never serialized as null —
/// so the in-DB floor's key-presence checks see exactly what the author asserted.
fn assertion_body(a: &LinkAssertion) -> Value {
    let mut p = json!({
        "subject_a": a.subject_a,
        "subject_b": a.subject_b,
        "provenance": a.provenance,
    });
    if let Some(c) = a.confidence {
        p.as_object_mut()
            .expect("json! built an object")
            .insert("confidence".into(), json!(c));
    }
    p
}

/// Build the `identity.link.asserted` payload (the value of `EventBody.payload`).
pub fn link_assertion_body(a: &LinkAssertion) -> Value {
    assertion_body(a)
}

/// Build the `identity.unlink.asserted` payload — same shape as a link.
pub fn unlink_assertion_body(a: &LinkAssertion) -> Value {
    assertion_body(a)
}

/// Render the §4.5-style legibility twin for a link: profile-independent plaintext.
pub fn render_link_twin(a: &LinkAssertion) -> String {
    format!("link: {} ↔ {} ({})", a.subject_a, a.subject_b, a.provenance)
}

/// Render the §4.5-style legibility twin for an unlink.
pub fn render_unlink_twin(a: &LinkAssertion) -> String {
    format!("unlink: {} ↔ {} ({})", a.subject_a, a.subject_b, a.provenance)
}

// ---------------------------------------------------------------------------
// §5.7 dispute (piece C3). A dispute is the patient-initiated "I was never there"
// front door (§5.5(b) identity theft): it flags a chart *under-review* and enters a
// triage worklist, and a later resolution clears it. A dispute has its OWN identity
// (`dispute_id`) because one chart can carry several concurrent, independently-
// resolvable disputes; the in-DB projection (db/023) overlays the standing state
// (open → resolved) by HLC per `dispute_id`, the same shape C1 uses per link edge.
//
// Both the open and the resolve carry `subject` so each assertion is self-describing:
// offline-first sync can deliver a resolution BEFORE the open it closes, and the
// overlay must still know which chart the dispute is about (see the db/023 design).
// A dispute is ADDITIVE (it annotates trust; it never erases, moves, or blocks
// anything), so it flows through the existing submit_event door like a link.
// ---------------------------------------------------------------------------

/// One §5.7 `identity.dispute.asserted` — opens a dispute against a chart.
pub struct DisputeAssertion<'a> {
    pub dispute_id: &'a str, // §5.7 — this dispute's own immortal id (string uuid)
    pub subject: &'a str,    // the patient UUID whose chart is under dispute
    pub reason: &'a str,     // §4.1 — required-present, value-open ("unknown" is honest, "" is not)
}

/// One §5.7 `identity.dispute.resolved` — closes a specific standing dispute.
pub struct DisputeResolution<'a> {
    pub dispute_id: &'a str, // the dispute being closed (matches an opened dispute_id)
    pub subject: &'a str,    // carried on the resolve too, so out-of-order arrival still binds the chart
    pub resolution: &'a str, // §4.1 — required-present, value-open (the adjudication outcome)
}

/// Build the `identity.dispute.asserted` payload. All three fields are mandatory —
/// no omit-when-absent discipline is needed here (unlike link's optional confidence).
pub fn dispute_assertion_body(d: &DisputeAssertion) -> Value {
    json!({
        "dispute_id": d.dispute_id,
        "subject": d.subject,
        "reason": d.reason,
    })
}

/// Build the `identity.dispute.resolved` payload — same key discipline, but the
/// descriptive field is `resolution` (the adjudication outcome) rather than `reason`.
pub fn dispute_resolution_body(d: &DisputeResolution) -> Value {
    json!({
        "dispute_id": d.dispute_id,
        "subject": d.subject,
        "resolution": d.resolution,
    })
}

/// Render the §4.5-style legibility twin for an opened dispute: profile-independent
/// plaintext that stays legible without the schema (the identity events are legible-
/// critical, so the db/023 floor HARD-requires a non-empty authored twin).
pub fn render_dispute_twin(d: &DisputeAssertion) -> String {
    format!("dispute opened: {} — {} (dispute {})", d.subject, d.reason, d.dispute_id)
}

/// Render the §4.5-style legibility twin for a resolved dispute.
pub fn render_dispute_resolved_twin(d: &DisputeResolution) -> String {
    format!("dispute resolved: {} — {} (dispute {})", d.subject, d.resolution, d.dispute_id)
}

// ---------------------------------------------------------------------------
// §5.4/§5.7 identity-pending + `identify` (piece C4). These two events open and
// close the *unconfirmed* chart trust state — the third state of the §5.7 contract
// C3 built (confirmed / unconfirmed / under-review).
//
// `identity.pending.asserted` marks a chart identity-pending (the §5.4 "John Doe"
// front door: an unconscious/unknown patient gets a UUID immediately, care proceeds,
// and the chart renders in *unconfirmed* trust mode). `identity.identify.asserted`
// establishes who the patient is — "who, method" (§5.7) — flipping the chart to
// *confirmed*; linking to a PRIOR chart, when one exists, is a separate ordinary
// `link` assertion (C1), so identify records only this chart's own identity + method.
//
// Deliberate contrast with `dispute`: identity-pending is a SINGLE per-chart lifecycle
// state (a chart is pending or not), so the in-DB overlay (db/024) keys on the SUBJECT
// itself — there is no separate id and no subject-consistency guard. The standing state
// (pending -> identified, and a later pending re-opening it) overlays by HLC per subject,
// converging under out-of-order sync exactly like the dispute open/resolve pair.
//
// Both are ADDITIVE (they annotate the trust state; they never erase, move, or block
// anything), so they flow through the existing submit_event door with the same low
// ceremony as a link. §5.7's "Human; method recorded": `method` is structurally required
// (below + the db/024 floor), and the "human vouches" requirement composes via the
// existing attestation gate when a responsibility-bearing contributor is named.
// ---------------------------------------------------------------------------

/// One §5.7 `identity.pending.asserted` — marks a chart identity-pending (unconfirmed).
pub struct PendingAssertion<'a> {
    pub subject: &'a str, // the patient UUID registered identity-pending (string uuid)
    pub basis: &'a str,   // §4.1 — required-present, value-open ("unconscious ED arrival, no ID", "unknown")
}

/// One §5.7 `identity.identify.asserted` — establishes identity → confirmed.
pub struct IdentifyAssertion<'a> {
    pub subject: &'a str, // the patient UUID now identified
    pub method: &'a str,  // §5.7 "method recorded" — required-present, value-open ("driver's licence", ...)
}

/// Build the `identity.pending.asserted` payload. Both fields are mandatory — no
/// omit-when-absent discipline (unlike link's optional confidence).
pub fn pending_assertion_body(a: &PendingAssertion) -> Value {
    json!({
        "subject": a.subject,
        "basis": a.basis,
    })
}

/// Build the `identity.identify.asserted` payload — same key discipline, but the
/// descriptive field is `method` (how identity was established) rather than `basis`.
pub fn identify_assertion_body(a: &IdentifyAssertion) -> Value {
    json!({
        "subject": a.subject,
        "method": a.method,
    })
}

/// Render the §4.5-style legibility twin for an identity-pending marker: profile-
/// independent plaintext (the db/024 floor HARD-requires a non-empty authored twin,
/// like every other identity event).
pub fn render_pending_twin(a: &PendingAssertion) -> String {
    format!("identity pending: {} — {}", a.subject, a.basis)
}

/// Render the §4.5-style legibility twin for an `identify` event.
pub fn render_identify_twin(a: &IdentifyAssertion) -> String {
    format!("identity confirmed: {} via {}", a.subject, a.method)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> LinkAssertion<'static> {
        LinkAssertion {
            subject_a: "aaaaaaaa-0000-0000-0000-000000000001",
            subject_b: "bbbbbbbb-0000-0000-0000-000000000002",
            provenance: "matcher:cfg@hash",
            confidence: None,
        }
    }

    #[test]
    fn body_has_subjects_and_provenance() {
        let b = link_assertion_body(&sample());
        assert_eq!(b["subject_a"], "aaaaaaaa-0000-0000-0000-000000000001");
        assert_eq!(b["subject_b"], "bbbbbbbb-0000-0000-0000-000000000002");
        assert_eq!(b["provenance"], "matcher:cfg@hash");
    }

    #[test]
    fn confidence_omitted_when_absent_never_null() {
        let b = link_assertion_body(&sample());
        assert!(
            b.get("confidence").is_none(),
            "confidence must be omitted entirely when absent, never serialized as null"
        );
    }

    #[test]
    fn confidence_present_when_given() {
        let a = LinkAssertion { confidence: Some("0.91"), ..sample() };
        let b = link_assertion_body(&a);
        assert_eq!(b["confidence"], "0.91");
    }

    #[test]
    fn link_and_unlink_bodies_are_identical() {
        assert_eq!(link_assertion_body(&sample()), unlink_assertion_body(&sample()));
    }

    #[test]
    fn twins_distinguish_link_from_unlink() {
        assert!(render_link_twin(&sample()).starts_with("link: "));
        assert!(render_unlink_twin(&sample()).starts_with("unlink: "));
        assert!(render_link_twin(&sample()).contains("matcher:cfg@hash"));
    }

    // --- §5.7 dispute builders (C3) ---

    fn sample_dispute() -> DisputeAssertion<'static> {
        DisputeAssertion {
            dispute_id: "dddddddd-0000-0000-0000-000000000009",
            subject: "aaaaaaaa-0000-0000-0000-000000000001",
            reason: "patient states never attended in March",
        }
    }

    fn sample_resolution() -> DisputeResolution<'static> {
        DisputeResolution {
            dispute_id: "dddddddd-0000-0000-0000-000000000009",
            subject: "aaaaaaaa-0000-0000-0000-000000000001",
            resolution: "dismissed — no supporting evidence",
        }
    }

    #[test]
    fn dispute_body_carries_id_subject_and_reason() {
        let b = dispute_assertion_body(&sample_dispute());
        assert_eq!(b["dispute_id"], "dddddddd-0000-0000-0000-000000000009");
        assert_eq!(b["subject"], "aaaaaaaa-0000-0000-0000-000000000001");
        assert_eq!(b["reason"], "patient states never attended in March");
    }

    #[test]
    fn resolution_body_carries_id_subject_and_resolution() {
        let b = dispute_resolution_body(&sample_resolution());
        assert_eq!(b["dispute_id"], "dddddddd-0000-0000-0000-000000000009");
        assert_eq!(b["subject"], "aaaaaaaa-0000-0000-0000-000000000001");
        assert_eq!(b["resolution"], "dismissed — no supporting evidence");
        // A resolution is not an open assertion: it must NOT carry a `reason` key.
        assert!(b.get("reason").is_none());
    }

    #[test]
    fn dispute_twins_distinguish_open_from_resolved_and_carry_subject_and_id() {
        let opened = render_dispute_twin(&sample_dispute());
        assert!(opened.starts_with("dispute opened: "));
        assert!(opened.contains("aaaaaaaa-0000-0000-0000-000000000001")); // subject
        assert!(opened.contains("dddddddd-0000-0000-0000-000000000009")); // dispute_id
        assert!(opened.contains("never attended")); // reason

        let resolved = render_dispute_resolved_twin(&sample_resolution());
        assert!(resolved.starts_with("dispute resolved: "));
        assert!(resolved.contains("dddddddd-0000-0000-0000-000000000009"));
        assert!(resolved.contains("dismissed")); // resolution
    }

    // --- §5.4/§5.7 identity-pending + identify builders (C4) ---

    fn sample_pending() -> PendingAssertion<'static> {
        PendingAssertion {
            subject: "aaaaaaaa-0000-0000-0000-000000000001",
            basis: "unconscious ED arrival, no ID",
        }
    }

    fn sample_identify() -> IdentifyAssertion<'static> {
        IdentifyAssertion {
            subject: "aaaaaaaa-0000-0000-0000-000000000001",
            method: "driver's licence + family confirmation",
        }
    }

    #[test]
    fn pending_body_carries_subject_and_basis_only() {
        let b = pending_assertion_body(&sample_pending());
        assert_eq!(b["subject"], "aaaaaaaa-0000-0000-0000-000000000001");
        assert_eq!(b["basis"], "unconscious ED arrival, no ID");
        // A pending marker is not an identify: it must NOT carry a `method` key.
        assert!(b.get("method").is_none());
    }

    #[test]
    fn identify_body_carries_subject_and_method_only() {
        let b = identify_assertion_body(&sample_identify());
        assert_eq!(b["subject"], "aaaaaaaa-0000-0000-0000-000000000001");
        assert_eq!(b["method"], "driver's licence + family confirmation");
        // An identify is not a pending marker: it must NOT carry a `basis` key.
        assert!(b.get("basis").is_none());
    }

    #[test]
    fn identity_state_twins_distinguish_pending_from_confirmed() {
        let pending = render_pending_twin(&sample_pending());
        assert!(pending.starts_with("identity pending: "));
        assert!(pending.contains("aaaaaaaa-0000-0000-0000-000000000001")); // subject
        assert!(pending.contains("unconscious ED arrival")); // basis

        let confirmed = render_identify_twin(&sample_identify());
        assert!(confirmed.starts_with("identity confirmed: "));
        assert!(confirmed.contains("aaaaaaaa-0000-0000-0000-000000000001")); // subject
        assert!(confirmed.contains("driver's licence")); // method
    }
}
