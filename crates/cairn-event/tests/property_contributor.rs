//! Property tests for the contributor-role partition and the authorship grade
//! (#212 — the first generative coverage on floor-twin logic).
//!
//! WHY PROPERTIES AND NOT MORE EXAMPLES: these fns are the Rust mirrors of floor
//! logic that reads **attacker-controlled** wire content (a role string and a
//! contributor set travel inside signed-but-hostile-authorable bodies). Example
//! tests pin the documented cases; the properties here pin the LAWS that must hold
//! for *every* input — totality (no panic on junk), the ADR-0051 partition rules,
//! and the ADR-0053 "a claim never silently upgrades or vanishes" grading shape.

use cairn_event::contributor::{
    classify_authorship_confidence, classify_role, is_ratified, AuthorshipConfidence,
    RolePartition, BEARING_PREFIX, CONTRIB_PREFIX,
};
use proptest::prelude::*;

proptest! {
    /// Total + lawful over ANY string: every role classifies into exactly the
    /// partition its prefix or ratification dictates, and everything else is
    /// honestly Unknown (vouching-unknown, never un-vouched).
    #[test]
    fn classify_role_is_total_and_lawful(role in ".{0,64}") {
        let got = classify_role(&role);
        if is_ratified(&role) {
            // Ratified members never classify Unknown.
            prop_assert_ne!(got, RolePartition::Unknown);
        } else if role.starts_with(BEARING_PREFIX) {
            prop_assert_eq!(got, RolePartition::Bearing);
        } else if role.starts_with(CONTRIB_PREFIX) {
            prop_assert_eq!(got, RolePartition::Contributory);
        } else {
            prop_assert_eq!(got, RolePartition::Unknown);
        }
    }

    /// A future member's mandatory prefix classifies it for ARBITRARY suffixes —
    /// the exact "old node meets a role it has never heard of" wire case (#96).
    #[test]
    fn prefixed_future_members_always_classify(suffix in ".{0,32}") {
        prop_assert_eq!(
            classify_role(&format!("{BEARING_PREFIX}{suffix}")),
            RolePartition::Bearing
        );
        prop_assert_eq!(
            classify_role(&format!("{CONTRIB_PREFIX}{suffix}")),
            RolePartition::Contributory
        );
    }
}

/// An arbitrary JSON value of bounded depth — the junk-shaped input an apply door
/// can hand the grader (contributors is hostile wire content, not a trusted shape).
fn arb_json() -> impl Strategy<Value = serde_json::Value> {
    let leaf = prop_oneof![
        Just(serde_json::Value::Null),
        any::<bool>().prop_map(serde_json::Value::from),
        any::<i64>().prop_map(serde_json::Value::from),
        ".{0,12}".prop_map(serde_json::Value::from),
    ];
    leaf.prop_recursive(3, 24, 4, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..4).prop_map(serde_json::Value::from),
            prop::collection::btree_map(".{0,8}", inner, 0..4)
                .prop_map(|m| serde_json::Value::Object(m.into_iter().collect())),
        ]
    })
}

proptest! {
    /// Totality: the grader never panics on arbitrary junk, and junk with no
    /// bearing-role entry grades Device (the honest device-additive default).
    #[test]
    fn authorship_grader_is_total_on_junk(v in arb_json(), signer in ".{0,16}") {
        let _ = classify_authorship_confidence(&v, &signer, None);
    }

    /// The three-way grading law over structured contributor sets:
    ///   * no bearing entry            → Device
    ///   * every bearing entry authenticated (actor == signer/attester) → Attested
    ///   * anything else — including a bearing entry with NO actor_id (an
    ///     anonymous claim) → Unverified, NEVER Device and NEVER Attested.
    ///
    /// The anonymous-claim arm is the sharp edge: dropping such an entry would
    /// silently downgrade "authorship claimed, not authenticated" to "device-
    /// generated", the exact collapse the AuthorshipConfidence doc forbids.
    #[test]
    fn authorship_grading_law(
        entries in prop::collection::vec(
            (
                prop_oneof![
                    Just("authored".to_string()),     // bearing, ratified
                    Just("attested".to_string()),     // bearing, ratified
                    Just("recorded".to_string()),     // contributory
                    Just("bearing:x".to_string()),    // bearing, future-prefixed
                    Just("junk".to_string()),         // unknown → vouching-unknown, not bearing
                ],
                prop_oneof![
                    Just(Some("signer-kid".to_string())),
                    Just(Some("attester-kid".to_string())),
                    Just(Some("stranger-kid".to_string())),
                    Just(None),                       // an anonymous claim
                ],
            ),
            0..6,
        )
    ) {
        let contributors: serde_json::Value = entries
            .iter()
            .map(|(role, actor)| match actor {
                Some(a) => serde_json::json!({"role": role, "actor_id": a}),
                None => serde_json::json!({"role": role}),
            })
            .collect::<Vec<_>>()
            .into();

        let bearing: Vec<&Option<String>> = entries
            .iter()
            .filter(|(role, _)| classify_role(role) == RolePartition::Bearing)
            .map(|(_, actor)| actor)
            .collect();
        let authenticated = |a: &Option<String>| {
            matches!(a.as_deref(), Some("signer-kid") | Some("attester-kid"))
        };

        let expected = if bearing.is_empty() {
            AuthorshipConfidence::Device
        } else if bearing.iter().all(|a| authenticated(a)) {
            AuthorshipConfidence::Attested
        } else {
            AuthorshipConfidence::Unverified
        };

        prop_assert_eq!(
            classify_authorship_confidence(&contributors, "signer-kid", Some("attester-kid")),
            expected
        );
    }
}
