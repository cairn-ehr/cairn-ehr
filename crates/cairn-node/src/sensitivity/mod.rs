//! §5.9 sensitivity — the operator surface over `cairn-event::sensitivity`'s pure wire
//! builders and db/048's read model (Task 8 of the sensitivity-stream plan, issue #232
//! part B). Follows `patient::register`'s shape: tick ONE HLC per event this call
//! actually authors (`crate::db::next_hlc`), build the body with the pure builder,
//! sign, submit through the validated `submit_event` door. See that module's own doc
//! for the fuller argument for why the tick-per-authored-event discipline matters — a
//! clock ticked for an event that never submits leaves a permanent, meaningless gap,
//! which is harmless, so the discipline is really about never ticking MORE than once
//! per event actually written, not about avoiding gaps altogether.
//!
//! # This module reports; it does not enforce
//!
//! `chart_sensitivity` answers "what grade would a client computing the §5.9 read model
//! see for this chart, and which subject produced it" — nothing more. No content is
//! withheld here, and nothing in this module may start withholding on the strength of a
//! grade: real enforcement needs CUSTODY NARROWING, a later slice currently blocked on
//! another issue (#232 part C). A projection-layer filter with no floor beneath it would
//! be security theatre — a client talking raw SQL walks straight past it — so this
//! surface stays honest about being read-only.
//!
//! The one thing the report MUST do is name which subject actually won: `chart-wide`,
//! `this thread`, `this event`, or `none`. A bare grade with no named source is not
//! fixable — if a whole chart reads as uniformly "sequestered", the person trying to
//! understand why needs to know whether that came from a chart-wide assertion (there is
//! exactly one thing to go and look at) or from every individual thread happening to be
//! graded that high (there are many). `subject_kind_phrase` is the one place that mapping
//! lives, so every caller reads the same phrase for the same wire value.
use cairn_event::sensitivity::{
    render_sensitivity_twin, render_withdrawal_twin, sensitivity_assertion_body,
    sensitivity_withdrawal_body, SensitivityAssertion, SensitivityWithdrawal, SubjectKind,
    SENSITIVITY_EVENT_TYPE, SENSITIVITY_SCHEMA_VERSION, WITHDRAWAL_EVENT_TYPE,
    WITHDRAWAL_SCHEMA_VERSION,
};
use cairn_event::{event_address, sign, sign_attestation, ClockGrade, EventBody, SigningKey};
use uuid::Uuid;

pub mod readback;
pub mod render;
pub mod report;

// Re-exported so `cairn_node::sensitivity::chart_sensitivity` and
// `cairn_node::sensitivity::ChartReport` keep working unchanged at every call site. The
// module split is an internal organisation decision; it is not an API change, and making
// callers move would turn a mechanical refactor into a reviewable one for no gain.
pub use readback::{subject_reading, withdraw_readback};
pub use report::{chart_sensitivity, ChartReport, ThreadGrade};

/// Map the `subject_kind` `cairn_effective_sensitivity` returns to the phrase a human
/// reads in a report. Pure and total (every input has an output, including one this
/// version has never seen) — the ONE place this mapping is allowed to exist, so a
/// second, independently-written copy elsewhere can never drift into naming the same
/// wire value differently.
///
/// `"patient"` reads as `"chart-wide"`, not `"patient"`: the wire word names WHERE the
/// grade is scoped in the data model (the `patient_id` column), but a report is read by
/// someone trying to understand the BLAST RADIUS of what they're looking at, and
/// "chart-wide" says that directly — "patient" could as easily be misread as "this is
/// about the patient's identity", which it is not.
///
/// `"coarsened"` is db/048 section 11's catch-all: an assertion that applies to this chart
/// but could not be matched to a specific subject — an unrecognised (future) subject kind,
/// or a KNOWN kind that is mis-targeted. The read model deliberately reports it under its
/// own name rather than echoing the row's raw kind, because echoing would say "this event"
/// for something that is in fact blurring the whole chart, which is the one confusion this
/// whole named-subject requirement exists to prevent.
///
/// An unrecognised value here (one a FUTURE db/048 returns and this build has never seen)
/// reads as coarsened to chart-wide — the same safe direction, put into words rather than
/// second-guessed.
pub fn subject_kind_phrase(kind: &str) -> &'static str {
    match kind {
        "patient" => "chart-wide",
        "thread" => "this thread",
        "event" => "this event",
        "coarsened" => "chart-wide (an assertion that names no subject on this chart)",
        "none" => "none",
        _ => "an unrecognised scope (read chart-wide)",
    }
}

/// Author a `sensitivity.grade.asserted` event: raise (or re-state) a confidentiality
/// grade over one event, one medication thread, or the whole chart.
///
/// `source` is hardcoded to `"human"` — this function is the manual, operator-driven
/// path (a clinician or records officer typed a grade), never the automatic blacklist
/// candidate db/048 section 13 computes (`cairn_sensitivity_candidate`, which an
/// advisory actor would assert with `source: "advisory"` — no such caller exists yet;
/// that is a later slice, not this one).
///
/// This is a thin builder, not a second door: a chart-wide raise (`SubjectKind::Patient`)
/// with `rationale: None` is built and submitted exactly as given, and the db/048
/// ceremony (`cairn_sensitivity_ceremony_ok`, invoked from db/005) refuses it at the real
/// door — see `assert_sensitivity_writes_a_well_formed_event_and_still_needs_a_rationale_chart_wide`
/// for the proof that this function does not quietly relax that rule.
#[allow(clippy::too_many_arguments)] // mirrors code_medication/register_patient's shape: one field per wire value, nothing bundleable without inventing a throwaway struct
pub async fn assert_sensitivity(
    client: &mut tokio_postgres::Client,
    sk: &SigningKey,
    kid: &str,
    node_origin: &str,
    patient: Uuid,
    subject_kind: SubjectKind,
    subject_id: Uuid,
    grade: &str,
    rationale: Option<&str>,
) -> anyhow::Result<Uuid> {
    // Tick the HLC only once we are committed to authoring exactly one event — the same
    // "tick per event actually authored" discipline `register_patient` documents at
    // length, trivial here because this function only ever authors one.
    let hlc = crate::db::next_hlc(client, node_origin).await?;
    let event_id = Uuid::now_v7();
    let a = SensitivityAssertion {
        subject_kind,
        subject_id,
        grade,
        source: "human",
        rationale,
    };
    let body = EventBody {
        event_id: event_id.to_string(),
        patient_id: patient.to_string(),
        event_type: SENSITIVITY_EVENT_TYPE.into(),
        schema_version: SENSITIVITY_SCHEMA_VERSION.into(),
        hlc,
        t_effective: None,
        signer_key_id: kid.into(),
        // "recorded", never "attested": raising a grade claims no clinical
        // responsibility, only that the node recorded the operator's instruction — the
        // same shape `register_patient`'s registration act uses, for the same reason
        // (see that module's doc). A raise needs no bound human author (db/048 section
        // 12); only a WITHDRAWAL does — see `withdraw_sensitivity` below.
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: sensitivity_assertion_body(&a),
        attachments: vec![],
        plaintext_twin: Some(render_sensitivity_twin(&a)),
        clock_grade: ClockGrade::SelfAsserted,
        safety: None,
    };
    let signed = sign(&body, sk)?;
    // Sensitivity bodies are plaintext by necessity (cairn-event::sensitivity's own module
    // doc: a node must READ the grade to coarsen, so sealing it under the key it governs
    // would be circular) — the plain 1-arg door, no DEK, no attestation token.
    client
        .execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await?;
    Ok(event_id)
}

/// Author a `sensitivity.grade-withdrawal.asserted` event: retract a standing grade.
/// Nothing is erased — the withdrawn assertion stays on the record, readable and
/// re-assertable (see `render_withdrawal_twin`).
///
/// `sk`/`kid` MUST be an ENROLLED HUMAN actor's key, not the plain node/device key:
/// db/048's ceremony refuses a withdrawal with no bound human author (ADR-0053) — removing
/// protection is accountable, raising one is not (the doc on `cairn_sensitivity_ceremony_ok`
/// explains the asymmetry). Mirroring the `identify --link` / `attest_thread_in_tx`
/// precedent (`medication/attestation.rs`), the SAME human key both signs the event
/// envelope AND mints the attestation token: there is no separate device signer here, so
/// the caller passes the loaded human key directly rather than a (device, human) pair.
/// The CLI's `sensitivity-withdraw` verb enforces "the key really is an enrolled human"
/// BEFORE calling this (a legible pre-check, mirroring `resolve_attester`) — this function
/// does not re-check it, and relies on the real door (db/005 step 4b / db/048 section 12)
/// as the actual enforcement point, matching the twelfth founding principle (the type
/// system permits the illegal state; the database refuses it).
pub async fn withdraw_sensitivity(
    client: &mut tokio_postgres::Client,
    sk: &SigningKey,
    kid: &str,
    node_origin: &str,
    patient: Uuid,
    withdraws_hex: &str,
    rationale: &str,
) -> anyhow::Result<Uuid> {
    let hlc = crate::db::next_hlc(client, node_origin).await?;
    let event_id = Uuid::now_v7();
    let w = SensitivityWithdrawal {
        withdraws_hex,
        rationale,
    };
    let body = EventBody {
        event_id: event_id.to_string(),
        patient_id: patient.to_string(),
        event_type: WITHDRAWAL_EVENT_TYPE.into(),
        schema_version: WITHDRAWAL_SCHEMA_VERSION.into(),
        hlc,
        t_effective: None,
        signer_key_id: kid.into(),
        // "attested" + a responsibility marker naming this SAME key: the ADR-0051 wire
        // shape the db/005 gate demands before it will accept the attestation token
        // below as satisfying the ceremony's bound-human-author requirement.
        contributors: serde_json::json!([{"actor_id": kid, "role": "attested",
                                          "responsibility": {"held_by": kid}}]),
        payload: sensitivity_withdrawal_body(&w),
        attachments: vec![],
        plaintext_twin: Some(render_withdrawal_twin(&w)),
        clock_grade: ClockGrade::SelfAsserted,
        safety: None,
    };
    let signed = sign(&body, sk)?;
    // The 3-arg door: signed envelope + a human attestation token binding this same key
    // as the responsible attester (the `submit_attested` idiom `tests/common/mod.rs`
    // uses throughout this cluster) + the verifying key bytes the door checks the token
    // against. No DEK: withdrawals are plaintext, same reasoning as the assertion above.
    let ca = event_address(&signed.signed_bytes);
    let token = sign_attestation(&ca, kid, "attested", sk)?;
    let vk = sk.verifying_key().to_bytes().to_vec();
    client
        .execute(
            "SELECT submit_event($1,$2,$3)",
            &[&signed.signed_bytes, &token, &vk],
        )
        .await?;
    Ok(event_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subject_kind_phrase_names_each_recognised_scope() {
        assert_eq!(subject_kind_phrase("patient"), "chart-wide");
        assert_eq!(subject_kind_phrase("thread"), "this thread");
        assert_eq!(subject_kind_phrase("event"), "this event");
        assert_eq!(subject_kind_phrase("none"), "none");
    }

    #[test]
    fn subject_kind_phrase_coarsens_an_unrecognised_scope_to_chart_wide_reading() {
        // A future peer's subject kind db/048 admits structurally (ADR-0056) but this
        // version does not understand — cairn_effective_sensitivity's own fallback arm
        // treats it as chart-wide-bounded, and the phrase must say so plainly rather than
        // printing the raw, meaningless wire token straight into a report.
        let phrase = subject_kind_phrase("episode");
        assert!(
            phrase.contains("chart-wide") || phrase.contains("unrecognised"),
            "an unknown scope must read as coarsened, not silently pass the raw token \
             through: {phrase}"
        );
    }
}
