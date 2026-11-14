//! Charting slice 2: the node SIGNS the giving of a stat dose, and the striking of a given (db/082).
//!
//! On paper a stat order is a line in the "stat" box; the giver initials it and the order is done.
//! Here that is TWO events signed together (the architecture's floor, not the clinician's burden):
//! the order's cessation (reason `STAT_GIVEN_REASON`, no stop date: it leaves the current list) and
//! the given, which `completes` that cessation. Striking a given ("entered in error") is one event
//! that voids the stop, so the stat order is waiting to be given again.
use super::cessation::{build_cease_body, CeaseMedicationInput};
use super::draft_versions_sign::submit;
use super::sealed_submit::{prepare_sealed, AuthorParams};
use cairn_event::medication::{
    given_body, given_retraction_body, render_given_retraction_twin, render_given_twin, Given,
    GivenRetraction, GIVEN_EVENT_TYPE, GIVEN_RETRACTION_EVENT_TYPE, GIVEN_RETRACTION_SCHEMA,
    GIVEN_SCHEMA, STAT_GIVEN_REASON,
};
use cairn_event::{ClockGrade, EventBody, SigningKey};
use chrono::{DateTime, SecondsFormat, Utc};
use uuid::Uuid;

/// A medication event envelope as every human-signed act here builds it: the node's `recorded`
/// contributor (`prepare_sealed` rewrites it to authored + recorded, ADR-0053).
#[allow(clippy::too_many_arguments)] // the fixed envelope fields, named at the one call site each
fn envelope(
    patient: Uuid,
    event_type: &str,
    schema: &str,
    hlc: cairn_event::Hlc,
    t_effective: Option<String>,
    node_kid: &str,
    payload: serde_json::Value,
    twin: String,
) -> EventBody {
    EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: event_type.into(),
        schema_version: schema.into(),
        hlc,
        t_effective,
        signer_key_id: node_kid.into(),
        contributors: serde_json::json!([{"actor_id": node_kid, "role": "recorded"}]),
        payload,
        attachments: vec![],
        plaintext_twin: Some(twin),
        clock_grade: ClockGrade::SelfAsserted,
        safety: None,
    }
}

/// Give a stat order: ONE transaction, the cessation (STAT_GIVEN_REASON, no stop date) FIRST, then
/// the given (`completes` = the cessation's event id, `t_effective` = `given_at`). Both are signed
/// by `author`: prepare both, then submit both, as `chart_draft_line` does. Returns the given's id.
///
/// NOTHING may wrap the stop's submit in a SAVEPOINT / subtransaction: db/082 requires the stop
/// to carry this transaction's own top-level id (`xmin = pg_current_xact_id()`), and a
/// subtransaction would stamp its own, so the given would be refused (it fails closed).
#[allow(clippy::too_many_arguments)] // signer + node context + the act's own facts
pub async fn give_stat(
    client: &mut tokio_postgres::Client,
    node_sk: &SigningKey,
    node_kid: &str,
    node_origin: &str,
    patient: Uuid,
    medication_id: Uuid,
    given_at: DateTime<Utc>,
    author: &AuthorParams<'_>,
) -> anyhow::Result<crate::db_diagnosis::Committed<Uuid>> {
    let stop_hlc = crate::db::next_hlc(client, node_origin).await?;
    let stop_body = build_cease_body(
        Uuid::now_v7(),
        medication_id,
        patient,
        &CeaseMedicationInput {
            stopped: None,
            stopped_precision: None,
            reason: Some(STAT_GIVEN_REASON),
        },
        node_kid,
        stop_hlc,
    );
    let stop = prepare_sealed(&*client, node_sk, stop_body, Some(author)).await?;

    let (mid, completes) = (medication_id.to_string(), stop.event_id.to_string());
    let g = Given {
        medication_id: &mid,
        completes: Some(&completes),
    };
    let given_body_ = envelope(
        patient,
        GIVEN_EVENT_TYPE,
        GIVEN_SCHEMA,
        crate::db::next_hlc(client, node_origin).await?,
        Some(given_at.to_rfc3339_opts(SecondsFormat::Millis, true)),
        node_kid,
        given_body(&g),
        render_given_twin(&g),
    );
    let given = prepare_sealed(&*client, node_sk, given_body_, Some(author)).await?;

    let tx = client.transaction().await?;
    submit(&tx, &stop).await?;
    submit(&tx, &given).await?;
    tx.commit()
        .await
        .map_err(|e| crate::db_diagnosis::commit_failure(given.event_id, e.into()))?;
    Ok(crate::db_diagnosis::Committed {
        value: given.event_id,
        event_id: given.event_id,
    })
}

/// Strike a given ("entered in error"): reads the given's `completes` from `medication_given` ON
/// THIS CHART and signs `voids` = it, in its own transaction. Errors, writing nothing, when the
/// given is not on this chart. Blank `text` is written as none (ADR-0097 decision 3).
#[allow(clippy::too_many_arguments)] // signer + node context + the act's own facts
pub async fn strike_given(
    client: &mut tokio_postgres::Client,
    node_sk: &SigningKey,
    node_kid: &str,
    node_origin: &str,
    patient: Uuid,
    given_event: Uuid,
    text: Option<&str>,
    author: &AuthorParams<'_>,
) -> anyhow::Result<crate::db_diagnosis::Committed<Uuid>> {
    let row = client
        .query_opt(
            "SELECT medication_id::text, completes::text FROM medication_given \
              WHERE event_id = $1::text::uuid AND patient_id = $2::text::uuid",
            &[&given_event.to_string(), &patient.to_string()],
        )
        .await?;
    let Some(row) = row else {
        anyhow::bail!("that given dose is not on this chart; nothing was written");
    };
    let (mid, voids): (String, Option<String>) = (row.try_get(0)?, row.try_get(1)?);
    let target = given_event.to_string();
    let r = GivenRetraction {
        medication_id: &mid,
        target: &target,
        voids: voids.as_deref(),
        text,
    };
    let body = envelope(
        patient,
        GIVEN_RETRACTION_EVENT_TYPE,
        GIVEN_RETRACTION_SCHEMA,
        crate::db::next_hlc(client, node_origin).await?,
        None,
        node_kid,
        given_retraction_body(&r),
        render_given_retraction_twin(&r),
    );
    let prepared = prepare_sealed(&*client, node_sk, body, Some(author)).await?;
    let tx = client.transaction().await?;
    submit(&tx, &prepared).await?;
    tx.commit()
        .await
        .map_err(|e| crate::db_diagnosis::commit_failure(prepared.event_id, e.into()))?;
    Ok(crate::db_diagnosis::Committed {
        value: prepared.event_id,
        event_id: prepared.event_id,
    })
}
