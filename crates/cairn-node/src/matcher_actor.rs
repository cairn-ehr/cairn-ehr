//! The per-epoch matcher actor (§7.5 / ADR-0029). Each distinct `matcher_version`
//! (already `"{pkg}+{weights-digest}"`, ADR-0014's config-pin) is its OWN `agent`
//! actor with its OWN signing key. A fresh key per epoch gives UNIQUE key->actor
//! attribution, so `submit_event` stamps `event_log.actor_id` precisely and a
//! contamination-cascade recall (db/006 `events_by_actor_epoch`) selects exactly one
//! config's auto-links.
//!
//! The epoch is carried in the pinned set under the ADR-0029 field name `skill_epoch`,
//! because that is the field `events_by_actor_epoch(p_key, p_epoch)` matches on
//! (`pinned ->> 'skill_epoch' = p_epoch`). So the matcher's skill epoch IS its
//! `matcher_version`.
//!
//! Pure determinants first; the IO `resolve_matcher_actor` (load-or-generate key +
//! idempotent enroll) follows.

use crate::keystore;
use cairn_event::SigningKey;
use serde_json::{json, Value};
use std::path::Path;
use tokio_postgres::Client;

/// The ADR-0029 pinned determinant set for a matcher epoch. `matcher_version` is carried
/// under `skill_epoch` (the recall key, db/006). Deterministic: same version -> byte-
/// identical pinned set -> same actor_id (`cairn_actor_id`, in-DB) on every node.
pub fn matcher_pinned(matcher_version: &str) -> Value {
    json!({ "kind": "agent", "actor": "cairn-matcher", "skill_epoch": matcher_version })
}

/// A filesystem-safe, collision-free filename for a per-epoch key. `matcher_version`
/// carries punctuation (`.` and `+`, plus `/`/`=` for a base64 weights digest), so the
/// old "map every non-alphanumeric byte to `_`" scheme was NOT injective: `0.3.0+abc`
/// and `0_3_0+abc` collapsed to one file, silently loading the WRONG epoch's key — and
/// hiding one epoch's auto-links under another actor, which defeats the db/006
/// contamination-cascade recall this whole module exists to make precise. We instead
/// hex-encode the version: hex is `[0-9a-f]` (filesystem-safe, no `/`, no `..`) and
/// injective, so distinct versions ALWAYS get distinct files.
pub fn matcher_key_filename(matcher_version: &str) -> String {
    format!("matcher_{}.key", hex::encode(matcher_version.as_bytes()))
}

/// Resolve (load-or-create) the per-epoch matcher signing key AND ensure its `agent`
/// actor is enrolled. Idempotent and owner-privileged: the caller connects as a role that
/// may run `enroll_actor` — the runtime `cairn_agent` role deliberately cannot, per the
/// db/004 trust-anchor floor.
///
/// Key at rest: sealed under `secret` when present (passed as BOTH seal recipients, so one
/// operational passphrase both seals and unseals — a matcher key needs no separate paper
/// recovery escrow because it is regenerable: losing it only retires the epoch). When
/// `secret` is None (throwaway/test nodes) the key is written plaintext 0600.
///
/// Returns `(signing_key, kid_hex)` where `kid_hex = hex(verifying_key)` — the
/// `signing_key_id` the actor is enrolled under and the events are signed by.
pub async fn resolve_matcher_actor(
    client: &Client,
    keystore_dir: &Path,
    secret: Option<&str>,
    matcher_version: &str,
) -> anyhow::Result<(SigningKey, String)> {
    std::fs::create_dir_all(keystore_dir)?;
    let path = keystore_dir.join(matcher_key_filename(matcher_version));

    // 1. Load the key if the epoch already has one; else mint + persist a fresh key.
    let (sk, kid) = if path.exists() {
        let sk = keystore::load(&path, secret)?;
        let kid = hex::encode(sk.verifying_key().to_bytes());
        (sk, kid)
    } else if let Some(s) = secret {
        // Seal under the op passphrase (both recipients = the same secret).
        keystore::generate_sealed(&path, s, s)?
    } else {
        keystore::generate_plaintext(&path)?
    };

    // 2. Decide enroll vs reuse vs REFUSE from the registry history, NOT from a bare
    //    actor_current check. actor_current holds only non-revoked identities, so a naive
    //    "enroll when absent" would RE-ENROLL a deliberately revoked epoch: the fresh enroll
    //    outranks the earlier revoke in the actor_current view (db/004 orders on
    //    (recorded_at, seq)), silently RESURRECTING a recalled matcher and re-authorising
    //    its auto-links. The point of the per-epoch actor is that a contamination-cascade
    //    recall STAYS recalled, so the three cases are:
    //      - live in actor_current       -> already enrolled and current: reuse, no write.
    //      - ever enrolled, not current  -> revoked/superseded: REFUSE (never resurrect).
    //      - never enrolled              -> first sight: enroll.
    let live: bool = client
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM actor_current WHERE signing_key_id = $1 AND kind = 'agent')",
            &[&kid],
        )
        .await?
        .get(0);
    if !live {
        let ever_enrolled: bool = client
            .query_one(
                "SELECT EXISTS(SELECT 1 FROM actor_event \
                 WHERE signing_key_id = $1 AND kind = 'agent' AND op = 'enroll')",
                &[&kid],
            )
            .await?
            .get(0);
        if ever_enrolled {
            anyhow::bail!(
                "matcher epoch '{matcher_version}' (key {kid}) was enrolled and is no longer \
                 current (revoked or superseded) — refusing to re-enroll and resurrect a \
                 recalled matcher actor"
            );
        }
        // tokio-postgres in this crate has no serde_json ToSql; pass the pinned set as a
        // text string and cast with `$1::jsonb` (the project convention — see the enroll
        // call in tests/apply_proposal.rs).
        let pinned = matcher_pinned(matcher_version).to_string();
        client
            .execute(
                "SELECT enroll_actor('agent', $1::text::jsonb, $2)",
                &[&pinned, &kid],
            )
            .await?;
    }

    Ok((sk, kid))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_is_deterministic_agent_with_skill_epoch() {
        let p = matcher_pinned("0.3.0+abc123");
        assert_eq!(p["kind"], "agent");
        assert_eq!(p["actor"], "cairn-matcher");
        // The epoch is carried under skill_epoch (the db/006 recall key), = matcher_version.
        assert_eq!(p["skill_epoch"], "0.3.0+abc123");
        assert_eq!(p, matcher_pinned("0.3.0+abc123"));
    }

    #[test]
    fn distinct_versions_give_distinct_pinned_sets() {
        assert_ne!(matcher_pinned("0.3.0+aaa"), matcher_pinned("0.3.0+bbb"));
    }

    #[test]
    fn key_filename_is_safe_and_distinct() {
        let f = matcher_key_filename("0.3.0+abc123");
        assert!(f.starts_with("matcher_") && f.ends_with(".key"));
        assert!(!f.contains('/') && !f.contains(".."));
        // Hex-encoded body: only [0-9a-f]; the single '.' is the extension.
        assert_eq!(f, format!("matcher_{}.key", hex::encode("0.3.0+abc123")));
        assert_ne!(
            matcher_key_filename("0.3.0+aaa"),
            matcher_key_filename("0.3.0+bbb")
        );
        // The collision the old sanitize-to-underscore scheme allowed is now impossible:
        // two versions differing only in punctuation map to DIFFERENT files.
        assert_ne!(
            matcher_key_filename("0.3.0+abc"),
            matcher_key_filename("0_3_0+abc")
        );
    }
}
