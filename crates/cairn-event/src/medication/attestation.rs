//! Medication attestation builder (slice 4). Pure: shapes only payload JSON.
//! A human takes clinical responsibility for one `medication_id` thread, pinning
//! a convergent commitment of the thread's content-event set it reviewed (so a
//! later change flips the vouch stale). Mirrors `reconciliation.rs`. The db/034
//! floor rejects a malformed medication_id / commitment; the db/005 gate enforces
//! the human attestation (the responsibility-bearing contributor is added by the
//! cairn-node author path, not here).
use serde_json::{json, Value};

/// One attestation of one `medication_id` thread. `reviewed_commitment` is a hex
/// digest of the reviewed content-event set (see `cairn_medication_thread_commitment`,
/// db/034); `reviewed_count` is a legibility hint only. `basis`/`note` are omitted
/// entirely when absent (never serialized as null) so an added-later field never
/// changes an existing event's content address (principle 11, demographics idiom).
pub struct MedicationAttestation<'a> {
    pub medication_id: &'a str,
    pub reviewed_commitment: &'a str,
    pub reviewed_count: u32,
    pub basis: Option<&'a str>,
    pub note: Option<&'a str>,
}

/// Build the `clinical.medication-attestation.asserted` payload.
pub fn medication_attestation_body(a: &MedicationAttestation) -> Value {
    let mut p = json!({
        "medication_id": a.medication_id,
        "reviewed_commitment": a.reviewed_commitment,
        "reviewed_count": a.reviewed_count,
    });
    let obj = p.as_object_mut().expect("json! built an object");
    if let Some(b) = a.basis {
        obj.insert("basis".into(), json!(b));
    }
    if let Some(n) = a.note {
        obj.insert("note".into(), json!(n));
    }
    p
}

/// The §3.13 legibility twin. Always non-empty.
pub fn render_medication_attestation_twin(a: &MedicationAttestation) -> String {
    let mut s = format!(
        "Reviewed & vouched for medication thread {} ({} entries)",
        a.medication_id, a.reviewed_count
    );
    if let Some(b) = a.basis {
        s.push_str(&format!(" — {b}"));
    }
    if let Some(n) = a.note {
        s.push_str(&format!(" [{n}]"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> MedicationAttestation<'static> {
        MedicationAttestation {
            medication_id: "11111111-1111-7111-8111-111111111111",
            reviewed_commitment: "abcdef0123456789",
            reviewed_count: 4,
            basis: Some("admission reconciliation"),
            note: None,
        }
    }

    #[test]
    fn body_carries_id_commitment_count() {
        let v = medication_attestation_body(&sample());
        assert_eq!(v["medication_id"], "11111111-1111-7111-8111-111111111111");
        assert_eq!(v["reviewed_commitment"], "abcdef0123456789");
        assert_eq!(v["reviewed_count"], 4);
        assert_eq!(v["basis"], "admission reconciliation");
    }

    #[test]
    fn basis_and_note_omitted_when_absent_not_null() {
        let a = MedicationAttestation {
            basis: None,
            note: None,
            ..sample()
        };
        let v = medication_attestation_body(&a);
        let o = v.as_object().unwrap();
        assert!(!o.contains_key("basis"), "absent basis omitted, not null");
        assert!(!o.contains_key("note"), "absent note omitted, not null");
        assert_eq!(v["reviewed_count"], 4);
    }

    #[test]
    fn twin_is_nonempty_and_reads_naturally() {
        let s = render_medication_attestation_twin(&sample());
        assert!(s.contains("Reviewed"));
        assert!(s.contains("4 entries"));
        assert!(s.contains("admission reconciliation"));
        assert!(!s.trim().is_empty());
        // non-empty even with no basis/note
        let bare = MedicationAttestation {
            basis: None,
            note: None,
            ..sample()
        };
        assert!(!render_medication_attestation_twin(&bare).trim().is_empty());
    }

    #[test]
    fn twin_surfaces_note_when_present() {
        let a = MedicationAttestation {
            note: Some("verified with pharmacy"),
            ..sample()
        };
        let s = render_medication_attestation_twin(&a);
        assert!(
            s.contains("verified with pharmacy"),
            "note must surface, got: {s}"
        );
    }
}
