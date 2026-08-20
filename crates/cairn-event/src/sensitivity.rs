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
/// a second ordering in Rust is the drift shape ADR-0064's rejected "per-dial authority
/// check" describes — `cairn_prospective_sensitivity`/`cairn_effective_sensitivity` are a
/// hand-maintained mirror pair that has ALREADY diverged once (#404) — and decision 5
/// ("one predicate, consulted at exactly one site per dial") exists to avoid.
///
/// These four words ARE checked against db/048, but from the one place that can do it
/// honestly: `the_ladder_orders_the_named_grades_and_ranks_the_unknown_maximum` in
/// `crates/cairn-node/tests/sensitivity_ladder.rs` feeds each const to
/// `cairn_sensitivity_rank` over a live connection. Asserting them against their own
/// literals here would pin nothing.
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
/// The warrant is what db/048 does, not what an ADR says: `source` is checked non-empty,
/// stored, and then referenced by **no query, no projection and no rank function** — the
/// whole file writes it and never reads it back, and nothing in `crates/` branches on it
/// either. (ADR-0062 decision 5 names the two values, but only in passing: that decision
/// is titled "The matched blacklist category never travels on the wire" and is about the
/// category, so read it as where the two values are *recorded*, not as an argument that
/// the set is closed. Decision 2 makes the contrasting case for `grade` being open at
/// length.)
///
/// So an untyped `&str` here bought no forward-compatibility and cost real safety:
/// `"Human"`, `"operator"` or `"advisory "` would pass the builder AND the floor into a
/// plaintext, unconditionally-replicating body; be read by nothing except
/// [`render_sensitivity_twin`], which prints it into the mandatory signed §3.13 legibility
/// twin a human reads FOREVER; and — append-only — be correctable only by overlay (#387).
/// The twin is the reason a typo here matters at all.
///
/// Builder-side only, and that scoping is load-bearing in both directions:
///   * db/048's non-empty check is untouched, so a peer's future value is still admitted
///     at the apply door (ADR-0056 governs the door, not the builder); and
///   * db/048 itself mints a THIRD value — `sensitivity_assertion_apply` writes
///     `source = 'unreadable'` on the born-sealed branch — which is the projection
///     speaking, not an author. A read model must therefore keep `source` as open text
///     (copy `WithdrawalWorklistRow::arm`'s treatment) and must never parse back INTO
///     this enum.
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

/// Define the closed `SubjectKind` set ONCE — the enum, its wire words, and the
/// enumerable list all expand from the single table in the invocation below.
///
/// # Why a macro, in a crate that otherwise prefers plain functions (house rule 4)
///
/// Because a hand-written `ALL` **cannot be guarded by any test.** Every check you could
/// write has to name the variants itself, so the check drifts alongside the list it is
/// meant to be checking. That is not hypothetical — it is what the first version of this
/// code shipped as, and the review caught it: `assert_eq!(ALL.len(), 3)` compared
/// `[SubjectKind; 3]::len()`, a compile-time constant, against its own literal. It could
/// not fail. Adding a fourth variant and fixing only the resulting compile error left the
/// whole suite green while `ALL` still listed three, at which point `as_str` emits a wire
/// word that `try_from` refuses and `--help` never lists — "a value this build emits is a
/// value it cannot read back", which is precisely the drift #387 exists to end.
///
/// A macro is the only construction in stable Rust (no new dependency — house rule 1) that
/// removes the second list entirely. There is nothing left to fall behind.
///
/// Deliberately NOT extended to the `grade` ladder: that vocabulary is OPEN (ADR-0062
/// decision 2), so there is no closed set to generate.
macro_rules! subject_kinds {
    ($( $(#[$doc:meta])* $variant:ident => $wire:literal ),+ $(,)?) => {
        /// What an assertion names. Adding a member here means adding it to db/048's
        /// `cairn_check_sensitivity_grade` in the same commit — and note that db/048 does
        /// NOT refuse an unknown kind: an unrecognised subject kind from a future peer is
        /// admitted and interpreted CONSERVATIVELY as chart-wide (ADR-0062; the floor
        /// gates effect, not presence — ADR-0056).
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum SubjectKind {
            $( $(#[$doc])* $variant, )+
        }

        impl SubjectKind {
            /// Every member, in declaration order — the ONE list callers may enumerate
            /// (the CLI's accepted values are built from it, so `--help` cannot drift from
            /// the enum). Generated from the same table as the variants themselves, so it
            /// is incapable of falling behind them.
            pub const ALL: &'static [SubjectKind] = &[ $( SubjectKind::$variant ),+ ];

            /// The wire word. Generated beside the variant it belongs to.
            pub fn as_str(self) -> &'static str {
                match self { $( SubjectKind::$variant => $wire ),+ }
            }
        }
    };
}

subject_kinds! {
    /// One event.
    Event => "event",
    /// A medication thread (`medication_id`). Later events on the thread inherit the grade
    /// automatically, because the effective grade is computed at READ.
    Thread => "thread",
    /// The whole chart. Deliberately the most effortful path: db/005 requires a rationale,
    /// and the blacklist can never author one (ADR-0062).
    Patient => "patient",
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
            .iter()
            .copied()
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
    fn no_two_subject_kinds_share_a_wire_word() {
        // COMPLETENESS is now structural: `ALL` and the enum expand from one table in
        // `subject_kinds!`, so there is no second list left to fall behind. (The previous
        // guard here tried to enforce that by hand with `assert_eq!(ALL.len(), 3)` and
        // could not — `ALL` was declared `[SubjectKind; 3]`, so the assertion compared a
        // constant to its own literal and never failed. That is why the macro exists.)
        //
        // What the macro does NOT catch is a duplicated wire word: `Thread => "event"` in
        // that table compiles perfectly. `try_from` returns the FIRST match, so
        // `--subject-kind thread` would silently author an EVENT-scoped grade — the wrong
        // confidentiality scope, applied quietly, on the append-only path where it can
        // only be corrected by overlay.
        let listed: Vec<&str> = SubjectKind::ALL.iter().map(|k| k.as_str()).collect();
        let mut unique = listed.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(
            unique.len(),
            listed.len(),
            "two subject kinds share a wire word: {listed:?}"
        );
    }

    #[test]
    fn every_subject_kind_round_trips_through_its_wire_word() {
        // `as_str` is what goes on the wire; `try_from` is what comes off a CLI argument.
        // If they ever disagree, a value this build emits is a value it cannot read back.
        for &k in SubjectKind::ALL {
            assert_eq!(SubjectKind::try_from(k.as_str()), Ok(k), "{}", k.as_str());
        }
    }

    #[test]
    fn an_unknown_subject_kind_is_refused_and_the_message_names_what_is_accepted() {
        // Refused at the CLI boundary only. The APPLY door must keep admitting an
        // unrecognised kind (ADR-0056/ADR-0062 — it is interpreted conservatively as
        // chart-wide), so this must never be mistaken for a wire-level rejection.
        //
        // The fixture is deliberately UNSPELLABLE as a variant. An earlier version used
        // "episode", which is a plausible future subject kind — the day someone adds it,
        // this test would have flipped from "an unknown kind is refused" to "refusing a
        // kind we support is correct", and defended the defect while staying green.
        let e = SubjectKind::try_from("\u{0}not-a-subject-kind")
            .expect_err("no variant can ever carry this wire word");
        for &k in SubjectKind::ALL {
            assert!(
                e.contains(k.as_str()),
                "the error must name {}: {e}",
                k.as_str()
            );
        }
    }

    #[test]
    fn provenance_reaches_the_wire_as_the_two_words_db048_stores() {
        // Pinned AT THE WIRE, not at the accessor: `as_str()` equalling its own literal
        // proves nothing, and `source` is only ever interesting as the bytes that land in
        // the signed body and the §3.13 twin. The `human` half is also covered by the
        // builder tests above; `advisory` has no other caller yet (that is a later slice),
        // so this is its only pin anywhere.
        for (p, word) in [
            (Provenance::Human, "human"),
            (Provenance::Advisory, "advisory"),
        ] {
            let a = SensitivityAssertion {
                subject_kind: SubjectKind::Thread,
                subject_id: uuid::Uuid::nil(),
                grade: GRADE_RESTRICTED,
                source: p,
                rationale: None,
            };
            assert_eq!(sensitivity_assertion_body(&a)["source"], word);
            assert!(
                render_sensitivity_twin(&a).contains(&format!("source: {word}")),
                "the twin is the reason a typo here is permanent"
            );
        }
    }

    #[test]
    fn the_ladder_constants_are_four_distinct_rungs() {
        // NOT `assert_eq!(GRADE_ROUTINE, "routine")` — that compares a const to its own
        // literal and cannot detect the drift its doc comment worries about. The real
        // lockstep is `the_ladder_matches_db048s_rank_function` in
        // `crates/cairn-node/tests/sensitivity_ladder.rs`, which feeds these four to
        // `cairn_sensitivity_rank` over a live connection.
        //
        // What is worth pinning HERE, with no database: that a copy-paste never collapsed
        // two rungs onto one word. Duplicating a rung would make a raise to the duplicated
        // grade a silent no-op against the rung it shadows — in the direction that
        // under-protects.
        let ladder = [
            GRADE_ROUTINE,
            GRADE_SENSITIVE,
            GRADE_RESTRICTED,
            GRADE_SEQUESTERED,
        ];
        let mut unique = ladder.to_vec();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(
            unique.len(),
            ladder.len(),
            "two rungs collapsed: {ladder:?}"
        );
        assert!(
            ladder.iter().all(|g| !g.trim().is_empty()),
            "db/048 refuses a blank grade at the door: {ladder:?}"
        );
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
