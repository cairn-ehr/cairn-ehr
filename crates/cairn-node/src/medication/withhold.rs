//! Medication withhold and release (ADR-0081) — the paper chart's "W". A withhold puts a
//! time-bounded hold on an existing `medication_id` thread ("do not give this until X, or
//! until reviewed"); a release ends named holds early. Both reference the thread and are
//! offline-first like cessation: no local-presence check here. The in-DB floor (db/059)
//! is the real enforcement — it refuses a malformed span at both doors and, at the local
//! door only, a withhold on a ceased line or a release of something that is not a hold on
//! this thread, or of a hold that has already ended (#843, ADR-0082 — refused here first, as a
//! stale picture, so the caller re-reads).
use cairn_event::medication::{
    medication_withhold_body, medication_withhold_release_body,
    render_medication_withhold_release_twin, render_medication_withhold_twin, HoldUntil,
    MedicationWithhold, MedicationWithholdRelease,
};
use cairn_event::{EventBody, Hlc, SigningKey};
use uuid::Uuid;

const MEDICATION_WITHHOLD_SCHEMA_VERSION: &str = "clinical.medication-withhold/1";
const MEDICATION_WITHHOLD_RELEASE_SCHEMA_VERSION: &str = "clinical.medication-withhold-release/1";

/// The clinician-supplied fields of a withhold. `from` absent means "from now"; `until` is
/// required — an instant, or `HoldUntil::Reviewed` for the explicit open end.
pub struct WithholdMedicationInput<'a> {
    pub reason: &'a str,
    pub from: Option<&'a str>,
    pub until: HoldUntil<'a>,
}

/// Wrap a payload + twin in the device-shaped EventBody every medication verb uses (the
/// node signs, sole `recorded` contributor; ADR-0053 authorship is layered later by
/// `seal_sign_submit`). `t_effective` is left absent here; the orchestrators stamp it with
/// the physical clock where a hold starts "now" (see [`physical_now`]). An advance hold's
/// start rides in the payload's `from` instead: a future `t_effective` would be recorded as
/// a clock-ceiling anomaly (db/005 step 1b').
#[allow(clippy::too_many_arguments)] // the envelope's fields, one per argument
fn device_body(
    event_id: Uuid,
    patient: Uuid,
    event_type: &str,
    schema_version: &str,
    payload: serde_json::Value,
    twin: String,
    node_kid: &str,
    hlc: Hlc,
) -> EventBody {
    EventBody {
        event_id: event_id.to_string(),
        patient_id: patient.to_string(),
        event_type: event_type.into(),
        schema_version: schema_version.into(),
        hlc,
        t_effective: None,
        signer_key_id: node_kid.into(),
        contributors: serde_json::json!([{"actor_id": node_kid, "role": "recorded"}]),
        payload,
        attachments: vec![],
        plaintext_twin: Some(twin),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    }
}

/// Assemble the signed-to-be `clinical.medication-withhold.asserted` EventBody.
pub fn build_withhold_body(
    event_id: Uuid,
    medication_id: Uuid,
    patient: Uuid,
    input: &WithholdMedicationInput<'_>,
    node_kid: &str,
    hlc: Hlc,
) -> EventBody {
    let mid = medication_id.to_string();
    let w = MedicationWithhold {
        medication_id: &mid,
        reason: input.reason,
        from: input.from,
        until: input.until,
    };
    device_body(
        event_id,
        patient,
        "clinical.medication-withhold.asserted",
        MEDICATION_WITHHOLD_SCHEMA_VERSION,
        medication_withhold_body(&w),
        render_medication_withhold_twin(&w),
        node_kid,
        hlc,
    )
}

/// The holds a release names, each once, in first-seen order. `medication-release --withhold
/// X --withhold X` names ONE hold: the payload, the twin and the CLI's success line all count
/// through this, so none of them can say "2 holds".
pub fn distinct_holds(withholds: &[Uuid]) -> Vec<Uuid> {
    let mut out: Vec<Uuid> = Vec::with_capacity(withholds.len());
    for id in withholds {
        if !out.contains(id) {
            out.push(*id);
        }
    }
    out
}

/// Assemble the `clinical.medication-withhold-release.asserted` EventBody. `withholds` are
/// the event ids of the holds it ends — only holds its author saw (ADR-0081 decision 4),
/// each recorded once ([`distinct_holds`]).
pub fn build_release_body(
    event_id: Uuid,
    medication_id: Uuid,
    patient: Uuid,
    withholds: &[Uuid],
    reason: Option<&str>,
    node_kid: &str,
    hlc: Hlc,
) -> EventBody {
    let mid = medication_id.to_string();
    let releases: Vec<String> = distinct_holds(withholds)
        .iter()
        .map(Uuid::to_string)
        .collect();
    let r = MedicationWithholdRelease {
        medication_id: &mid,
        releases: &releases,
        reason,
    };
    device_body(
        event_id,
        patient,
        "clinical.medication-withhold-release.asserted",
        MEDICATION_WITHHOLD_RELEASE_SCHEMA_VERSION,
        medication_withhold_release_body(&r),
        render_medication_withhold_release_twin(&r),
        node_kid,
        hlc,
    )
}

/// The node's PHYSICAL clock now, as an RFC 3339 UTC instant (millisecond precision) — the
/// `t_effective` of a withhold without `from` and of every release (ADR-0081).
///
/// Why not the HLC wall: syncing with a peer whose clock runs fast pushes this node's HLC up
/// to 24 h ahead (db/020), and `node_hlc_tick` keeps issuing that wall. A "W until reviewed"
/// stamped with it would START hours in the future — the fold calls it pending and the drug
/// reads as givable meanwhile. Postgres's `clock_timestamp()` is the same host clock the
/// HLC tick reads, so this needs no new dependency.
///
/// Call it BEFORE `next_hlc`: the tick then reads the clock no earlier than this did, and
/// rounds rather than truncates, so the instant is never after the event's HLC wall and the
/// bitemporal ceiling never flags it.
async fn physical_now(client: &tokio_postgres::Client) -> anyhow::Result<String> {
    let row = client
        .query_one(
            "SELECT to_char(date_trunc('milliseconds', clock_timestamp()) AT TIME ZONE 'UTC', \
                            'YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"')",
            &[],
        )
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "reading the node's physical clock: {}",
                crate::db_diagnosis::legible_db_error(&e)
            )
        })?;
    Ok(row.get(0))
}

/// The named holds that are no longer standing at instant `at`, each as "hold <id> <why>" —
/// db/059's `cairn_medication_hold_ended`, the floor's own predicate, so the two never
/// disagree. A hold that is not a withhold here at all is not in this list: the floor
/// refuses that one with its own sentence.
async fn ended_holds(
    client: &tokio_postgres::Client,
    withholds: &[Uuid],
    at: &str,
) -> anyhow::Result<Vec<String>> {
    let ids: Vec<String> = distinct_holds(withholds)
        .iter()
        .map(Uuid::to_string)
        .collect();
    let rows = client
        .query(
            "SELECT 'hold ' || w::text || ' ' || why FROM unnest($1::text[]::uuid[]) AS w, \
                    cairn_medication_hold_ended(w, $2::text::timestamptz) AS why \
              WHERE why IS NOT NULL",
            &[&ids, &at],
        )
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "checking that the holds to release are still standing: {}",
                crate::db_diagnosis::legible_db_error(&e)
            )
        })?;
    Ok(rows.iter().map(|r| r.get(0)).collect())
}

/// Withhold a medication thread — write a "W". Returns the withhold's event id, which is
/// what a later release names. `author` is ADR-0053's human-authorship overlay (`None` ⇒
/// the node signs alone). Sealed, signed and submitted through the one strict door.
#[allow(clippy::too_many_arguments)] // signer + node context + patient/thread/input/author, mirrors cease_medication
pub async fn withhold_medication(
    client: &mut tokio_postgres::Client,
    node_sk: &SigningKey,
    node_kid: &str,
    node_origin: &str,
    patient: Uuid,
    medication_id: Uuid,
    input: &WithholdMedicationInput<'_>,
    author: Option<&crate::medication::AuthorParams<'_>>,
) -> anyhow::Result<Uuid> {
    // "From now" means the physical now, carried as t_effective (see `physical_now`).
    let started = match input.from {
        None => Some(physical_now(client).await?),
        Some(_) => None,
    };
    let hlc = crate::db::next_hlc(client, node_origin).await?;
    let event_id = Uuid::now_v7();
    let mut body = build_withhold_body(event_id, medication_id, patient, input, node_kid, hlc);
    body.t_effective = started;
    crate::medication::sealed_submit::seal_sign_submit(client, node_sk, body, author, None).await?;
    Ok(event_id)
}

/// End the named holds on a thread early. Returns the release's event id.
#[allow(clippy::too_many_arguments)] // signer + node context + patient/thread/holds/reason/author
pub async fn release_medication_holds(
    client: &mut tokio_postgres::Client,
    node_sk: &SigningKey,
    node_kid: &str,
    node_origin: &str,
    patient: Uuid,
    medication_id: Uuid,
    withholds: &[Uuid],
    reason: Option<&str>,
    author: Option<&crate::medication::AuthorParams<'_>>,
) -> anyhow::Result<Uuid> {
    // A release ends its holds at the physical now, carried as t_effective (`physical_now`).
    let released = physical_now(client).await?;
    // #843: a hold that ended before `released` (a colleague released it, or it ran out) means
    // the caller's picture is behind the record — refused as stale, before anything is
    // signed, so a window re-reads. The floor refuses the same release (db/059 section 8);
    // this check exists so the refusal carries its scope, which a floor error cannot.
    let ended = ended_holds(client, withholds, &released).await?;
    if !ended.is_empty() {
        return Err(crate::db_diagnosis::stale_picture_refusal(format!(
            "{}; nothing was released",
            ended.join("; ")
        )));
    }
    let hlc = crate::db::next_hlc(client, node_origin).await?;
    let event_id = Uuid::now_v7();
    let mut body = build_release_body(
        event_id,
        medication_id,
        patient,
        withholds,
        reason,
        node_kid,
        hlc,
    );
    body.t_effective = Some(released);
    crate::medication::sealed_submit::seal_sign_submit(client, node_sk, body, author, None).await?;
    Ok(event_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hlc() -> Hlc {
        Hlc {
            wall: 1,
            counter: 0,
            node_origin: "test-node".into(),
        }
    }

    #[test]
    fn a_release_naming_a_hold_twice_names_it_once() {
        // `medication-release --withhold X --withhold X` must not record — or say — "2 holds".
        let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
        let body = build_release_body(
            Uuid::now_v7(),
            Uuid::now_v7(),
            Uuid::now_v7(),
            &[a, b, a, a],
            None,
            "kid",
            hlc(),
        );
        assert_eq!(
            body.payload["releases"],
            serde_json::json!([a.to_string(), b.to_string()]),
            "duplicates dropped, first-seen order kept"
        );
        assert_eq!(
            body.plaintext_twin.as_deref(),
            Some("Released withheld medication (2 holds)")
        );
    }
}
