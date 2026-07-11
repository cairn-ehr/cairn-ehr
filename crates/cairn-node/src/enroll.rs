//! §5.4 — the human-actor enrollment ceremony. A clinician's signing key becomes a
//! `kind='human'` actor carrying an ADR-0044 person-distinguishing determinant, so it may
//! sign+attest the optional `identify --link` (finisher 3). Reuses the `enroll_actor` in-DB
//! floor (`db/004`); this module only shapes the pinned set (pure) and guards the one way a
//! second enrollment could corrupt attribution (the async orchestrator, Task 2).

use anyhow::bail;
use serde_json::{json, Value};

/// Outcome of `enroll_human_actor`: a fresh enrollment vs an idempotent no-op (the same key,
/// same determinant, already enrolled as a human — re-runnable provisioning).
pub enum EnrollHumanOutcome {
    Enrolled,
    AlreadyEnrolled,
}

/// Assemble the pinned determinant set for a human actor. The `actor_id` is the content-address
/// of THIS set (`cairn_actor_id`, in-DB), so it is what keeps two clinicians distinct — and it
/// must NEVER include the signing key, because `rotate-key` (ADR-0011 §5) keeps `actor_id` stable
/// across a key change.
///
/// ADR-0044 §2 requires a person-distinguishing determinant but deliberately does not fix WHICH
/// field (ADR-0011 keeps pinned-set contents as policy). This node's convention: a professional
/// `registration_id` (preferred — the real-world unique person id, ADR-0033) and/or a node-local
/// `handle`. At least one MUST be present: a determinant is a real field, never fabricated
/// (principle 4), and we do not lean on the floor's loud collision-refusal as the only guard.
/// Blank inputs are treated as absent.
pub fn build_human_pinned(
    role: &str,
    registration_id: Option<&str>,
    handle: Option<&str>,
) -> anyhow::Result<Value> {
    // Trim + drop blanks so `--handle "  "` cannot masquerade as a determinant. A nested `fn`
    // (rather than a closure) is used here because it gets a fully generic elided lifetime,
    // letting it clean both `registration_id` and `handle` even though they borrow from
    // independent lifetimes — a closure would get pinned to a single concrete lifetime.
    fn clean(s: Option<&str>) -> Option<&str> {
        s.map(str::trim).filter(|s| !s.is_empty())
    }
    let role = role.trim();
    if role.is_empty() {
        bail!("enroll-human: --role must not be blank");
    }
    let registration_id = clean(registration_id);
    let handle = clean(handle);
    if registration_id.is_none() && handle.is_none() {
        bail!(
            "enroll-human: a person-distinguishing determinant is required — supply \
             --registration-id (a professional licence/registration number) and/or --handle. \
             Without one, two clinicians would compute the same actor_id (ADR-0044)."
        );
    }
    let mut obj = serde_json::Map::new();
    obj.insert("role".into(), json!(role));
    if let Some(r) = registration_id {
        obj.insert("registration_id".into(), json!(r));
    }
    if let Some(h) = handle {
        obj.insert("handle".into(), json!(h));
    }
    Ok(Value::Object(obj))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_registration_id_and_role_never_the_key() {
        let p = build_human_pinned("clinician", Some("AHPRA-MED0001234567"), None).unwrap();
        assert_eq!(p["role"], json!("clinician"));
        assert_eq!(p["registration_id"], json!("AHPRA-MED0001234567"));
        assert!(p.get("handle").is_none(), "absent handle is omitted");
        assert!(
            p.as_object()
                .unwrap()
                .keys()
                .all(|k| k != "signing_key" && k != "key"),
            "the signing key must never enter the pinned set (rotate-key stability, ADR-0044)"
        );
    }

    #[test]
    fn pins_handle_when_no_registration_id() {
        let p = build_human_pinned("clinician", None, Some("dr-a")).unwrap();
        assert_eq!(p["handle"], json!("dr-a"));
        assert!(p.get("registration_id").is_none());
    }

    #[test]
    fn pins_both_determinants_when_supplied() {
        let p = build_human_pinned("registrar", Some("MED123"), Some("dr-b")).unwrap();
        assert_eq!(p["registration_id"], json!("MED123"));
        assert_eq!(p["handle"], json!("dr-b"));
    }

    #[test]
    fn refuses_when_no_determinant() {
        let err = build_human_pinned("clinician", None, None).unwrap_err();
        assert!(err
            .to_string()
            .contains("person-distinguishing determinant"));
    }

    #[test]
    fn treats_blank_determinant_as_absent() {
        assert!(build_human_pinned("clinician", Some("   "), Some("")).is_err());
    }

    #[test]
    fn refuses_blank_role() {
        assert!(build_human_pinned("  ", Some("MED123"), None).is_err());
    }
}
