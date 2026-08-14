//! Medication assertion — mints the immortal `medication_id` thread (§3.3).
//! Device-additive: signed by the node/clinician key with a `recorded`
//! contributor and NO responsibility attestation (mirrors identify.rs).
use cairn_event::medication::{
    medication_assertion_body, render_medication_twin, MedicationAssertion,
};
// Re-exported (not just used internally): a caller building `coding_from_parts`'/
// `AssertMedicationInput::coding`'s value needs to name this type too (final-review
// finding 5) — without `pub` here, `mod.rs`'s `pub use assert::{.., SubstanceCoding}`
// cannot reach it, forcing every caller to depend on cairn-event directly instead.
pub use cairn_event::medication::SubstanceCoding;
use cairn_event::{EventBody, Hlc, SigningKey};
use uuid::Uuid;

const MEDICATION_SCHEMA_VERSION: &str = "clinical.medication/1";

/// The clinician-supplied fields of a medication statement. `term` is required;
/// everything else is an honest Option (unknown when None).
pub struct AssertMedicationInput<'a> {
    pub term: &'a str,
    /// Drug-identity coding (ADR-0059) — `None` when nobody has coded it yet.
    pub coding: Option<SubstanceCoding<'a>>,
    pub formulation: Option<&'a str>,
    pub dose_amount: Option<&'a str>,
    pub dose_unit: Option<&'a str>,
    pub sig: Option<&'a str>,
    pub info_source: &'a str,
    pub started: Option<&'a str>,
    pub started_precision: Option<&'a str>,
}

/// Advisory Rust-side guard mirroring the DB floor: refuse a blank term with a
/// clinical message. The DB floor is the real, unbypassable enforcement.
pub fn validate_term(term: &str) -> anyhow::Result<()> {
    if term.trim().is_empty() {
        anyhow::bail!(
            "medication term must not be empty: record WHAT the patient takes (even if vague)"
        );
    }
    Ok(())
}

/// Turn three independent CLI flags into an all-or-nothing coding claim.
///
/// Pure and total: the three flags are supplied together or not at all. A *partial*
/// triple is a caller mistake, and this is where it is caught — the DB floor would
/// refuse it too, but at the source the message can name the flag that is missing.
/// Blank is not a value: `--coding-code "  "` is a missing code, not an empty one.
pub fn coding_from_parts<'a>(
    system: Option<&'a str>,
    code: Option<&'a str>,
    display: Option<&'a str>,
) -> anyhow::Result<Option<SubstanceCoding<'a>>> {
    let parts = [
        ("coding-system", system),
        ("coding-code", code),
        ("coding-display", display),
    ];
    let missing: Vec<&str> = parts
        .iter()
        .filter(|(_, v)| v.is_none_or(|s| s.trim().is_empty()))
        .map(|(name, _)| *name)
        .collect();
    if missing.len() == parts.len() {
        return Ok(None); // not-yet-coded: a permanently valid state (principle 4)
    }
    if !missing.is_empty() {
        anyhow::bail!(
            "a drug coding needs all three parts; missing or blank: --{}. \
             Omit all three to record the medication uncoded (ADR-0059)",
            missing.join(", --")
        );
    }
    Ok(Some(SubstanceCoding {
        system: system.expect("checked non-empty above").trim(),
        code: code.expect("checked non-empty above").trim(),
        display: display.expect("checked non-empty above").trim(),
    }))
}

/// Assemble the signed `clinical.medication.asserted` EventBody. Pure — the caller
/// mints `event_id`/`medication_id`, supplies the HLC, and signs.
///
/// `safety` is the §5.9 precise safety claim, already looked up by the caller (ADR-0063).
/// Passed IN rather than looked up here so this function stays PURE and unit-testable
/// without a database (house rule 4 / the §9 blast-radius rule). `None` — an uncoded
/// medication, or a coding this deployment's class map has no row for — writes no `safety`
/// key at all, so the event's bytes are identical to the pre-ADR-0063 shape.
#[allow(clippy::too_many_arguments)] // one parameter per wire value; nothing bundleable without inventing a throwaway struct
pub fn build_assert_body(
    event_id: Uuid,
    medication_id: Uuid,
    patient: Uuid,
    input: &AssertMedicationInput<'_>,
    node_kid: &str,
    hlc: Hlc,
    safety: Option<cairn_event::safety::PreciseSafety<'_>>,
) -> EventBody {
    let mid = medication_id.to_string();
    let a = MedicationAssertion {
        medication_id: &mid,
        term: input.term,
        // SubstanceCoding is Copy (borrowed &str fields only), so the Option copies
        // whole — no need to rebuild it field-by-field (house rule 4).
        coding: input.coding,
        formulation: input.formulation,
        dose_amount: input.dose_amount,
        dose_unit: input.dose_unit,
        sig: input.sig,
        info_source: input.info_source,
        started: input.started,
        started_precision: input.started_precision,
        safety,
    };
    EventBody {
        event_id: event_id.to_string(),
        patient_id: patient.to_string(),
        event_type: "clinical.medication.asserted".into(),
        schema_version: MEDICATION_SCHEMA_VERSION.into(),
        hlc,
        t_effective: None,
        signer_key_id: node_kid.into(),
        contributors: serde_json::json!([{"actor_id": node_kid, "role": "recorded"}]),
        payload: medication_assertion_body(&a),
        attachments: vec![],
        plaintext_twin: Some(render_medication_twin(&a)),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    }
}

/// Record a medication the patient takes/took. Mints and returns the thread's
/// `medication_id`. Device-additive when `attest` is `None` (goes through the 1-arg
/// submit door, auto-commit — unchanged from the pre-attestation path). When `attest`
/// is `Some`, the assert AND the human's responsibility attestation for the
/// newly-minted thread run in ONE transaction, so the attestation's commitment sees
/// the assert event just submitted (mirrors `identify_patient`'s identify+link atomic
/// shape). A rejected attestation rolls the assert back with it.
#[allow(clippy::too_many_arguments)] // signer + node context + patient/input/author/attest, mirrors dose orchestrators
pub async fn assert_medication(
    client: &mut tokio_postgres::Client,
    node_sk: &SigningKey,
    node_kid: &str,
    node_origin: &str,
    patient: Uuid,
    input: &AssertMedicationInput<'_>,
    author: Option<&crate::medication::AuthorParams<'_>>,
    attest: Option<&crate::medication::AttestParams<'_>>,
) -> anyhow::Result<Uuid> {
    validate_term(input.term)?;
    // Mint HLCs up front (self-committing; a rolled-back submit just leaves a gap —
    // the HLC is monotonic and gaps are allowed, exactly like identify_patient).
    let verb_hlc = crate::db::next_hlc(client, node_origin).await?;
    let event_id = Uuid::now_v7();
    let medication_id = Uuid::now_v7();
    // §5.9 part B (ADR-0063): the class is looked up on the CODING node, PRE-SEAL, because
    // it is a drug-knowledge lookup a reader cannot redo without a drug database — and
    // making the safety floor depend on holding one would defeat the floor (#294 /
    // ADR-0059 decision 4). An uncoded medication, or a coding this deployment's map has no
    // row for, yields None: both are honest absences and emit no signal at all, rather than
    // a content-free marker that would manufacture a warning from nothing.
    //
    // Held in a local `class` first because `PreciseSafety` BORROWS its two strings; the
    // owned pair must outlive the body construction below.
    //
    // A lookup that ERRORS is treated as "no class", never propagated: an advisory field
    // must never be able to fail a clinical write (ADR-0060 — the system may fail to record
    // an order, but it may never cancel one). Falling back is the WITHHOLDING direction, so
    // the worst case is a missing decoration on a recorded medication, never a lost
    // medication. See `crate::safety::advisory_or_withheld` for the full argument.
    let class = match input.coding.as_ref() {
        Some(coding) => crate::safety::advisory_or_withheld(
            crate::safety::lookup_class(client, coding).await,
            None,
            "the safety class lookup for a medication assert",
        ),
        None => None,
    };
    let safety = class
        .as_ref()
        .map(|(class, severity)| cairn_event::safety::PreciseSafety { class, severity });
    let body = build_assert_body(
        event_id,
        medication_id,
        patient,
        input,
        node_kid,
        verb_hlc,
        safety,
    );
    // ADR-0052 seal-at-write: the clear body is sealed, signed, and submitted through the
    // ONE strict door by seal_sign_submit — which also runs the atomic author-time
    // attestation when `attest` is Some (it vouches for the thread named in the body's
    // payload.medication_id), and rewrites the body to human-signed authorship when
    // `author` is Some (#204 / ADR-0053; the node still keeps DEK custody). We return the
    // THREAD id, not the content event id.
    crate::medication::sealed_submit::seal_sign_submit(client, node_sk, body, author, attest)
        .await?;
    Ok(medication_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coding_from_parts_accepts_all_three_or_none() {
        let none = coding_from_parts(None, None, None).expect("uncoded is valid");
        assert!(none.is_none(), "no flags at all means not-yet-coded");

        let some = coding_from_parts(
            Some("drugref-moiety"),
            Some("0f8c4b1e-1b7a-5c2d-9a3e-2b6f7c8d9e01"),
            Some("atorvastatin"),
        )
        .expect("a complete triple is valid")
        .expect("a complete triple yields a coding");
        assert_eq!(some.system, "drugref-moiety");
        assert_eq!(some.display, "atorvastatin");
    }

    #[test]
    fn coding_from_parts_refuses_a_partial_triple() {
        // A half-supplied coding must never reach the door: the DB floor would refuse
        // it anyway, but the caller deserves the error at the source, naming the gap.
        let e = coding_from_parts(Some("drugref-moiety"), Some("abc"), None)
            .expect_err("a partial coding must be refused");
        let msg = e.to_string();
        assert!(
            msg.contains("coding-display"),
            "the error names the missing flag: {msg}"
        );
    }

    #[test]
    fn coding_from_parts_refuses_a_blank_field() {
        let e = coding_from_parts(Some("drugref-moiety"), Some("   "), Some("atorvastatin"))
            .expect_err("a blank field is not a value");
        assert!(e.to_string().contains("coding-code"), "{e}");
    }
}
