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
    pub fn as_str(self) -> &'static str {
        match self {
            SubjectKind::Event => "event",
            SubjectKind::Thread => "thread",
            SubjectKind::Patient => "patient",
        }
    }
}

/// A single graded claim. Raising is frictionless by design — err toward confidential.
pub struct SensitivityAssertion<'a> {
    pub subject_kind: SubjectKind,
    pub subject_id: Uuid,
    /// Open vocabulary: db/048 ranks the named ladder and treats anything else as MAX.
    pub grade: &'a str,
    /// `human` | `advisory` — the provenance of the tag, never an authority claim.
    pub source: &'a str,
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
        "source": a.source,
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
        a.grade, subject, a.subject_id, a.source
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
            source: "human",
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
            source: "human",
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
            source: "advisory",
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
