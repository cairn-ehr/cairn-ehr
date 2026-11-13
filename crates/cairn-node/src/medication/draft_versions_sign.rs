//! Batch charting slice 2a: the node signs a "set aside" version (a drug version the clinician
//! did not believe) beside its companion line.
//!
//! Two shapes, one rule (a line's event and its spend are ONE transaction, durable-rules
//! BATCH CHARTING SLICE 1):
//! - the companion is a DRAFT line: `draft::chart_draft_line` signs the companion and every
//!   version of it in its own single transaction, using `prepare_version` and `spend_row`;
//! - the companion is already CHARTED: [`set_aside_draft_line`] signs the version alone.
//!
//! A set-aside carries NO safety claim (the floor refuses one at both doors), so the §5.9 class
//! lookup is never called here.
use super::sealed_submit::{prepare_sealed, AuthorParams, Prepared};
use anyhow::Context;
use cairn_event::medication::{
    render_set_aside_twin, set_aside_body, MedicationAssertion, MedicationSetAside,
    SET_ASIDE_EVENT_TYPE, SET_ASIDE_SCHEMA,
};
use cairn_event::{EventBody, SigningKey};
use tokio_postgres::Transaction;
use uuid::Uuid;

/// One version to sign beside its companion: which draft row it is (and the `saved_at` the
/// caller read it at, so the spend takes only the content that was read) and the drug it names.
/// `drug.medication_id` is ignored (a version is not a thread).
pub struct VersionToSign<'a> {
    pub line: Uuid,
    pub saved_at: &'a str,
    pub drug: MedicationAssertion<'a>,
}

/// Seal and sign one version of the thread `alternative_to`. Writes nothing but a gap in the HLC.
/// Returns what the strict door needs plus the freshly minted `version_id`.
#[allow(clippy::too_many_arguments)] // signer + node context + the version's identity
pub(crate) async fn prepare_version(
    client: &tokio_postgres::Client,
    node_sk: &SigningKey,
    node_kid: &str,
    node_origin: &str,
    patient: Uuid,
    v: &VersionToSign<'_>,
    alternative_to: Uuid,
    author: &AuthorParams<'_>,
) -> anyhow::Result<(Prepared, Uuid)> {
    let hlc = crate::db::next_hlc(client, node_origin).await?;
    let version_id = Uuid::now_v7();
    let (vid, alt) = (version_id.to_string(), alternative_to.to_string());
    let s = MedicationSetAside {
        version_id: &vid,
        alternative_to: &alt,
        drug: &v.drug,
    };
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: SET_ASIDE_EVENT_TYPE.into(),
        schema_version: SET_ASIDE_SCHEMA.into(),
        hlc,
        t_effective: None,
        signer_key_id: node_kid.into(),
        contributors: serde_json::json!([{"actor_id": node_kid, "role": "recorded"}]),
        payload: set_aside_body(&s),
        attachments: vec![],
        plaintext_twin: Some(render_set_aside_twin(&s)),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    };
    let prepared = prepare_sealed(client, node_sk, body, Some(author)).await?;
    Ok((prepared, version_id))
}

/// Submit a prepared event through the strict door, inside `tx`.
pub(crate) async fn submit(tx: &Transaction<'_>, p: &Prepared) -> anyhow::Result<()> {
    tx.execute(
        "SELECT submit_event($1, NULL, NULL, $2)",
        &[&p.signed_bytes, &p.dek.as_bytes().as_slice()],
    )
    .await
    .context("submitting the event (nothing was written)")?;
    Ok(())
}

/// Spend a draft row through the delete door, naming the `saved_at` it was read at. Anything but
/// `deleted` is a [`SpendRefused`](super::draft::SpendRefused): the caller returns, dropping `tx`
/// (which rolls back), so nothing is written.
pub(crate) async fn spend_row(
    tx: &Transaction<'_>,
    line: Uuid,
    patient: Uuid,
    terminal: &str,
    writer: Option<&str>,
    saved_at: &str,
) -> anyhow::Result<()> {
    let answer: String = tx
        .query_one(
            "SELECT cairn_med_draft_line_delete($1::text::uuid, $2::text::uuid, $3, $4, $5::text::timestamptz)",
            &[&line.to_string(), &patient.to_string(), &terminal, &writer, &saved_at],
        )
        .await
        .context("spending the draft line (nothing was written)")?
        .get(0);
    if answer != "deleted" {
        return Err(super::draft::SpendRefused(answer).into());
    }
    Ok(())
}

/// Sign a version whose companion is ALREADY charted (`alternative_to` is its `medication_id`):
/// the version's event and the spend of its draft row, in one transaction. The floor refuses a
/// companion that is not on this chart or has been stopped since; the transaction then rolls
/// back and the version stays in the list.
#[allow(clippy::too_many_arguments)] // signer + node context + the version's identity
pub async fn set_aside_draft_line(
    client: &mut tokio_postgres::Client,
    node_sk: &SigningKey,
    node_kid: &str,
    node_origin: &str,
    patient: Uuid,
    terminal: &str,
    writer: Option<&str>,
    v: &VersionToSign<'_>,
    alternative_to: Uuid,
    author: &AuthorParams<'_>,
) -> anyhow::Result<crate::db_diagnosis::Committed<Uuid>> {
    let (prepared, version_id) = prepare_version(
        client,
        node_sk,
        node_kid,
        node_origin,
        patient,
        v,
        alternative_to,
        author,
    )
    .await?;
    let tx = client.transaction().await?;
    submit(&tx, &prepared).await?;
    spend_row(&tx, v.line, patient, terminal, writer, v.saved_at).await?;
    tx.commit()
        .await
        .map_err(|e| crate::db_diagnosis::commit_failure(prepared.event_id, e.into()))?;
    Ok(crate::db_diagnosis::Committed {
        value: version_id,
        event_id: prepared.event_id,
    })
}
