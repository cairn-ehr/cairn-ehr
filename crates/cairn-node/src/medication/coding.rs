//! Medication coding *overlays* — the node authoring surface for coding as a
//! separately-authored act (ADR-0059 decision 3, slice 6b).
//!
//! Slice 6a can only code a medication *inline*, at assertion time, by the clinician who
//! recorded it. These two verbs let whoever actually codes — a pharmacist, a professional
//! coder — code a thread later, correct a wrong coding, or **strike** a coding back to
//! honest not-yet-coded. Signed and (with `--author-as`) authored as *theirs* (ADR-0053),
//! never silently as the clinician's.
//!
//! Offline-first, like every other medication verb: neither the thread being coded nor the
//! event being corrected has to be present locally — both may replicate later, or never.
use cairn_event::medication::{
    medication_coding_body, medication_coding_correction_body,
    render_medication_coding_correction_twin, render_medication_coding_twin, CodingClaim,
    MedicationCoding, MedicationCodingCorrection, SubstanceCoding,
};
use cairn_event::{EventBody, Hlc, SigningKey};
use uuid::Uuid;

const CODING_SCHEMA_VERSION: &str = "clinical.medication-coding/1";
const CODING_CORRECTION_SCHEMA_VERSION: &str = "clinical.medication-coding-correction/1";

/// Code a thread that was not coded inline.
pub struct CodeMedicationInput<'a> {
    pub coding: SubstanceCoding<'a>,
}

/// Correct a coding claim: replace it, or strike it back to not-yet-coded.
pub struct CorrectCodingInput<'a> {
    /// The event whose coding this fixes (a prior coding overlay, or the assertion itself
    /// when the coding was inline). Not required to be present locally.
    pub corrects: Uuid,
    /// What this correction claims — a `CodingClaim`, so "both" and "neither" cannot be
    /// spelled at all (they used to be an `Option` beside a `bool`, which the body builder
    /// then had to normalize by silently dropping one of them).
    pub claim: CodingClaim<'a>,
    /// Why this correction was made (audit).
    pub note: Option<&'a str>,
}

/// Turn a CLI's two independent switches — the three `--coding-*` flags and `--strike` —
/// into the one claim a correction is allowed to make, refusing the two incoherent
/// combinations at the source.
///
/// The type makes the invalid states unrepresentable from here ON; this is the boundary
/// where they can still arrive, because a command line has no such guarantee. The in-DB
/// floor remains the real, unbypassable enforcement for anything that did not come through
/// this function (principle 12) — a peer's bytes never touch it. Pure: no I/O, so it is
/// cheap to call and cheap to test.
pub fn coding_claim_from_parts<'a>(
    coding: Option<SubstanceCoding<'a>>,
    strike: bool,
) -> anyhow::Result<CodingClaim<'a>> {
    match (coding, strike) {
        (Some(_), true) => anyhow::bail!(
            "a coding correction cannot both replace and strike: supply the three --coding-* flags OR --strike, not both"
        ),
        (None, false) => anyhow::bail!(
            "a coding correction must either carry a replacement (all three --coding-* flags) or --strike it back to not-yet-coded"
        ),
        (Some(k), false) => Ok(CodingClaim::Replace(k)),
        (None, true) => Ok(CodingClaim::Strike),
    }
}

/// Assemble the signed `clinical.medication-coding.asserted` EventBody. Pure.
///
/// `safety` is the §5.9 precise safety claim, already looked up by the caller (ADR-0063) —
/// passed in for the same reason `build_assert_body` takes it: this function stays pure and
/// unit-testable without a database.
#[allow(clippy::too_many_arguments)] // one parameter per wire value, mirroring build_assert_body
pub fn build_coding_body(
    event_id: Uuid,
    medication_id: Uuid,
    patient: Uuid,
    input: &CodeMedicationInput<'_>,
    node_kid: &str,
    hlc: Hlc,
    safety: Option<cairn_event::safety::PreciseSafety<'_>>,
) -> EventBody {
    let mid = medication_id.to_string();
    let c = MedicationCoding {
        medication_id: &mid,
        coding: input.coding,
        safety,
    };
    EventBody {
        event_id: event_id.to_string(),
        patient_id: patient.to_string(),
        event_type: "clinical.medication-coding.asserted".into(),
        schema_version: CODING_SCHEMA_VERSION.into(),
        hlc,
        t_effective: None,
        signer_key_id: node_kid.into(),
        contributors: serde_json::json!([{"actor_id": node_kid, "role": "recorded"}]),
        payload: medication_coding_body(&c),
        attachments: vec![],
        plaintext_twin: Some(render_medication_coding_twin(&c)),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    }
}

/// Assemble the signed `clinical.medication-coding-correction.asserted` EventBody. Pure.
pub fn build_coding_correction_body(
    event_id: Uuid,
    medication_id: Uuid,
    patient: Uuid,
    input: &CorrectCodingInput<'_>,
    node_kid: &str,
    hlc: Hlc,
) -> EventBody {
    let mid = medication_id.to_string();
    let target = input.corrects.to_string();
    let c = MedicationCodingCorrection {
        medication_id: &mid,
        corrects: &target,
        claim: input.claim,
        note: input.note,
    };
    EventBody {
        event_id: event_id.to_string(),
        patient_id: patient.to_string(),
        event_type: "clinical.medication-coding-correction.asserted".into(),
        schema_version: CODING_CORRECTION_SCHEMA_VERSION.into(),
        hlc,
        t_effective: None,
        signer_key_id: node_kid.into(),
        contributors: serde_json::json!([{"actor_id": node_kid, "role": "recorded"}]),
        payload: medication_coding_correction_body(&c),
        attachments: vec![],
        plaintext_twin: Some(render_medication_coding_correction_twin(&c)),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    }
}

/// Code an existing medication thread. Returns the coding event's id — the value a later
/// correction passes as `corrects`. Offline-first (no local existence check on the
/// thread). `author` is ADR-0053's separable human-authorship overlay (`None` ⇒
/// device-additive, the node signs and is the sole `recorded` contributor); `attest` is
/// the separate ADR-0049 responsibility overlay for the thread.
#[allow(clippy::too_many_arguments)] // signer + node context + patient/thread/input/author/attest, mirrors the sibling orchestrators
pub async fn code_medication(
    client: &mut tokio_postgres::Client,
    node_sk: &SigningKey,
    node_kid: &str,
    node_origin: &str,
    patient: Uuid,
    medication_id: Uuid,
    input: &CodeMedicationInput<'_>,
    author: Option<&crate::medication::AuthorParams<'_>>,
    attest: Option<&crate::medication::AttestParams<'_>>,
) -> anyhow::Result<Uuid> {
    let hlc = crate::db::next_hlc(client, node_origin).await?;
    let event_id = Uuid::now_v7();
    // §5.9 part B (ADR-0063), same seam and same reason as `assert_medication`: a coding
    // OVERLAY is authored by whoever codes it — a pharmacist, a professional coder — and
    // that node is again the one holding a coding authority. The class is captured here,
    // pre-seal, and travels; no reader ever re-derives it.
    let class = crate::safety::lookup_class(client, &input.coding).await?;
    let safety = class
        .as_ref()
        .map(|(class, severity)| cairn_event::safety::PreciseSafety { class, severity });
    let body = build_coding_body(
        event_id,
        medication_id,
        patient,
        input,
        node_kid,
        hlc,
        safety,
    );
    // ADR-0052 seal-at-write: seal + sign + submit through the ONE strict door.
    crate::medication::sealed_submit::seal_sign_submit(client, node_sk, body, author, attest)
        .await?;
    Ok(event_id)
}

/// Correct (replace or strike) a thread's coding. Returns the correction event's id.
/// Offline-first: neither the thread nor the corrected event must exist locally.
///
/// §5.9 (ADR-0063): THIS VERB DELIBERATELY EMITS NO SAFETY SIGNAL, and the omission is a
/// decision rather than an oversight. A `CodingClaim::Strike` carries no coding, so there
/// is nothing to look up. A `CodingClaim::Replace` does carry one, but a correction's
/// safety consequences ride the THREAD — the question "what does this chart's medication
/// list now imply" is a thread-rollup, and rolling a signal up across a thread is a
/// separate design question this slice does not open. Attaching a per-event signal here
/// would answer it by accident, in the direction that is hardest to undo (a published
/// class cannot be recalled).
#[allow(clippy::too_many_arguments)] // as above
pub async fn correct_medication_coding(
    client: &mut tokio_postgres::Client,
    node_sk: &SigningKey,
    node_kid: &str,
    node_origin: &str,
    patient: Uuid,
    medication_id: Uuid,
    input: &CorrectCodingInput<'_>,
    author: Option<&crate::medication::AuthorParams<'_>>,
    attest: Option<&crate::medication::AttestParams<'_>>,
) -> anyhow::Result<Uuid> {
    let hlc = crate::db::next_hlc(client, node_origin).await?;
    let event_id = Uuid::now_v7();
    let body = build_coding_correction_body(event_id, medication_id, patient, input, node_kid, hlc);
    crate::medication::sealed_submit::seal_sign_submit(client, node_sk, body, author, attest)
        .await?;
    Ok(event_id)
}

#[cfg(test)]
mod coding_build_tests {
    use super::*;
    use cairn_event::Hlc;

    const MOIETY_ATORVASTATIN: &str = "0f8c4b1e-1b7a-5c2d-9a3e-2b6f7c8d9e01";

    fn hlc() -> Hlc {
        Hlc {
            wall: 1_700_000_000_000,
            counter: 0,
            node_origin: "test-node".into(),
        }
    }

    fn coding() -> SubstanceCoding<'static> {
        SubstanceCoding {
            system: "drugref-moiety",
            code: MOIETY_ATORVASTATIN,
            display: "atorvastatin",
        }
    }

    #[test]
    fn a_replacement_or_a_strike_becomes_the_matching_claim() {
        assert!(matches!(
            coding_claim_from_parts(Some(coding()), false).expect("a replacement is valid"),
            CodingClaim::Replace(k) if k.display == "atorvastatin"
        ));
        assert!(matches!(
            coding_claim_from_parts(None, true).expect("a strike is valid"),
            CodingClaim::Strike
        ));
    }

    #[test]
    fn neither_is_refused_at_the_source() {
        // The DB floor refuses this too, but the caller deserves the error where the
        // mistake was made, naming the two ways out.
        let e = coding_claim_from_parts(None, false).expect_err("neither must be refused");
        let msg = e.to_string();
        assert!(
            msg.contains("--strike"),
            "the error names the escape: {msg}"
        );
    }

    #[test]
    fn both_is_refused_as_incoherent() {
        let e = coding_claim_from_parts(Some(coding()), true)
            .expect_err("a correction cannot both replace and strike");
        assert!(e.to_string().contains("both"), "{e}");
    }

    #[test]
    fn build_coding_sets_type_schema_twin() {
        let b = build_coding_body(
            Uuid::now_v7(),
            Uuid::now_v7(),
            Uuid::now_v7(),
            &CodeMedicationInput { coding: coding() },
            "kid",
            hlc(),
            None,
        );
        assert_eq!(b.event_type, "clinical.medication-coding.asserted");
        assert_eq!(b.schema_version, "clinical.medication-coding/1");
        assert_eq!(b.payload["coding"]["code"], MOIETY_ATORVASTATIN);
        assert_eq!(
            b.plaintext_twin.as_deref(),
            Some("coded as atorvastatin [drugref-moiety]")
        );
        assert_eq!(b.contributors[0]["role"], "recorded");
        assert!(b.t_effective.is_none());
    }

    #[test]
    fn build_correction_carries_the_target_and_the_strike() {
        let corrects = Uuid::now_v7();
        let b = build_coding_correction_body(
            Uuid::now_v7(),
            Uuid::now_v7(),
            Uuid::now_v7(),
            &CorrectCodingInput {
                corrects,
                claim: CodingClaim::Strike,
                note: Some("not atorvastatin; substance unidentified"),
            },
            "kid",
            hlc(),
        );
        assert_eq!(
            b.event_type,
            "clinical.medication-coding-correction.asserted"
        );
        assert_eq!(b.schema_version, "clinical.medication-coding-correction/1");
        assert_eq!(b.payload["corrects"], corrects.to_string());
        assert_eq!(b.payload["strike"], true);
        assert!(b
            .plaintext_twin
            .as_deref()
            .unwrap()
            .starts_with("coding struck"));
    }
}
