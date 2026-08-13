//! §5.9 part B (ADR-0063) — the impure half of emission: two small queries.
//!
//! The PURE half (what each rung discloses) lives in `cairn_event::safety`. This module
//! only fetches what the pure half needs: the deployment's class for a coding, and the
//! disclosure rung the chart's current grade licenses.
//!
//! # Why the lookup runs HERE and never in a reader
//!
//! A coded drug's interaction class is a property of the code — a drug-knowledge lookup.
//! A reader that re-derived it would make the §5.9 safety floor depend on holding drugref
//! after all, which is precisely the failure ADR-0059 decision 4 / #294 exist to prevent.
//! The authoring node, by construction, had a coding authority in hand at that moment; so
//! the class is captured here, sealed with the body, and CARRIED.
//!
//! # A note on the binding idiom for a junior reader
//!
//! Every parameter below is bound as `$n::text::<type>`, never as a bare `$n::jsonb` or
//! `$n::uuid`. This crate does not enable tokio-postgres's `with-serde_json-1` /
//! `with-uuid-1` features, so `serde_json::Value` and `Uuid` have no wire encoding at all:
//! a bare cast fails CLIENT-side, before the statement ever reaches Postgres. Sending the
//! value as `text` and letting the database parse it is the repo-wide idiom (see
//! `tests/observed_evidence.rs`), and it keeps the failure — if any — on the database's
//! side where the error message is about the data rather than about the driver.
use anyhow::Context;
use cairn_event::medication::SubstanceCoding;
use cairn_event::safety::SafetyRung;
use uuid::Uuid;

/// This deployment's class + severity for a coding, or `None`.
///
/// `None` is the common case and is honest: `safety_class_map` ships empty, so a node with
/// no coding authority configured simply emits no signal. It never guesses.
///
/// Keyed on the (system, code) PAIR, because that is how `safety_class_map` is keyed: once
/// `drugref-clinical-drug` exists beside `drugref-moiety`, a bare-code key would collide
/// across composition-tree levels (db/049 section 5).
pub async fn lookup_class(
    client: &tokio_postgres::Client,
    coding: &SubstanceCoding<'_>,
) -> anyhow::Result<Option<(String, String)>> {
    let coding_json = serde_json::json!({ "system": coding.system, "code": coding.code });
    let rows = client
        .query(
            "SELECT class, severity FROM cairn_safety_class_candidate($1::text::jsonb)",
            &[&coding_json.to_string()],
        )
        .await?;
    Ok(rows.first().map(|r| (r.get(0), r.get(1))))
}

/// The disclosure rung the chart's currently-standing grade licenses for an event about to
/// be authored on `thread` (pass `None` when the event belongs to no thread).
///
/// Reads through `cairn_prospective_sensitivity` rather than `cairn_effective_sensitivity`
/// because the event does not exist yet — see db/049 section 6. That function always
/// returns exactly one row (a `LEFT JOIN LATERAL` over a constant), so `query_one` is safe
/// and a chart with no standing assertion reads `routine` rather than "no row".
///
/// KNOWN RACE, declared rather than defended against: this read and the subsequent submit
/// are separate statements, so a grade raised in between yields a rung one step too fine.
/// The window cannot be closed by moving the decision into `submit_event` — the rung must
/// be inside the SIGNED bytes, and signing happens in this daemon where the key lives. The
/// read model re-coarsens on every node that later holds the grade (db/049 section 7), so
/// the consequence is bounded: a stale rung is displayed at the grade the READER's node
/// licenses, never at the stale one.
pub async fn prospective_rung(
    client: &tokio_postgres::Client,
    patient: Uuid,
    thread: Option<Uuid>,
) -> anyhow::Result<SafetyRung> {
    let rung: String = client
        .query_one(
            "SELECT cairn_safety_rung_for_rank(cairn_sensitivity_rank(g.grade))
             FROM cairn_prospective_sensitivity($1::text::uuid, $2::text::uuid) g",
            &[&patient.to_string(), &thread.map(|t| t.to_string())],
        )
        .await?
        .get(0);
    Ok(rung_from_name(&rung))
}

/// Map db/049's rung NAME onto this build's ladder. Pure and total, so it is testable
/// without a database (house rule 4).
///
/// The `_` arm is the decision, not an oversight: an unrecognised name — including a rung
/// a FUTURE db/049 interposes that this build predates — is treated as the coarsest.
/// Disclosing on a value this build cannot interpret is the one direction that cannot be
/// undone, because bytes already on the wire cannot be recalled. This is the same
/// safe-default-by-omission discipline db/049 sections 1-3 use in SQL.
pub fn rung_from_name(name: &str) -> SafetyRung {
    match name {
        "precise" => SafetyRung::Precise,
        "kind" => SafetyRung::Kind,
        _ => SafetyRung::Existence,
    }
}

/// Read a precise `{class, severity}` claim back out of a clear payload, but ONLY when it
/// is usable in the clear. Pure and total over any JSON shape.
///
/// # Why this guard exists (it is a safety property, not tidiness)
///
/// db/049 section 4 refuses a CLEAR signal whose `precise` rung carries a blank class, and
/// refuses any signal whose `severity` key is present but blank. That refusal happens at
/// the strict door — which would take the *medication assertion* down with it. So a single
/// misconfigured `safety_class_map` row (the columns are `NOT NULL` but not non-blank)
/// could cancel clinical writes for every drug it names.
///
/// ADR-0060 forbids exactly that: a defect in a de-identified ADVISORY field must never
/// invalidate clinical content. "The system may fail to record an order, but it may never
/// cancel one." So a half-formed claim yields `None` — no clear signal — and the clinical
/// event lands unharmed. The sealed tier keeps whatever the builder wrote, because the
/// sealed side has no floor to trip and a custody-holder reading a blank class learns the
/// same thing this node knows.
///
/// Reading the claim back out of the payload (rather than threading the typed
/// `PreciseSafety` down here) keeps the seam TOTAL over any builder that writes a precise
/// claim, present or future — including one that has not been invented yet.
pub fn usable_precise_claim(payload_safety: &serde_json::Value) -> Option<(String, String)> {
    let nonblank = |k: &str| {
        payload_safety
            .get(k)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    Some((nonblank("class")?, nonblank("severity")?))
}

/// One de-identified safety line, already coarsened by db/049's read model.
///
/// `class` and `severity` are `Option` because the RUNG decides whether they exist at all —
/// they are not "missing data". A `None` class at rung `existence` is the mechanism working,
/// not a gap.
pub struct SafetyLine {
    pub event_id: Uuid,
    pub rung: String,
    pub class: Option<String>,
    pub severity: Option<String>,
    pub event_type: String,
    /// The §5.9 grade that produced this coarseness…
    pub grade: String,
    /// …and WHICH subject won it (ADR-0062 decision 8 control 3: a grade with no named
    /// source cannot be fixed, because nobody can tell one thing to go and look at from
    /// twenty).
    pub subject_kind: String,
}

/// Every standing safety signal on a chart, coarsest-safe and already de-identified.
///
/// A pure read: no signing key, no HLC tick, nothing authored. One query, so a UI opening a
/// chart pays a single round trip.
pub async fn chart_safety(
    client: &tokio_postgres::Client,
    patient: Uuid,
) -> anyhow::Result<Vec<SafetyLine>> {
    // T3-A: this crate enables neither `with-uuid-1` nor `with-serde_json-1` on
    // tokio-postgres, so `Uuid` has no `FromSql` impl and a bare `r.get::<_, Uuid>(0)`
    // fails to COMPILE, before any statement reaches the database. `event_id` is cast to
    // `text` in the query and parsed back on this side; every other column is already a
    // plain text/varchar and reads through `FromSql<String>` unmodified.
    let rows = client
        .query(
            "SELECT event_id::text, rung, class, severity, event_type, grade, subject_kind
             FROM cairn_patient_safety($1::text::uuid)",
            &[&patient.to_string()],
        )
        .await?;
    rows.iter()
        .map(|r| {
            let event_id_text: String = r.get(0);
            Ok(SafetyLine {
                event_id: event_id_text
                    .parse()
                    .with_context(|| format!("event_id {event_id_text:?} is not a uuid"))?,
                rung: r.get(1),
                class: r.get(2),
                severity: r.get(3),
                event_type: r.get(4),
                grade: r.get(5),
                subject_kind: r.get(6),
            })
        })
        .collect()
}

/// The human sentence for one line — §5.9's warning that NAMES NOTHING.
///
/// Pure and total, so the CLI and any future UI cannot phrase the same signal differently.
/// The event TYPE is already plaintext on the row, so naming it discloses nothing new and
/// is what makes the middle rung read as "confidential medication" rather than
/// "confidential something".
pub fn render_safety_line(line: &SafetyLine) -> String {
    let noun = if line.event_type.starts_with("clinical.medication") {
        "medication"
    } else {
        "content"
    };
    match (line.class.as_deref(), line.severity.as_deref()) {
        (Some(class), Some(sev)) => format!("⚠ {sev} — {class}"),
        (None, Some(sev)) => format!("⚠ {sev} — confidential {noun}, break glass to view"),
        _ => format!("⚠ confidential {noun} — break glass to view"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_well_formed_claim_is_read_back_whole() {
        let v = serde_json::json!({"class": "rh-sensitizing", "severity": "high"});
        assert_eq!(
            usable_precise_claim(&v),
            Some(("rh-sensitizing".to_string(), "high".to_string()))
        );
    }

    #[test]
    fn a_blank_or_missing_half_yields_nothing_rather_than_a_refusable_signal() {
        // THE POINT OF THIS TEST: db/049 section 4 refuses a clear signal whose `precise`
        // rung carries a blank class, or whose severity is present-but-blank — and that
        // refusal happens at the strict door, which would take the MEDICATION ASSERTION
        // down with it. A misconfigured `safety_class_map` row must never be able to
        // cancel a clinical write (ADR-0060). So a half-formed claim emits nothing.
        for v in [
            serde_json::json!({"class": "", "severity": "high"}),
            serde_json::json!({"class": "  ", "severity": "high"}),
            serde_json::json!({"class": "rh-sensitizing", "severity": ""}),
            serde_json::json!({"class": "rh-sensitizing"}),
            serde_json::json!({"severity": "high"}),
            serde_json::json!({"class": 7, "severity": "high"}),
            serde_json::json!("not an object"),
            serde_json::Value::Null,
        ] {
            assert_eq!(usable_precise_claim(&v), None, "half-formed claim: {v}");
        }
    }

    #[test]
    fn every_named_rung_round_trips_and_the_unknown_one_discloses_nothing() {
        // Round-trip against `as_str()` rather than against three literals, so a rename in
        // cairn-event cannot leave this mapping silently wrong.
        for r in [SafetyRung::Precise, SafetyRung::Kind, SafetyRung::Existence] {
            assert_eq!(rung_from_name(r.as_str()), r, "round trip for {r:?}");
        }
        // The arm that a database test cannot reach today: db/049's
        // `cairn_safety_rung_for_rank` only ever returns the three named rungs, so this
        // case exists purely for a future migration that adds a fourth.
        assert_eq!(
            rung_from_name("a-rung-this-build-predates"),
            SafetyRung::Existence,
            "an unrecognised rung must disclose nothing, never everything"
        );
    }

    /// A throwaway `SafetyLine` for `render_safety_line` tests. `grade` and `subject_kind`
    /// are fixed to recognisable, un-mistakable strings and `event_id` is fresh-minted
    /// (not cryptographic material — house rule 6 does not apply to a fixture UUID) so
    /// every test below can assert render_safety_line's output never contains ANY of the
    /// three: those fields exist on `SafetyLine` for the CLI to print SEPARATELY (see
    /// `main.rs`'s `Cmd::PatientSafety` handler — "(grade {}, winning subject: {})"), and a
    /// future edit that folded them into this function's own string would silently start
    /// disclosing scope information this pure seam is not licensed to name.
    fn line(class: Option<&str>, severity: Option<&str>, event_type: &str) -> SafetyLine {
        SafetyLine {
            event_id: Uuid::now_v7(),
            rung: "irrelevant-to-rendering".to_string(),
            class: class.map(str::to_string),
            severity: severity.map(str::to_string),
            event_type: event_type.to_string(),
            grade: "sequestered".to_string(),
            subject_kind: "patient".to_string(),
        }
    }

    /// Every assertion below checks NEGATIVE space too (`assert!(!rendered.contains(...))`),
    /// not just the happy string — a match arm that starts also printing `grade` or
    /// `subject_kind`, or that leaks a class/severity the rung does not license, would slip
    /// past a test that only pinned the expected substring.
    fn assert_never_leaks_scope_fields(rendered: &str, l: &SafetyLine) {
        assert!(
            !rendered.contains(&l.grade),
            "render_safety_line must not print the grade itself — the CLI prints it \
             separately: {rendered:?}"
        );
        assert!(
            !rendered.contains(&l.subject_kind),
            "render_safety_line must not print the winning subject — the CLI prints it \
             separately: {rendered:?}"
        );
        assert!(
            !rendered.contains(&l.event_id.to_string()),
            "render_safety_line must not print the event id: {rendered:?}"
        );
    }

    #[test]
    fn render_at_precise_names_both_class_and_severity() {
        // db/049 only ever hands the rung "precise" a (Some, Some) pair, so this pins the
        // ONE combination the read model actually produces at that rung.
        let l = line(
            Some("rh-sensitizing"),
            Some("high"),
            "clinical.medication.assert",
        );
        let rendered = render_safety_line(&l);
        assert_eq!(rendered, "⚠ high — rh-sensitizing");
        // Absence: this is the one rung licensed to disclose, but it must disclose
        // EXACTLY the two named fields — never the confidential-content fallback text.
        assert!(!rendered.contains("confidential"));
        assert!(!rendered.contains("break glass"));
        assert_never_leaks_scope_fields(&rendered, &l);
    }

    #[test]
    fn render_at_kind_names_severity_but_withholds_class() {
        // db/049 only ever hands the rung "kind" a (None, Some) pair.
        let l = line(None, Some("critical"), "clinical.medication.assert");
        let rendered = render_safety_line(&l);
        assert_eq!(
            rendered,
            "⚠ critical — confidential medication, break glass to view"
        );
        // Absence: the precise class from the OTHER test must never appear here — this is
        // what would catch a match-arm merge that accidentally carried a class value
        // through at the middle rung.
        assert!(!rendered.contains("rh-sensitizing"));
        assert_never_leaks_scope_fields(&rendered, &l);
    }

    #[test]
    fn render_at_existence_names_neither_class_nor_severity() {
        // db/049 only ever hands the rung "existence" a (None, None) pair.
        let l = line(None, None, "clinical.medication.assert");
        let rendered = render_safety_line(&l);
        assert_eq!(rendered, "⚠ confidential medication — break glass to view");
        // Absence: neither the precise class NOR the precise severity from the other two
        // tests may leak in at the coarsest rung.
        assert!(!rendered.contains("rh-sensitizing"));
        assert!(!rendered.contains("critical"));
        assert!(!rendered.contains("high"));
        assert_never_leaks_scope_fields(&rendered, &l);
    }

    #[test]
    fn render_names_medication_only_for_a_clinical_medication_event_type() {
        // This is what makes the middle/coarsest rung read as "confidential medication"
        // rather than "confidential content" — the event TYPE is already plaintext on the
        // row (never sealed), so naming it discloses nothing new (see the function's own
        // doc). Table-driven over both rungs that use the noun, and over a handful of
        // event types on each side of the `clinical.medication` prefix boundary.
        for event_type in [
            "clinical.medication.assert",
            "clinical.medication.cessation",
            "clinical.medication.dose-change",
        ] {
            let existence = render_safety_line(&line(None, None, event_type));
            assert!(
                existence.contains("medication") && !existence.contains("content"),
                "{event_type:?} at existence must read as medication: {existence:?}"
            );
            let kind = render_safety_line(&line(None, Some("high"), event_type));
            assert!(
                kind.contains("medication") && !kind.contains("content"),
                "{event_type:?} at kind must read as medication: {kind:?}"
            );
        }
        for event_type in [
            "clinical.note.add",
            "identity.link",
            "demographic.assert",
            "",
        ] {
            let existence = render_safety_line(&line(None, None, event_type));
            assert!(
                existence.contains("content") && !existence.contains("medication"),
                "{event_type:?} at existence must read as content, never medication: \
                 {existence:?}"
            );
            let kind = render_safety_line(&line(None, Some("high"), event_type));
            assert!(
                kind.contains("content") && !kind.contains("medication"),
                "{event_type:?} at kind must read as content, never medication: {kind:?}"
            );
        }
    }
}
