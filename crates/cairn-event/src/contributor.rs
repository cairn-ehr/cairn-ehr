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
}
