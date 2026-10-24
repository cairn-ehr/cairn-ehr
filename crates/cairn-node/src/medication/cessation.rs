//! Medication cessation — marks an existing `medication_id` thread "past"
//! (§3.3). References the thread; offline-first (no local-presence check).
use cairn_event::medication::{
    medication_cessation_body, render_medication_cessation_twin, MedicationCessation,
};
use cairn_event::{EventBody, Hlc, SigningKey};
use cairn_medication_view::MedicationStatus;
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
/// nothing (PR #856 review). This thread's own cessation is therefore asked too. That the line
/// still reads current is `still_current_note`'s to say, after the stop.
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
    // The line reads current. Was this one thread stopped already? (A row in
    // medication_cessation is a stop; its reason, the newest stop's, may be NULL.) Every other
    // reason the thread's stops gave comes from db/060 in the same statement (#857), so a
    // reasonless later stop is never said as "no reason was recorded" (PR #860 review).
    let thread_stop = client
        .query_opt(
            "SELECT c.reason, \
                    ARRAY(SELECT r.reason FROM medication_cessation_reason r \
                          WHERE r.medication_id = c.medication_id \
                          ORDER BY r.hlc_wall DESC, r.hlc_counter DESC, \
                                   r.origin COLLATE \"C\" DESC, r.content_address DESC) \
                      AS all_reasons \
             FROM medication_cessation c WHERE c.medication_id = $1::text::uuid",
            &[&medication_id.to_string()],
        )
        .await?;
    Ok(thread_stop.map(|r| {
        let shown = r
            .get::<_, Option<String>>("reason")
            .filter(|s| !s.trim().is_empty());
        let others = crate::medication::read::other_reasons(shown.as_deref(), r.get("all_reasons"));
        // The line reads current, so the list shows none of these reasons: the sentence must
        // not promise it keeps them (PR #860 review; showing them is #861).
        cairn_medication_view::already_stopped_sentence(
            "This thread",
            shown.as_deref(),
            &others,
            cairn_medication_view::LIST_SHOWS_REASONS_ONLY_WHEN_STOPPED,
        )
    }))
}

/// What `medication-cease` adds when, AFTER its stop of `medication_id`, the line that thread is
/// on still reads current (#859). The verb stops ONE thread, and a reconciled line reads by its
/// latest-effective ACTIVE member (db/033 `medication_group_status`), so "ceased medication
/// thread X" would otherwise read as "the drug is stopped" while the list still shows it
/// current. Call it after `cease_medication`.
///
/// The line can only read current after this stop through a member thread with no stop of its
/// own in `medication_cessation` (db/033 counts only those threads' doses as active), so the
/// note names those threads, for the operator to stop too or to leave current on purpose. It
/// says "readable here", not "recorded": a stop sealed without custody on this node, or a
/// thread's only stop shredded, leaves no row though a stop was recorded (PR #860 review; a
/// shredded stop with an earlier one left is re-folded to it, #863).
///
/// Those threads are read from the whole group (`medication_thread_group`), as db/033 reads
/// it, NOT from `row.members`, which holds only this patient's charts: a peer's reconciliation
/// can put a thread from a chart not linked to this patient on the line, and that thread can
/// keep it current too. It is counted, never named: it is another chart's record (PR #860
/// review).
///
/// `None` when the line now reads stopped, and for a thread on no line here: the note read
/// before the stop already said that case cannot be told, or that it could not read the line
/// (`already_stopped_note`). A failed read is an `Err`; the caller turns it into
/// `not_known_whether_current`.
pub async fn still_current_note(
    client: &(impl tokio_postgres::GenericClient + Sync),
    patient: Uuid,
    medication_id: Uuid,
) -> anyhow::Result<Option<String>> {
    let list = crate::medication::read::list_patient_medications(client, patient).await?;
    let Some(row) = list.rows.iter().find(|r| {
        r.status == MedicationStatus::Active
            && r.members.iter().any(|m| m.medication_id == medication_id)
    }) else {
        return Ok(None);
    };
    let unstopped: Vec<Uuid> = client
        .query(
            "SELECT g.medication_id::text FROM medication_thread_group g \
             WHERE g.group_id = $1::text::uuid AND g.medication_id <> $2::text::uuid \
               AND NOT EXISTS (SELECT 1 FROM medication_cessation c \
                               WHERE c.medication_id = g.medication_id) \
             ORDER BY 1",
            &[&row.group_id.to_string(), &medication_id.to_string()],
        )
        .await?
        .iter()
        .map(|r| r.get::<_, String>(0).parse())
        .collect::<Result<_, _>>()?;
    let (here, unlinked): (Vec<Uuid>, Vec<Uuid>) = unstopped
        .into_iter()
        .partition(|t| row.members.iter().any(|m| m.medication_id == *t));
    let here: Vec<String> = here.iter().map(Uuid::to_string).collect();
    Ok(Some(still_current_sentence(&here, unlinked.len())))
}

/// `still_current_note`'s sentence: names this patient's threads that have no stop
/// (`unstopped`) and counts the ones on charts not linked to this patient (`unlinked`).
/// **Pure.** Both are empty only in a race (a sibling stopped between the two reads); the
/// sentence then says only what was read. With an unlinked thread it never says "stop that
/// one too if the drug is to stop": stopping this chart's threads may not stop the line.
pub fn still_current_sentence(unstopped: &[String], unlinked: usize) -> String {
    let head = "This drug's line still reads current";
    let tail = "This command stops one thread";
    let mut said = Vec::new();
    match unstopped {
        [] => {}
        [one] => said.push(format!("no stop of thread {one} is readable here")),
        many => said.push(format!(
            "no stop of threads {} is readable here",
            many.join(", ")
        )),
    }
    if unlinked > 0 {
        let threads = if unlinked == 1 { "thread" } else { "threads" };
        said.push(format!(
            "no stop of {unlinked} {threads} on a chart not linked to this patient is readable \
             here"
        ));
    }
    let advice = match (unstopped.len(), unlinked) {
        (0, 0) => return format!("{head}. {tail}."),
        (_, 1..) => "stopping this patient's threads alone may not stop the line",
        (1, 0) => "stop that one too if the drug is to stop",
        _ => "stop those too if the drug is to stop",
    };
    format!("{head}: {}. {tail}; {advice}.", said.join("; "))
}

/// The note `medication-cease` prints when its read of the line AFTER the stop failed: the stop
/// is already written, and the operator is told what is therefore not known (#859).
pub fn not_known_whether_current(e: &anyhow::Error) -> String {
    format!(
        "Could not read this drug's line after stopping this thread ({e:#}), so whether the \
         line still reads current is not known."
    )
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

#[cfg(test)]
mod tests {
    use super::*;

    /// #859: the sentence names every unstopped thread, and fits one, many, or (a race) none.
    #[test]
    fn still_current_sentence_names_the_threads_with_no_stop() {
        let one = still_current_sentence(&["y".into()], 0);
        assert_eq!(
            one,
            "This drug's line still reads current: no stop of thread y is readable here. This \
             command stops one thread; stop that one too if the drug is to stop."
        );
        let many = still_current_sentence(&["y".into(), "z".into()], 0);
        assert!(many.contains("of threads y, z is readable here."), "{many}");
        assert!(many.contains("stop those too"), "{many}");
        assert_eq!(
            still_current_sentence(&[], 0),
            "This drug's line still reads current. This command stops one thread."
        );
    }

    /// PR #860 review: a thread on a chart not linked to this patient is counted, never named,
    /// and the sentence no longer promises that stopping this chart's threads stops the drug.
    #[test]
    fn still_current_sentence_counts_threads_on_unlinked_charts() {
        assert_eq!(
            still_current_sentence(&[], 1),
            "This drug's line still reads current: no stop of 1 thread on a chart not linked \
             to this patient is readable here. This command stops one thread; stopping this \
             patient's threads alone may not stop the line."
        );
        let both = still_current_sentence(&["y".into()], 2);
        assert!(
            both.contains("thread y is readable here; no stop of 2 threads on a chart"),
            "{both}"
        );
        assert!(!both.contains("stop that one too"), "{both}");
    }
}
