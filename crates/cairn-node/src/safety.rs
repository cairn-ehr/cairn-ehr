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
}
