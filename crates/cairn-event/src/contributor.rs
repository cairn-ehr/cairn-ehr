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
/// STILL NOT WIRED TO A DISPLAY READ PATH. No code in this repo surfaces the contributor
/// set to a clinician yet, so #245's DISPLAY half — the §5.10 authorship-confidence
/// projection — is still open; do not read this type's existence as evidence that grading
/// is in force at any UI. #245 is NOT closed by anything below.
///
/// It now DOES have two SQL counterparts, at opposite doors, and both owe it lockstep:
///
///   * `cairn_authorship_bound` (db/005) asks the same question at authoring — that one
///     REFUSES an unbound claim at the STRICT door, this one GRADES an admitted claim at
///     read (see the "STRICT DOOR ONLY" note on that function for why the asymmetry is
///     deliberate).
///   * `cairn_claim_authority` (db/005, ADR-0064) asks a related-but-not-identical question
///     at a DIFFERENT read: whether a claim over an EXISTING event (e.g. a sensitivity
///     withdrawal) is authoritative. The two read DISJOINT inputs — this function reads
///     `contributors`/`signer_key_id`, `cairn_claim_authority`'s R1 branch reads only
///     `attester_key`/`cairn_attestation_vouched`/`actor_current` and never looks at
///     `contributors` at all — so "mirror" overstates it, and it is not a full mapping
///     either: its R2 ('self': a human withdrawing their own claim) has no Rust counterpart
///     here; this function's `Device` (no responsibility-bearing contributor at all) has no
///     SQL counterpart there; and R1 additionally demands the attester resolve to EXACTLY
///     ONE `kind = 'human'` actor, a check this function does not perform at all. Two
///     door-admissible shapes are ALREADY KNOWN to diverge on that account — a key mapped to
///     more than one actor can grade `Attested` here and `'unverified'` in SQL; a
///     suppressing-mode event whose only contributor is `recorded` (no `responsibility`
///     object, so `cairn_responsibility_bound` is vacuously true) can grade `Device` here and
///     `'attested'` in SQL — filed as an issue for #245's display half to resolve, not fixed
///     by anything below. Where the two DO overlap on the shapes
///     `crates/cairn-node/tests/authority_lockstep.rs` actually exercises (a single vouched
///     human attester; a claimed-but-unattested human author; a device-only contributor set)
///     they agree: `Attested` <-> `'attested'`, `Unverified`/`Device` both <-> `'unverified'`.
///     That is what the test pins — an obligation on those shapes, not a universal guarantee
///     over every event this repo can admit.
///
/// **This enum stays exhaustive, deliberately** (#412 weighed `#[non_exhaustive]` and
/// declined it; rationale corrected 2026-08-16 after the first version rebutted the wrong
/// form). Two forms exist and they are not the same argument:
///
///   * **Variant-level** — would stop a downstream crate MINTING an `Attested`. Pointless
///     here: a minted value is harmless on its own, since the defect #412 closed was a
///     forgeable *computation*, now closed at the classifier's inputs by [`VerifiedKid`].
///     It also costs real ergonomics — cross-crate `AuthorshipConfidence::Attested` stops
///     being usable as an *expression*, so every `assert_eq!` (the shape the lockstep test
///     uses to hold Rust and SQL together) would have to become a `matches!`.
///   * **Enum-level** — the form anyone would actually propose, and it costs those
///     `assert_eq!`s nothing. It is declined for a SAFETY reason, not an ergonomic one:
///     it would force every downstream consumer to add a `_ =>` arm, i.e. a silent handler
///     for a grade that does not exist yet. For a display grade whose named hazard is
///     exactly *"silently downgrade authorship-claimed to device-generated"* (see the
///     anonymous-claim note on [`classify_authorship_confidence`]), breaking every
///     consumer's build when a fourth grade lands is the FEATURE. An exhaustive enum forces
///     a conscious rendering decision at each site; a wildcard arm guarantees the collapse
///     this type exists to prevent.
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

/// A key id whose provenance is a COMPLETED VERIFICATION — never a claim read out of a
/// body (#412).
///
/// # Why a newtype rather than a `&str` and a careful reviewer
///
/// [`EventBody`] carries `signer_key_id` — what the body *claims* — immediately beside
/// `contributors`. When both arguments to [`classify_authorship_confidence`] were bare
/// `&str`, the natural line to write was
///
/// ```ignore
/// classify_authorship_confidence(&body.contributors, &body.signer_key_id, None)
/// ```
///
/// which graded `Attested` for any forgery that set `contributors[0].actor_id` equal to
/// `signer_key_id`, with no signature checked anywhere. The place a display path runs is
/// exactly where an `EventBody` has just been deserialised from an untrusted peer, so that
/// line is not a hypothetical mistake.
///
/// SQL never had the bug, for a structural reason this type copies — but the structure is
/// **two conjuncts, not one**, and getting that wrong is how a caller re-opens #412 through
/// this very type (2026-08-16 review). `cairn_claim_authority` (db/005) does not merely read
/// `event_log.attester_key`; its R1 arm is `attester_key IS NOT NULL AND
/// cairn_attestation_vouched(event_id)`. The second conjunct is load-bearing because the
/// column can hold an UNVERIFIED token: db/020's deferred arm stores `p_attester_key`
/// verbatim before any `cairn_attestation_ok` runs, and says so — *"an attestation on a row
/// that carries an `event_deferred` marker is CARRIED, NOT VOUCHED — nothing has verified
/// it."* So *reading the column* is **not** the proof; reading it **and** clearing the vouch
/// marker is. Here the type carries the proof only as far as its minter's premise holds.
///
/// # The two routes to a mint, and the one constructor each takes
///
///   * [`crate::VerifiedEvent::signer`] — this crate verified the bytes just now. It mints
///     through a crate-private constructor, so the compiler underwrites this route whole.
///   * [`VerifiedKid::from_event_log_column`] — the caller read a proof-carrying column.
///     Same proof, different route, and no bytes left to re-verify; the compiler cannot
///     check the premise, so what remains is making a wrong call site conspicuous in review
///     and greppable in the tree — a deliberate act instead of the path of least resistance.
///
/// Keeping the two constructors distinct is what keeps that grep honest: every
/// `from_event_log_column` hit is a real DB-provenance assertion a reviewer must weigh, and
/// none of them is this crate's own verifier laundering itself through a name that would be
/// false at that call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifiedKid<'a>(&'a str);

impl<'a> VerifiedKid<'a> {
    /// Mint from an `event_log` column whose existence IS a completed verification.
    ///
    /// **The two columns do not carry equal proof, and the caller owes the difference:**
    ///
    ///   * `signer_key_id` — unconditionally sound. db/005 step 1 runs `cairn_verify`, which
    ///     is [`crate::verify_self_described`], and that refuses bytes whose body claims a
    ///     signer other than the key the signature used
    ///     ([`crate::EventError::SignerKeyMismatch`]). No row exists otherwise.
    ///   * `attester_key` — sound **only for a row that clears `cairn_attestation_vouched`**
    ///     (db/001), which is the predicate `cairn_claim_authority`'s R1 pairs it with.
    ///     db/020's deferred arm writes this column from an unverified, peer-supplied token;
    ///     minting from such a row hands the grader a key nothing checked, which is #412
    ///     again with extra steps. Read the marker, or do not mint.
    ///
    /// **Never call this on a value taken from a deserialised [`EventBody`].** That value
    /// is the claim, not the proof, and passing it here re-opens #412 in full.
    pub fn from_event_log_column(kid_hex: &'a str) -> Self {
        VerifiedKid(kid_hex)
    }

    /// Mint from a signature THIS CRATE just verified.
    ///
    /// Crate-private on purpose: its only caller is [`crate::VerifiedEvent::signer`], whose
    /// `signer_kid_hex` is the key the COSE signature actually verified against. Keeping it
    /// separate from [`Self::from_event_log_column`] is not cosmetic — it is what stops the
    /// honest in-crate route from showing up in a grep for DB-provenance assertions, which
    /// is the only enforcement the second constructor has.
    pub(crate) fn from_verified_signature(kid_hex: &'a str) -> Self {
        VerifiedKid(kid_hex)
    }

    /// The key id itself, for callers that must render or compare it.
    pub fn as_str(&self) -> &'a str {
        self.0
    }
}

/// Grade an event's authorship from its contributor set, the verified signer, and the
/// verified attester (if any). Pure; total. A bearing contributor is "authenticated"
/// iff its actor is the signer or the verified attester; every bearing author must be
/// authenticated for `Attested`, else `Unverified`; no bearing contributor at all is
/// `Device`.
///
/// Both key arguments are [`VerifiedKid`], so the *natural* forgeable call — reading the
/// signer straight off the untrusted body — no longer compiles (#412). Note the scope: this
/// closes the accidental spelling, not every spelling. Wrapping the same field in
/// [`VerifiedKid::from_event_log_column`] still compiles and still reproduces #412; that
/// route stays a review obligation, which is why the constructor is named after the premise
/// it asserts rather than something neutral like `new`.
///
/// ```compile_fail
/// use cairn_event::contributor::classify_authorship_confidence;
/// # fn demo(body: &cairn_event::EventBody) {
/// // `&body.signer_key_id` is the body's own CLAIM. It is a `&str`, not a VerifiedKid,
/// // and this line must never compile again.
/// let _ = classify_authorship_confidence(&body.contributors, &body.signer_key_id, None);
/// # }
/// ```
///
/// The companion below is what keeps that negative test HONEST, and it is not decoration.
/// `compile_fail` asserts only that a snippet fails to build, never why — and rustdoc on
/// stable ignores an `E0308`-style error-code annotation entirely (verified: a deliberately
/// wrong code still reports `ok`), so pinning the code is not available as a fix. Rename
/// `EventBody::signer_key_id` or `::contributors` and the block above would start failing
/// for that unrelated reason while still reporting success — silently pinning nothing. This
/// one names the same fields on the honest path, so the vacuity shows up as a failure here.
///
/// ```
/// use cairn_event::contributor::classify_authorship_confidence;
/// # fn demo(bytes: &[u8]) -> Result<(), cairn_event::EventError> {
/// let v = cairn_event::verify_self_described_event(bytes)?;
/// let _claim: &str = &v.body().signer_key_id;   // the field the forgery above reads
/// let _ = classify_authorship_confidence(&v.body().contributors, v.signer(), None);
/// # Ok(()) }
/// ```
///
/// A bearing entry with a missing or non-string `actor_id` is an ANONYMOUS claim: it
/// still counts as a bearing contributor (→ never `Device`) and can never authenticate
/// (→ never `Attested`). Dropping it instead would silently downgrade "authorship
/// claimed, not authenticated" to "device-generated" — the exact collapse this grade
/// exists to prevent (caught by the #212 property suite before any read path shipped).
pub fn classify_authorship_confidence(
    contributors: &serde_json::Value,
    signer: VerifiedKid<'_>,
    verified_attester: Option<VerifiedKid<'_>>,
) -> AuthorshipConfidence {
    let signer_key_id = signer.as_str();
    let verified_attester = verified_attester.map(|v| v.as_str());
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

    /// A verified key id for grading tests.
    ///
    /// These tests are about the classification LAW, not about provenance, so they mint
    /// through the DB-column constructor: in the shapes they describe the value would
    /// indeed have come from `event_log.signer_key_id` or `event_log.attester_key`. The
    /// provenance guarantee itself is pinned in `tests/verified_kid.rs` (the honest mint)
    /// and by the `compile_fail` doctest on `classify_authorship_confidence` (the forgery
    /// that must no longer build).
    fn kid(s: &str) -> VerifiedKid<'_> {
        VerifiedKid::from_event_log_column(s)
    }

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
            safety: None,
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
            classify_authorship_confidence(&c, kid("H"), None),
            AuthorshipConfidence::Attested
        );
    }

    #[test]
    fn authorship_grade_attested_when_bearing_author_is_the_verified_attester() {
        let c = serde_json::json!([{"actor_id": "H", "role": "attested",
                                    "responsibility": {"held_by": "H"}}]);
        // signer is the node, but the bearing human is the verified attester.
        assert_eq!(
            classify_authorship_confidence(&c, kid("N"), Some(kid("H"))),
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
            classify_authorship_confidence(&c, kid("N"), None),
            AuthorshipConfidence::Unverified
        );
    }

    #[test]
    fn authorship_grade_device_when_no_bearing_contributor() {
        let c = serde_json::json!([{"actor_id": "N", "role": "recorded"}]);
        assert_eq!(
            classify_authorship_confidence(&c, kid("N"), None),
            AuthorshipConfidence::Device
        );
    }
}
