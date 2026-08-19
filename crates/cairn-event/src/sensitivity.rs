//! §5.9 sensitivity — the wire shape of a graded confidentiality claim and of its
//! withdrawal (ADR-0006 decision 3, ADR-0062).
//!
//! # Why these bodies are plaintext
//!
//! A node must read the grade in order to COARSEN, and coarsening is exactly what a node
//! holding no custody of the graded body must still do. Sealing the grade under the key it
//! governs is circular — so sensitivity joins ADR-0052 §2's plaintext-by-necessity list.
//!
//! # What is deliberately absent
//!
//! The matched blacklist CATEGORY. These bodies replicate unconditionally in the clear, so
//! `category: "termination-of-pregnancy"` on the wire is the disclosure the grade exists to
//! prevent (ADR-0006 decision 4). The category stays node-local.
//!
//! # Why the builders permit bodies the doors refuse
//!
//! A chart-wide raise with no rationale, and a withdrawal with no rationale, are BUILDABLE
//! here and REFUSED at the local authoring door (db/005). That split is deliberate: the
//! ceremony is a local-authoring rule, never a wire rule (ADR-0060 — a door check at apply
//! would let a peer's rationale-less act fork the event set and wedge replication), and the
//! tests that pin the remote door's leniency need to construct exactly those bodies.
use serde_json::{json, Value};
use uuid::Uuid;

/// Registered in `event_type_class` and the twin-check registry (db/048).
pub const SENSITIVITY_EVENT_TYPE: &str = "sensitivity.grade.asserted";
/// Wire schema version. Bumping it is an ADDITIVE act (ADR-0012).
pub const SENSITIVITY_SCHEMA_VERSION: &str = "sensitivity.grade.asserted/1";
pub const WITHDRAWAL_EVENT_TYPE: &str = "sensitivity.grade-withdrawal.asserted";
pub const WITHDRAWAL_SCHEMA_VERSION: &str = "sensitivity.grade-withdrawal.asserted/1";

/// The four grades db/048 ranks, as they appear on the wire.
///
/// `const`s and deliberately NOT an enum. ADR-0062 decision 2 makes `grade` an **open
/// vocabulary** — a future grade from an upgraded peer is admitted verbatim and ranks MAX
/// ("unknown must coarsen, never expose") — and an enum would both foreclose that and make
/// the inverted-unknown path unreachable through the real API. Naming the four does not
/// close the set; it just stops the ladder existing only as scattered string literals with
/// no Rust definition anywhere (#387).
///
/// The RANKING lives in db/048's `cairn_sensitivity_rank`, not here, and must stay there:
/// a second ordering in Rust is the mirror-pair drift ADR-0064 decision 1 exists to avoid.
pub const GRADE_ROUTINE: &str = "routine";
/// See [`GRADE_ROUTINE`].
pub const GRADE_SENSITIVE: &str = "sensitive";
/// See [`GRADE_ROUTINE`].
pub const GRADE_RESTRICTED: &str = "restricted";
/// See [`GRADE_ROUTINE`].
pub const GRADE_SEQUESTERED: &str = "sequestered";

/// Where a grade came from — the provenance of the tag, never an authority claim.
///
/// A CLOSED enum, unlike [`GRADE_ROUTINE`] and friends, and the asymmetry is deliberate.
/// ADR-0062 decision 5 names exactly two values and offers **no** evolution argument, in
/// sharp contrast to decision 2's extended case for `grade`; db/048 never reads `source`
/// (it is checked non-empty, stored, and referenced by no query, no projection and no rank
/// function); and nothing anywhere branches on it. So an untyped `&str` here bought no
/// forward-compatibility and cost real safety: `"Human"`, `"operator"` or `"advisory "`
/// would pass the builder AND the floor into a plaintext, unconditionally-replicating
/// body, be read by nothing, and — append-only — be correctable only by overlay (#387).
///
/// Builder-side only. db/048's non-empty check is untouched, so a peer's future value is
/// still admitted at the apply door (ADR-0056 governs the door, not the builder).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    /// A human typed it — the manual, operator-driven path.
    Human,
    /// The advisory blacklist candidate db/048 section 13 computes. No such caller exists
    /// yet; it is a later slice.
    Advisory,
}

impl Provenance {
    pub fn as_str(self) -> &'static str {
        match self {
            Provenance::Human => "human",
            Provenance::Advisory => "advisory",
        }
    }
}

/// What an assertion names. Adding a member here means adding it to db/048's
/// `cairn_check_sensitivity_grade` in the same commit — and note that db/048 does NOT
/// refuse an unknown kind: an unrecognised subject kind from a future peer is admitted and
/// interpreted CONSERVATIVELY as chart-wide (ADR-0062; the floor gates effect, not
/// presence — ADR-0056).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubjectKind {
    /// One event.
    Event,
    /// A medication thread (`medication_id`). Later events on the thread inherit the grade
    /// automatically, because the effective grade is computed at READ.
    Thread,
    /// The whole chart. Deliberately the most effortful path: db/005 requires a rationale,
    /// and the blacklist can never author one (ADR-0062).
    Patient,
}

impl SubjectKind {
    /// Every member, in ladder-independent declaration order — the ONE list callers may
    /// enumerate (the CLI's accepted values are built from it, so `--help` cannot drift
    /// from the enum). Pinned by `all_lists_every_subject_kind`, which stops compiling if
    /// a variant is added.
    pub const ALL: [SubjectKind; 3] = [
        SubjectKind::Event,
        SubjectKind::Thread,
        SubjectKind::Patient,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            SubjectKind::Event => "event",
            SubjectKind::Thread => "thread",
            SubjectKind::Patient => "patient",
        }
    }
}

/// Parse a subject kind at a LOCAL INPUT boundary — a CLI argument, a form field.
///
/// **Never at the apply door.** An unrecognised kind arriving from a peer must be ADMITTED
/// and interpreted conservatively as chart-wide (ADR-0056 / ADR-0062: the floor gates
/// effect, not presence); refusing it there would fork the event set. This rejects only
/// what a *local operator* typed, where refusing early and legibly is the kindness.
///
/// The error names the accepted values, derived from [`SubjectKind::ALL`] so the message
/// cannot fall behind the enum.
impl TryFrom<&str> for SubjectKind {
    type Error = String;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        SubjectKind::ALL
            .into_iter()
            .find(|k| k.as_str() == s)
            .ok_or_else(|| {
                let accepted: Vec<&str> = SubjectKind::ALL.iter().map(|k| k.as_str()).collect();
                format!(
                    "{s:?} is not a subject kind this build recognises; accepted: {}",
                    accepted.join(", ")
                )
            })
    }
}

/// A single graded claim. Raising is frictionless by design — err toward confidential.
pub struct SensitivityAssertion<'a> {
    pub subject_kind: SubjectKind,
    /// The event, medication thread, or chart being graded. When `subject_kind` is
    /// `Patient` the local door requires this to equal the envelope's `patient_id`: a
    /// mis-typed pair coarsens the chart it was authored on while leaving the chart the
    /// author meant to seal silently reading `routine` (db/048 section 12).
    pub subject_id: Uuid,
    /// Open vocabulary: db/048 ranks the named ladder and treats anything else as MAX.
    pub grade: &'a str,
    /// Where the tag came from — see [`Provenance`]. Typed, unlike `grade`, because
    /// ADR-0062 decision 5 closes this set and nothing reads it.
    pub source: Provenance,
    /// Required by the local door when `subject_kind` is `Patient`; optional otherwise.
    pub rationale: Option<&'a str>,
}

/// Removing a claim from the standing set. Nothing is erased: the assertion stays in the
/// log, readable and re-assertable.
pub struct SensitivityWithdrawal<'a> {
    /// Hex `content_address` of the assertion being withdrawn. Hex because that is what the
    /// payload carries; db/048 decodes it through `cairn_decode_hex_or_raise` so a malformed
    /// value fails legibly with P0001 rather than stalling a pull (#228).
    pub withdraws_hex: &'a str,
    /// The audited why. **Clear text forever, and it replicates** — a rationale naming the
    /// condition leaks precisely what the grade protects. The UI must say so at entry.
    pub rationale: &'a str,
}

pub fn sensitivity_assertion_body(a: &SensitivityAssertion) -> Value {
    let mut body = json!({
        "subject_kind": a.subject_kind.as_str(),
        "subject_id": a.subject_id.to_string(),
        "grade": a.grade,
        "source": a.source.as_str(),
    });
    // Absent, never `null`: an explicit null is an author asserting something about a
    // rationale, and absence is the honest "none given".
    if let Some(r) = a.rationale {
        body["rationale"] = json!(r);
    }
    body
}

pub fn sensitivity_withdrawal_body(w: &SensitivityWithdrawal) -> Value {
    json!({ "withdraws": w.withdraws_hex, "rationale": w.rationale })
}

/// The mandatory §3.13 legibility twin — this act in plain language, for a reader with no
/// schema at all (principle 11).
pub fn render_sensitivity_twin(a: &SensitivityAssertion) -> String {
    let subject = match a.subject_kind {
        SubjectKind::Event => "one event",
        SubjectKind::Thread => "one medication thread",
        SubjectKind::Patient => "this whole chart",
    };
    let mut out = format!(
        "Confidentiality grade \"{}\" asserted over {} ({}), source: {}",
        a.grade,
        subject,
        a.subject_id,
        a.source.as_str()
    );
    if let Some(r) = a.rationale {
        out.push_str(&format!("; reason: {r}"));
    }
    out
}

pub fn render_withdrawal_twin(w: &SensitivityWithdrawal) -> String {
    format!(
        "Confidentiality grade withdrawn (assertion {}); reason: {}. \
         The withdrawn assertion remains on the record.",
        w.withdraws_hex, w.rationale
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_thread_assertion_carries_subject_grade_and_source_and_no_category() {
        let a = SensitivityAssertion {
            subject_kind: SubjectKind::Thread,
            subject_id: uuid::Uuid::nil(),
            grade: "restricted",
            source: Provenance::Human,
            rationale: None,
        };
        let b = sensitivity_assertion_body(&a);
        assert_eq!(b["subject_kind"], "thread");
        assert_eq!(b["grade"], "restricted");
        assert_eq!(b["source"], "human");
        // The matched blacklist category must NEVER be on the wire: a plaintext,
        // unconditionally-replicated body naming the category IS the disclosure.
        assert!(b.get("category").is_none(), "category must never travel");
        assert!(b.get("rationale").is_none(), "absent, not null");
    }

    #[test]
    fn the_builder_can_construct_a_rationale_less_chart_wide_raise() {
        // Deliberate: rationale is a DOOR rule (db/005), never a builder invariant. The
        // remote-door leniency test needs exactly this body, so a builder that refused it
        // would make the door asymmetry untestable.
        let a = SensitivityAssertion {
            subject_kind: SubjectKind::Patient,
            subject_id: uuid::Uuid::nil(),
            grade: "sensitive",
            source: Provenance::Human,
            rationale: None,
        };
        let b = sensitivity_assertion_body(&a);
        assert_eq!(b["subject_kind"], "patient");
        assert!(b.get("rationale").is_none());
    }

    #[test]
    fn a_withdrawal_names_the_assertion_it_withdraws_in_hex() {
        let w = SensitivityWithdrawal {
            withdraws_hex: "a1b2c3",
            rationale: "patient consent 2026-08-09, recorded in note E44",
        };
        let b = sensitivity_withdrawal_body(&w);
        assert_eq!(b["withdraws"], "a1b2c3");
        assert_eq!(
            b["rationale"],
            "patient consent 2026-08-09, recorded in note E44"
        );
    }

    #[test]
    fn the_twins_read_without_a_schema_and_never_name_the_category() {
        let a = SensitivityAssertion {
            subject_kind: SubjectKind::Patient,
            subject_id: uuid::Uuid::nil(),
            grade: "restricted",
            source: Provenance::Advisory,
            rationale: Some("staff member treated here"),
        };
        let t = render_sensitivity_twin(&a);
        assert!(t.contains("restricted"), "the grade is the point: {t}");
        assert!(
            t.contains("whole chart"),
            "the subject must be legible: {t}"
        );

        let w = SensitivityWithdrawal {
            withdraws_hex: "a1b2c3",
            rationale: "consent",
        };
        let tw = render_withdrawal_twin(&w);
        assert!(
            tw.contains("consent"),
            "the audited why must be legible: {tw}"
        );
    }
}

#[cfg(test)]
mod type_design {
    //! #387 — the closed sets get one definition each.
    use super::*;

    #[test]
    fn all_lists_every_subject_kind() {
        // THE DRIFT GUARD, and it works in two directions at once. The `match` is
        // exhaustive over the enum, so adding a variant stops this file COMPILING until
        // someone looks here; the length assertion then stops them from fixing the compile
        // error without also extending `ALL`. Before this, the closed set lived in three
        // hand-maintained Rust copies and only the SQL side had test pressure.
        for k in SubjectKind::ALL {
            match k {
                SubjectKind::Event | SubjectKind::Thread | SubjectKind::Patient => {}
            }
        }
        assert_eq!(SubjectKind::ALL.len(), 3);
    }

    #[test]
    fn every_subject_kind_round_trips_through_its_wire_word() {
        // `as_str` is what goes on the wire; `try_from` is what comes off a CLI argument.
        // If they ever disagree, a value this build emits is a value it cannot read back.
        for k in SubjectKind::ALL {
            assert_eq!(SubjectKind::try_from(k.as_str()), Ok(k), "{}", k.as_str());
        }
    }

    #[test]
    fn an_unknown_subject_kind_is_refused_and_the_message_names_what_is_accepted() {
        // Refused at the CLI boundary only. The APPLY door must keep admitting an
        // unrecognised kind (ADR-0056/ADR-0062 — it is interpreted conservatively as
        // chart-wide), so this must never be mistaken for a wire-level rejection.
        let e = SubjectKind::try_from("episode").expect_err("not a subject kind this build has");
        for k in SubjectKind::ALL {
            assert!(
                e.contains(k.as_str()),
                "the error must name {}: {e}",
                k.as_str()
            );
        }
    }

    #[test]
    fn provenance_carries_the_two_values_adr_0062_names() {
        // ADR-0062 decision 5 names exactly two and offers NO evolution argument — in sharp
        // contrast to decision 2's extended case for `grade` being open. db/048 never reads
        // `source`: it is checked non-empty, stored, and referenced by no query, no
        // projection and no rank function. So `"Human"` / `"operator"` / `"advisory "` would
        // pass builder AND floor into a plaintext, unconditionally-replicating body, and
        // nothing would ever notice.
        assert_eq!(Provenance::Human.as_str(), "human");
        assert_eq!(Provenance::Advisory.as_str(), "advisory");
    }

    #[test]
    fn the_ladder_constants_carry_db048s_wire_words() {
        // The ladder is ranked in db/048 and, until now, had no Rust definition at all.
        // These are `const`s and NOT an enum ON PURPOSE: ADR-0062 decision 2 makes `grade`
        // an OPEN vocabulary, and an enum would both break that and make the
        // inverted-unknown path (unknown ranks MAX) unreachable through the real API.
        assert_eq!(GRADE_ROUTINE, "routine");
        assert_eq!(GRADE_SENSITIVE, "sensitive");
        assert_eq!(GRADE_RESTRICTED, "restricted");
        assert_eq!(GRADE_SEQUESTERED, "sequestered");
    }

    #[test]
    fn a_grade_outside_the_named_ladder_is_still_buildable() {
        // The open-vocabulary guarantee, pinned. A future peer's grade must be
        // constructible here, or this build could not round-trip its own inbound traffic.
        let a = SensitivityAssertion {
            subject_kind: SubjectKind::Patient,
            subject_id: uuid::Uuid::nil(),
            grade: "embargoed-by-court-order",
            source: Provenance::Human,
            rationale: Some("why"),
        };
        assert_eq!(
            sensitivity_assertion_body(&a)["grade"],
            "embargoed-by-court-order"
        );
    }
}
