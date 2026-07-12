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
///
/// `role` is pinned too, so it is part of `actor_id` BY DESIGN: the actor is the **(entity, role)**
/// pair, not the bare person. One clinician deliberately holds several role-actors (e.g. `clinician`
/// and `registrar`) — each a distinct `actor_id` under its own signing key — because authorship and
/// attestation attach to what someone did *in a role*, so the person↔actor relationship is 1:many.
/// Those role-actors are recognised as one underlying person by their **shared `registration_id`**
/// (the entity anchor carried in every one of them). A consequence, NOT a bug: a mistyped `--role`
/// mints an *unintended* role-actor rather than being rejected — at enroll time it is
/// indistinguishable from a genuine new role — but it is never a silent *merge*, stays linkable via
/// the shared `registration_id`, and is correctable later by supersede/rotate (ADR-0011 §5). Making
/// that entity→role-actor (1:many) relationship first-class is a tracked design thread (issue #168).
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

/// Advisory pre-mint collision check for the CLI's fresh-key path: does `pinned` already compute
/// to an `actor_id` that some existing `actor_event` row claims?
///
/// This exists so the `enroll-human` handler can refuse a doomed ceremony BEFORE minting a key
/// (and, on the sealed path, printing a shown-once recovery code) — a brand-new random key can
/// never be the idempotent case, so if the determinant's `actor_id` is already claimed it must
/// belong to a DIFFERENT, already-enrolled key, and `enroll_human_actor`'s guard 2 would reject
/// the ceremony a moment later anyway. Checking here avoids minting a stray key + recovery code.
///
/// On that fresh-key path this is exactly `cairn_actor_id_key_conflict(cairn_actor_id(pinned), kid)`
/// with a never-seen `kid` (which `IS DISTINCT FROM` every stored `signing_key_id`), so it reduces
/// to "does ANY actor_event carry this actor_id". It is ADVISORY only — `enroll_human_actor` and the
/// in-DB floor remain the real, unbypassable enforcement (the same advisory-mirrors-the-floor
/// pattern as guard 2). Split out of the handler so the DB-gated tests exercise it directly, not
/// only through the binary.
pub async fn determinant_already_claimed(
    db: &tokio_postgres::Client,
    pinned: &Value,
) -> anyhow::Result<bool> {
    let claimed: bool = db
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM actor_event WHERE actor_id = \
             cairn_actor_id($1::text::jsonb))",
            &[&pinned.to_string()],
        )
        .await
        .context("pre-mint determinant-collision check")?
        .get(0);
    Ok(claimed)
}

/// Enroll `kid` as a `kind='human'` actor with `pinned`, reusing the in-DB `enroll_actor`
/// floor (`db/004`). Two guards wrap the floor call:
///
/// 1. **Dual-mapping shortcut (advisory).** `submit_event` resolves a signer to an actor
///    purely by `signing_key_id`; if one key maps to MORE than one `actor_current` row it
///    sets `actor_id = NULL` for EVERY event that key authors node-wide (`db/005`). Since
///    issue #166 the `enroll_actor` FLOOR enforces "one key binds at most one actor_id"
///    race-safely (`cairn_key_actor_id_conflict` + a per-key advisory lock), so this check
///    is no longer the enforcement — it is advisory legibility plus the idempotent
///    shortcut: same key, same `actor_id`, already human is a re-runnable no-op we can
///    answer without a floor round-trip (and without inserting a duplicate `enroll` row).
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

    // Guard 1 — is this key already an actor? The db/004 FLOOR is the enforcement now
    // (issue #166): enroll_actor refuses a fresh enroll of an already-bound key under a
    // different actor_id, serialized by a per-key advisory lock, so the concurrent
    // same-key/different-actor race is closed at the floor. This read is advisory: it lets
    // us return the idempotent AlreadyEnrolled outcome (same key, same actor, already human)
    // without a floor round-trip or a duplicate enroll row, and surface a friendly message
    // for the non-idempotent case before the floor's own RAISE — the same
    // advisory-mirrors-the-floor pattern as Guard 2 (attester_is_enrolled_human).
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
             map one key to two actors and silently NULL its authorship node-wide (db/005). A \
             genuinely different person needs a fresh key; if this is the SAME human's own \
             determinant changing, that is not a re-enroll — it is a future supersede/rotate \
             operation (ADR-0011 §5), which has no door yet."
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
            "enroll-human: this determinant set is already claimed — either it identifies \
             another actor under a different key (two people must not share one actor_id, \
             ADR-0044), or it belongs to a retired/revoked actor_id that a fresh enroll must not \
             resurrect (db/004). If this is genuinely a new person, add a distinguishing \
             --registration-id or --handle."
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
