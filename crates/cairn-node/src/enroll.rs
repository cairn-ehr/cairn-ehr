//! §5.4 — the human-actor enrollment ceremony. A clinician's signing key becomes a
//! `kind='human'` actor carrying an ADR-0044 person-distinguishing determinant, so it may
//! sign+attest the optional `identify --link` (finisher 3). Reuses the `enroll_actor` in-DB
//! floor (`db/004`); this module only shapes the pinned set (pure) and guards the one way a
//! second enrollment could corrupt attribution (the async orchestrator, Task 2).

use anyhow::{bail, Context};
use serde_json::{json, Value};

/// Outcome of `enroll_human_actor`: a fresh enrollment vs an idempotent no-op (the same key,
/// same determinant, already enrolled as a human — re-runnable provisioning).
///
/// `Debug` is required so `Result<EnrollHumanOutcome, _>::unwrap_err()` compiles in tests (the
/// panic message on the non-error branch prints the `Ok` value).
#[derive(Debug)]
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

/// Enroll `kid` as a `kind='human'` actor with `pinned`, reusing the in-DB `enroll_actor`
/// floor (`db/004`). Two guards wrap the floor call:
///
/// 1. **Dual-mapping guard.** `submit_event` resolves a signer to an actor purely by
///    `signing_key_id`; if one key maps to MORE than one `actor_current` row it sets
///    `actor_id = NULL` for EVERY event that key authors node-wide (`db/005`,
///    `array_length(v_actor_ids,1)=1`) — a silent, irreversible attribution loss. So a key
///    already bound to an actor must not be re-bound: we refuse, EXCEPT the idempotent case
///    (same key, same `actor_id`, already a human) which is a re-runnable no-op.
/// 2. **Advisory ADR-0044 collision pre-check.** `cairn_actor_id_key_conflict` tells us up front
///    whether this determinant set already identifies another actor under a different key, so we
///    can name the remedy (add a distinguishing determinant) instead of surfacing a raw floor
///    error. The floor re-checks regardless — this is legibility, not the enforcement (the same
///    advisory-mirrors-the-floor pattern as `attester_is_enrolled_human`).
pub async fn enroll_human_actor(
    db: &tokio_postgres::Client,
    kid: &str,
    pinned: &Value,
) -> anyhow::Result<EnrollHumanOutcome> {
    let pinned_str = pinned.to_string();
    // The actor_id this determinant set will compute to (content-address of the pinned set).
    let new_actor_id: Vec<u8> = db
        .query_one("SELECT cairn_actor_id($1::text::jsonb)", &[&pinned_str])
        .await
        .context("computing cairn_actor_id for the human pinned set")?
        .get(0);

    // Guard 1 — is this key already an actor?
    let rows = db
        .query(
            "SELECT actor_id, kind FROM actor_current WHERE signing_key_id = $1",
            &[&kid],
        )
        .await?;
    if !rows.is_empty() {
        let idempotent = rows.len() == 1
            && rows[0].get::<_, Vec<u8>>(0) == new_actor_id
            && rows[0].get::<_, String>(1) == "human";
        if idempotent {
            return Ok(EnrollHumanOutcome::AlreadyEnrolled);
        }
        bail!(
            "enroll-human: key {kid} is already enrolled as an actor; enrolling it again would \
             map one key to two actors and silently NULL its authorship node-wide (db/005). \
             Use a fresh key for this human."
        );
    }

    // Guard 2 — advisory determinant-collision hint (the floor is the real enforcement).
    let conflict: bool = db
        .query_one(
            "SELECT cairn_actor_id_key_conflict($1, $2)",
            &[&new_actor_id, &kid],
        )
        .await?
        .get(0);
    if conflict {
        bail!(
            "enroll-human: this determinant set already identifies another actor under a \
             different key — two people must not share one actor_id (ADR-0044). Add a \
             distinguishing --registration-id or --handle."
        );
    }

    // The real floor. Re-checks the collision itself and fails closed if a concurrent enroll
    // slipped in between guard 2 and here.
    db.execute(
        "SELECT enroll_actor('human', $1::text::jsonb, $2)",
        &[&pinned_str, &kid],
    )
    .await
    .context("enroll_actor('human', …) refused the enrollment")?;
    Ok(EnrollHumanOutcome::Enrolled)
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
