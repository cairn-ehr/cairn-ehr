//! The ratified contributor-role vocabulary and its partition classifier
//! (ADR-0028 membership + ADR-0051 ratification of `recorded` and the wire
//! encoding for future members; spec §3.9).
//!
//! Why this exists: the role enum is a safety primitive — the structural
//! *"AI-generated"* reading and the suppression owner-gate branch on whether a
//! role **bears responsibility**. ADR-0051 closes two wire windows (#203/#96):
//!
//!   * the 12 ratified members travel as bare names (`"attested"`, `"recorded"`);
//!   * any member a FUTURE ADR adds must travel partition-prefixed
//!     (`"bearing:delegated"` / `"contrib:annotated"`) so a node that has never
//!     heard of it can still classify it — set-union sync must never depend on
//!     both ends sharing a vocabulary version;
//!   * a role that is neither known nor prefixed classifies as [`RolePartition::Unknown`]
//!     and any consumer must render it as **vouching-unknown** — an honest
//!     first-class state (principle 4), never collapsed to "un-vouched".
//!
//! The same vocabulary lives in SQL as the `contributor_role` table (db/005) where
//! the unbypassable floor reads it; `contributor_roles.rs` in cairn-node carries the
//! drift guard that keeps the two in lockstep.

/// The wire prefix a future *responsibility-bearing* member must carry.
pub const BEARING_PREFIX: &str = "bearing:";
/// The wire prefix a future *contributory* member must carry.
pub const CONTRIB_PREFIX: &str = "contrib:";

/// The ratified vocabulary: `(wire value, bears responsibility)`. Additive-only —
/// a new entry is an ADR-recorded act (ADR-0028 extension discipline) and must be
/// appended here AND to the `contributor_role` table in db/005 together.
pub const ROLE_VOCABULARY: [(&str, bool); 12] = [
    // Responsibility-bearing (6) — ADR-0028.
    ("authored", true),
    ("ordered", true),
    ("attested", true),
    ("co-signed", true),
    ("witnessed", true),
    ("dictated", true),
    // Contributory (6) — ADR-0028's five + `recorded` (ADR-0051): the recording
    // device/system that captured and persisted the event. It asserts capture
    // fidelity, adds no clinical content, and bears no clinical responsibility.
    ("drafted", false),
    ("transcribed", false),
    ("graded", false),
    ("triaged", false),
    ("suggested", false),
    ("recorded", false),
];

/// How a role value classifies against the bearing/contributory partition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RolePartition {
    /// A responsibility-bearing role (ratified member or `bearing:`-prefixed).
    Bearing,
    /// A contributory role (ratified member or `contrib:`-prefixed).
    Contributory,
    /// Neither ratified nor prefixed — consumers MUST render this as
    /// vouching-unknown, never as un-vouched (ADR-0051 / #96).
    Unknown,
}

/// Classify a wire role value against the partition. Pure; total over any string.
///
/// Known members classify from the ratified table; unknown members classify from
/// their mandatory partition prefix; everything else is honestly [`RolePartition::Unknown`].
pub fn classify_role(role: &str) -> RolePartition {
    if let Some(&(_, bears)) = ROLE_VOCABULARY.iter().find(|(r, _)| *r == role) {
        return if bears {
            RolePartition::Bearing
        } else {
            RolePartition::Contributory
        };
    }
    if role.starts_with(BEARING_PREFIX) {
        RolePartition::Bearing
    } else if role.starts_with(CONTRIB_PREFIX) {
        RolePartition::Contributory
    } else {
        RolePartition::Unknown
    }
}

/// True iff `role` is a ratified member this vocabulary version may AUTHOR.
/// (The submit door only authors what it can stand behind; prefixed future
/// members are sync-plane admissible but never locally authorable.)
pub fn is_ratified(role: &str) -> bool {
    ROLE_VOCABULARY.iter().any(|(r, _)| *r == role)
}

use crate::EventBody;

/// Rewrite a device-shaped clinical body so a human takes AUTHORSHIP of it (#204 /
/// ADR-0053): prepend an `authored` contributor for the human (no `responsibility`
/// object — "authored, not-yet-vouched", a legitimate §3.9 state) and make the human
/// the event's signer. The device `recorded` contributor is preserved AFTER the
/// human — mixed sets like `{human, authored} + {node, recorded}` are compositional
/// authorship working as designed (ADR-0051). Pure; the caller then signs the sealed
/// bytes with the human's key while the node keeps custody (session ≠ author).
pub fn with_human_author(mut body: EventBody, human_kid: &str) -> EventBody {
    let author = serde_json::json!({"actor_id": human_kid, "role": "authored"});
    match body.contributors.as_array_mut() {
        Some(arr) => arr.insert(0, author),
        None => body.contributors = serde_json::json!([author]),
    }
    body.signer_key_id = human_kid.to_string();
    body
}

/// The authorship-confidence grade an event carries (ADR-0008 "a grade, not a gate";
/// ADR-0053). The single, shared reading every consumer must use so an unverifiable
/// claim is never displayed as authenticated.
///
/// NOT YET WIRED TO A READ PATH. This grade has no production consumer today — the SQL
/// mirror and the §5.10 authorship-confidence projection are issue #245. No read path in
/// this repo surfaces the contributor set to a clinician yet, so nothing is currently
/// rendering an unverified claim as authenticated; but do not read this type's existence
/// as evidence that grading is in force. It is the reference definition #245 will mirror
/// into SQL, and its "authenticated" test must stay in lockstep with
/// `cairn_authorship_bound` (db/005): the two ask the same question at opposite doors —
/// that one REFUSES at authoring, this one GRADES at read (see the "STRICT DOOR ONLY"
/// note there for why the asymmetry is deliberate).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorshipConfidence {
    /// A responsibility-bearing human author, authenticated as the signer or a verified attester.
    Attested,
    /// A responsibility-bearing author this node cannot verify (actor ≠ signer, no verifiable
    /// token) — a forgery OR an author authenticated by a scheme this node is too old to parse.
    /// Rendered "authorship claimed, not authenticated here", never `Attested`, and upgradable.
    Unverified,
    /// No responsibility-bearing contributor — the honest device-additive default (`recorded`).
    Device,
}

/// Grade an event's authorship from its contributor set, the verified signer, and the
/// verified attester (if any). Pure; total. A bearing contributor is "authenticated"
/// iff its actor is the signer or the verified attester; every bearing author must be
/// authenticated for `Attested`, else `Unverified`; no bearing contributor at all is
/// `Device`.
///
/// A bearing entry with a missing or non-string `actor_id` is an ANONYMOUS claim: it
/// still counts as a bearing contributor (→ never `Device`) and can never authenticate
/// (→ never `Attested`). Dropping it instead would silently downgrade "authorship
/// claimed, not authenticated" to "device-generated" — the exact collapse this grade
/// exists to prevent (caught by the #212 property suite before any read path shipped).
pub fn classify_authorship_confidence(
    contributors: &serde_json::Value,
    signer_key_id: &str,
    verified_attester: Option<&str>,
) -> AuthorshipConfidence {
    // Every bearing-role entry's actor claim, kept as Option so an anonymous claim
    // stays visible to the grading instead of vanishing from the set.
    let bearing: Vec<Option<&str>> = contributors
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|e| {
                    classify_role(e.get("role").and_then(|r| r.as_str()).unwrap_or(""))
                        == RolePartition::Bearing
                })
                .map(|e| e.get("actor_id").and_then(|v| v.as_str()))
                .collect()
        })
        .unwrap_or_default();
    if bearing.is_empty() {
        return AuthorshipConfidence::Device;
    }
    let authenticated = |actor: Option<&str>| match actor {
        Some(a) => a == signer_key_id || verified_attester == Some(a),
        None => false,
    };
    if bearing.iter().all(|a| authenticated(*a)) {
        AuthorshipConfidence::Attested
    } else {
        AuthorshipConfidence::Unverified
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratified_members_classify_from_the_table() {
        assert_eq!(classify_role("attested"), RolePartition::Bearing);
        assert_eq!(classify_role("dictated"), RolePartition::Bearing);
        assert_eq!(classify_role("recorded"), RolePartition::Contributory);
        assert_eq!(classify_role("triaged"), RolePartition::Contributory);
    }

    #[test]
    fn future_members_classify_from_their_mandatory_prefix() {
        assert_eq!(classify_role("bearing:delegated"), RolePartition::Bearing);
        assert_eq!(
            classify_role("contrib:annotated"),
            RolePartition::Contributory
        );
    }

    #[test]
    fn unknown_unprefixed_roles_are_honestly_unknown() {
        // `reviewed` is ADR-0028's deliberately-rejected candidate — it must never
        // silently classify; a consumer renders it vouching-unknown.
        assert_eq!(classify_role("reviewed"), RolePartition::Unknown);
        assert_eq!(classify_role(""), RolePartition::Unknown);
        assert_eq!(classify_role("curated"), RolePartition::Unknown);
    }

    #[test]
    fn vocabulary_is_twelve_six_six() {
        assert_eq!(ROLE_VOCABULARY.len(), 12);
        assert_eq!(ROLE_VOCABULARY.iter().filter(|(_, b)| *b).count(), 6);
        assert!(is_ratified("recorded") && !is_ratified("bearing:delegated"));
    }

    #[test]
    fn with_human_author_prepends_authored_and_makes_human_the_signer() {
        // A device-shaped body (node recorded, node signs) gains the human author IN
        // FRONT, and the human becomes the signer — session(node) ≠ author(human).
        let body = crate::EventBody {
            event_id: "e".into(),
            patient_id: "p".into(),
            event_type: "clinical.medication.asserted".into(),
            schema_version: "clinical.medication/1".into(),
            hlc: crate::Hlc {
                wall: 1,
                counter: 0,
                node_origin: "n".into(),
            },
            t_effective: None,
            signer_key_id: "NODEKID".into(),
            contributors: serde_json::json!([{"actor_id": "NODEKID", "role": "recorded"}]),
            payload: serde_json::json!({}),
            attachments: vec![],
            plaintext_twin: Some("twin".into()),
            clock_grade: crate::ClockGrade::SelfAsserted,
        };
        let out = with_human_author(body, "HUMANKID");
        assert_eq!(out.signer_key_id, "HUMANKID");
        assert_eq!(out.contributors[0]["actor_id"], "HUMANKID");
        assert_eq!(out.contributors[0]["role"], "authored");
        assert!(out.contributors[0].get("responsibility").is_none());
        // The device recorded contributor is preserved after the human author.
        assert_eq!(out.contributors[1]["actor_id"], "NODEKID");
        assert_eq!(out.contributors[1]["role"], "recorded");
    }

    #[test]
    fn authorship_grade_attested_when_bearing_author_is_the_signer() {
        let c = serde_json::json!([
            {"actor_id": "H", "role": "authored"},
            {"actor_id": "N", "role": "recorded"}]);
        assert_eq!(
            classify_authorship_confidence(&c, "H", None),
            AuthorshipConfidence::Attested
        );
    }

    #[test]
    fn authorship_grade_attested_when_bearing_author_is_the_verified_attester() {
        let c = serde_json::json!([{"actor_id": "H", "role": "attested",
                                    "responsibility": {"held_by": "H"}}]);
        // signer is the node, but the bearing human is the verified attester.
        assert_eq!(
            classify_authorship_confidence(&c, "N", Some("H")),
            AuthorshipConfidence::Attested
        );
    }

    #[test]
    fn authorship_grade_unverified_when_bearing_author_is_neither_signer_nor_attester() {
        let c = serde_json::json!([
            {"actor_id": "H", "role": "authored"},   // claimed human author
            {"actor_id": "N", "role": "recorded"}]);
        // signed by the node, no token for H — a forgery OR a future credential; either way unverified.
        assert_eq!(
            classify_authorship_confidence(&c, "N", None),
            AuthorshipConfidence::Unverified
        );
    }

    #[test]
    fn authorship_grade_device_when_no_bearing_contributor() {
        let c = serde_json::json!([{"actor_id": "N", "role": "recorded"}]);
        assert_eq!(
            classify_authorship_confidence(&c, "N", None),
            AuthorshipConfidence::Device
        );
    }
}
