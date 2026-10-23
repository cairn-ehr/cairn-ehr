//! Medication cessation — marks an existing `medication_id` thread "past"
//! (§3.3). References the thread; offline-first (no local-presence check).
use cairn_event::medication::{
    medication_cessation_body, render_medication_cessation_twin, MedicationCessation,
};
use cairn_event::{EventBody, Hlc, SigningKey};
use uuid::Uuid;

const MEDICATION_CESSATION_SCHEMA_VERSION: &str = "clinical.medication-cessation/1";

/// The clinician-supplied fields of a cessation. All optional.
pub struct CeaseMedicationInput<'a> {
    pub stopped: Option<&'a str>,
    pub stopped_precision: Option<&'a str>,
    pub reason: Option<&'a str>,
}

/// Assemble the signed `clinical.medication-cessation.asserted` EventBody.
pub fn build_cease_body(
    event_id: Uuid,
    medication_id: Uuid,
    patient: Uuid,
    input: &CeaseMedicationInput<'_>,
    node_kid: &str,
    hlc: Hlc,
) -> EventBody {
    let mid = medication_id.to_string();
    let csn = MedicationCessation {
        medication_id: &mid,
        stopped: input.stopped,
        stopped_precision: input.stopped_precision,
        reason: input.reason,
    };
    EventBody {
        event_id: event_id.to_string(),
        patient_id: patient.to_string(),
        event_type: "clinical.medication-cessation.asserted".into(),
        schema_version: MEDICATION_CESSATION_SCHEMA_VERSION.into(),
        hlc,
        t_effective: None,
        signer_key_id: node_kid.into(),
        contributors: serde_json::json!([{"actor_id": node_kid, "role": "recorded"}]),
        payload: medication_cessation_body(&csn),
        attachments: vec![],
        plaintext_twin: Some(render_medication_cessation_twin(&csn)),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    }
}

/// Cease a medication thread — makes it "past". Offline-first: does NOT check the
/// assert is present locally. Returns the cessation event id. Device-additive when
/// `attest` is `None` (unchanged 1-arg submit door). When `attest` is `Some`, the
/// cessation AND the human's responsibility attestation for the thread run in ONE
/// transaction (same atomic shape as `assert_medication`); a rejected attestation
/// rolls the cessation back with it. `author` is ADR-0053's separable human-authorship
/// overlay (`None` ⇒ device-additive, the node signs and is the sole `recorded`
/// contributor); it is independent of `attest` (who vouches) — see `AuthorParams`.
#[allow(clippy::too_many_arguments)] // signer + node context + patient/thread/input/author/attest, mirrors dose orchestrators
pub async fn cease_medication(
    client: &mut tokio_postgres::Client,
    node_sk: &SigningKey,
    node_kid: &str,
    node_origin: &str,
    patient: Uuid,
    medication_id: Uuid,
    input: &CeaseMedicationInput<'_>,
    author: Option<&crate::medication::AuthorParams<'_>>,
    attest: Option<&crate::medication::AttestParams<'_>>,
) -> anyhow::Result<Uuid> {
    let verb_hlc = crate::db::next_hlc(client, node_origin).await?;
    let event_id = Uuid::now_v7();
    let body = build_cease_body(event_id, medication_id, patient, input, node_kid, verb_hlc);
    // ADR-0052 seal-at-write: seal + sign + submit through the ONE strict door, with the
    // atomic author-time attestation folded in when `attest` is Some (see sealed_submit).
    crate::medication::sealed_submit::seal_sign_submit(client, node_sk, body, author, attest)
        .await?;
    Ok(event_id)
}

/// What `medication-cease` adds after its success line when `medication_id`'s line was
/// ALREADY stopped as it began (#854, ADR-0082 decision 4): a colleague's stop landed first.
/// The cease is still written, so this only keeps "ceased" from being read as "this ceased
/// it". Call it BEFORE `cease_medication`: afterwards, the caller's own stop would make every
/// line read as already stopped.
///
/// Reads the same list the window reads (`list_patient_medications`) and decides first by the
/// same LINE rule (`MedicationRow::already_stopped_note`). But the window stops every thread
/// of a line and this verb stops ONE: on a reconciled line, a thread already stopped beside a
/// later active sibling leaves the line reading current, so the line rule alone would say
/// nothing (PR #856 review). This thread's own cessation is therefore asked too, and that
/// note also says the line still reads current — after this stop as before it.
///
/// `None` only when neither the line nor the thread was stopped. A thread on no line of this
/// patient's list here (offline-first: the assert need not be local) gets a sentence saying
/// so, because silence would read as "not already stopped" (principle 4). A failed read is an
/// `Err`, never `Ok(None)`, for the same reason.
pub async fn already_stopped_note(
    client: &(impl tokio_postgres::GenericClient + Sync),
    patient: Uuid,
    medication_id: Uuid,
) -> anyhow::Result<Option<String>> {
    let list = crate::medication::read::list_patient_medications(client, patient).await?;
    let Some(row) = list
        .rows
        .iter()
        .find(|r| r.members.iter().any(|m| m.medication_id == medication_id))
    else {
        return Ok(Some(
            "No line for this thread is on this patient's list here, so whether it was \
             already stopped is not known."
                .to_string(),
        ));
    };
    if let Some(note) = row.already_stopped_note() {
        return Ok(Some(note));
    }
    // The line reads current. Was this one thread stopped already? (A row here is a stop;
    // its reason may be NULL.)
    let thread_stop = client
        .query_opt(
            "SELECT reason FROM medication_cessation WHERE medication_id = $1::text::uuid",
            &[&medication_id.to_string()],
        )
        .await?;
    Ok(thread_stop.map(|r| {
        format!(
            "{} The line it is on still reads current: another record of this drug on it is \
             active.",
            cairn_medication_view::already_stopped_sentence(
                "This thread",
                r.get::<_, Option<&str>>(0)
            )
        )
    }))
}

/// The note `medication-cease` prints when its read of the line FAILED: the stop is a safety
/// write, so a failed display-only read never blocks it, and the operator is told what is
/// therefore not known (#854 review).
pub fn not_known_whether_stopped(e: &anyhow::Error) -> String {
    format!(
        "Could not read this drug's line before stopping it ({e:#}), so whether it was \
         already stopped is not known."
    )
}
