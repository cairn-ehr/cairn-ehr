//! Medication reconciliation / separation builders (slice 3). Pure: shapes only
//! payload JSON, mirroring `identity.rs` link/unlink. Two immortal `medication_id`
//! threads are asserted to be (reconciliation) or to NOT be (separation) the same
//! real drug; the event_type — not the payload — carries the direction. The
//! in-DB floor (db/033) rejects a self-reconcile (a == b) and an empty provenance.
use serde_json::{json, Value};

/// One reconciliation/separation assertion between two immortal `medication_id`
/// threads. `reason` is omitted entirely when absent (never serialized as null),
/// so the floor's key-presence checks see exactly what the author asserted.
pub struct ReconciliationAssertion<'a> {
    pub subject_a: &'a str,  // a medication_id thread (string uuid)
    pub subject_b: &'a str,  // the other medication_id thread, distinct from subject_a
    pub provenance: &'a str, // §4.1 ladder — required-present, value-open ("clinician-judgment")
    pub reason: Option<&'a str>, // free-text ("brand vs generic"); omitted when None
}

/// Shared payload shape for reconciliation and separation (identical; the
/// event_type distinguishes them).
fn assertion_body(a: &ReconciliationAssertion) -> Value {
    let mut p = json!({
        "subject_a": a.subject_a,
        "subject_b": a.subject_b,
        "provenance": a.provenance,
    });
    if let Some(r) = a.reason {
        p.as_object_mut()
            .expect("json! built an object")
            .insert("reason".into(), json!(r));
    }
    p
}

/// Build the `clinical.medication-reconciliation.asserted` payload.
pub fn reconciliation_body(a: &ReconciliationAssertion) -> Value {
    assertion_body(a)
}

/// Build the `clinical.medication-separation.asserted` payload — same shape.
pub fn separation_body(a: &ReconciliationAssertion) -> Value {
    assertion_body(a)
}

/// The §3.13 legibility twin for a reconciliation. Always non-empty.
pub fn render_reconciliation_twin(a: &ReconciliationAssertion) -> String {
    let mut s = format!(
        "Reconciled as the same medication: {} ↔ {} ({})",
        a.subject_a, a.subject_b, a.provenance
    );
    if let Some(r) = a.reason {
        s.push_str(&format!(" — {r}"));
    }
    s
}

/// The §3.13 legibility twin for a separation (the never-erase reversal).
pub fn render_separation_twin(a: &ReconciliationAssertion) -> String {
    let mut s = format!(
        "Separated as distinct medications: {} ↔ {} ({})",
        a.subject_a, a.subject_b, a.provenance
    );
    if let Some(r) = a.reason {
        s.push_str(&format!(" — {r}"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ReconciliationAssertion<'static> {
        ReconciliationAssertion {
            subject_a: "11111111-1111-7111-8111-111111111111",
            subject_b: "22222222-2222-7222-8222-222222222222",
            provenance: "clinician-judgment",
            reason: Some("brand vs generic"),
        }
    }

    #[test]
    fn body_carries_subjects_provenance_reason() {
        let v = reconciliation_body(&sample());
        assert_eq!(v["subject_a"], "11111111-1111-7111-8111-111111111111");
        assert_eq!(v["subject_b"], "22222222-2222-7222-8222-222222222222");
        assert_eq!(v["provenance"], "clinician-judgment");
        assert_eq!(v["reason"], "brand vs generic");
    }

    #[test]
    fn reason_omitted_when_absent_not_null() {
        let a = ReconciliationAssertion { reason: None, ..sample() };
        let v = separation_body(&a);
        assert!(
            !v.as_object().unwrap().contains_key("reason"),
            "absent reason omitted entirely, not null"
        );
        assert_eq!(v["subject_a"], "11111111-1111-7111-8111-111111111111");
    }

    #[test]
    fn separation_payload_matches_reconciliation_shape() {
        // The two verbs carry an identical body; only event_type differs.
        assert_eq!(reconciliation_body(&sample()), separation_body(&sample()));
    }

    #[test]
    fn twins_are_nonempty_and_read_naturally() {
        let r = render_reconciliation_twin(&sample());
        assert!(r.contains("Reconciled"));
        assert!(r.contains("brand vs generic"));
        assert!(!r.trim().is_empty());
        let s = render_separation_twin(&sample());
        assert!(s.contains("Separated"));
        assert!(!s.trim().is_empty());
        // non-empty even with no reason
        let bare = ReconciliationAssertion { reason: None, ..sample() };
        assert!(!render_reconciliation_twin(&bare).trim().is_empty());
    }
}
