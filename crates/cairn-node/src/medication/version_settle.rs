//! Batch charting slice 2b: the node SIGNS the settling of a set-aside version (db/080).
//!
//! A reviewing clinician settles a version one of two ways:
//! - RESTORE it ("use this version instead"): `draft::chart_draft_line` with a [`RestoreToSign`]
//!   signs one cessation per line that currently stands for the companion (ruling R3), then the
//!   new line carrying `restores`, then the spends, all in ONE transaction. This file only
//!   prepares the stops (`prepare_stops`); the transaction is `chart_draft_line`'s.
//! - CHECK it ("checked: not current"): [`check_version`], one event in its own transaction.
//!
//! What stands for the companion is read by `version_settle_read::version_for_restore` through
//! db/080's lineage function, the same one the floor checks at COMMIT (ruling R4).
use super::cessation::{build_cease_body, CeaseMedicationInput};
use super::sealed_submit::{prepare_sealed, AuthorParams, Prepared};
use cairn_event::medication::{
    render_version_checked_twin, version_checked_body, SourceNote, VersionChecked,
    VERSION_CHECKED_EVENT_TYPE, VERSION_CHECKED_SCHEMA,
};
use cairn_event::{EventBody, SigningKey};
use uuid::Uuid;

/// A restore line's extra at Sign: the version it restores, and every line that stands for the
/// companion on the fresh read (`VersionForRestore::standing`), each with its cessation's
/// reason. Empty `stop`: the companion was stopped with no current replacement (ruling R3).
pub struct RestoreToSign<'a> {
    pub version_id: Uuid,
    pub stop: Vec<(Uuid, &'a str)>,
}

/// Seal and sign one cessation per `stop` entry (reason given, no stop date), authored by
/// `author`. Writes nothing but gaps in the HLC; the caller submits them inside its transaction,
/// BEFORE the restore line, so the floor's pairing check passes even under
/// `SET CONSTRAINTS IMMEDIATE`.
pub(crate) async fn prepare_stops(
    client: &tokio_postgres::Client,
    node_sk: &SigningKey,
    node_kid: &str,
    node_origin: &str,
    patient: Uuid,
    stop: &[(Uuid, &str)],
    author: &AuthorParams<'_>,
) -> anyhow::Result<Vec<Prepared>> {
    let mut out = Vec::with_capacity(stop.len());
    for (line, reason) in stop {
        let hlc = crate::db::next_hlc(client, node_origin).await?;
        let input = CeaseMedicationInput {
            stopped: None,
            stopped_precision: None,
            reason: Some(*reason),
        };
        let body = build_cease_body(Uuid::now_v7(), *line, patient, &input, node_kid, hlc);
        out.push(prepare_sealed(client, node_sk, body, Some(author)).await?);
    }
    Ok(out)
}

/// One "checked: not current" of the set-aside version `version_id`, with the source of the
/// check (a `From:` category word) and its optional detail and why.
pub struct VersionCheck<'a> {
    pub version_id: Uuid,
    pub info_source: &'a str,
    pub source_note: Option<SourceNote<'a>>,
}

/// Sign a check of a set-aside version of `patient`: one event, in its own transaction. The floor
/// (db/080 §2) refuses a version that is not on this chart; nothing is then written. A lost
/// commit is marked outcome-unknown (#814), as every clinical write here is.
pub async fn check_version(
    client: &mut tokio_postgres::Client,
    node_sk: &SigningKey,
    node_kid: &str,
    node_origin: &str,
    patient: Uuid,
    c: &VersionCheck<'_>,
    author: &AuthorParams<'_>,
) -> anyhow::Result<crate::db_diagnosis::Committed<Uuid>> {
    let hlc = crate::db::next_hlc(client, node_origin).await?;
    let vid = c.version_id.to_string();
    let checked = VersionChecked {
        version_id: &vid,
        info_source: c.info_source,
        source_note: c.source_note,
    };
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: VERSION_CHECKED_EVENT_TYPE.into(),
        schema_version: VERSION_CHECKED_SCHEMA.into(),
        hlc,
        t_effective: None,
        signer_key_id: node_kid.into(),
        contributors: serde_json::json!([{"actor_id": node_kid, "role": "recorded"}]),
        payload: version_checked_body(&checked),
        attachments: vec![],
        plaintext_twin: Some(render_version_checked_twin(&checked)),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    };
    let prepared = prepare_sealed(&*client, node_sk, body, Some(author)).await?;
    let tx = client.transaction().await?;
    super::draft_versions_sign::submit(&tx, &prepared).await?;
    tx.commit()
        .await
        .map_err(|e| crate::db_diagnosis::commit_failure(prepared.event_id, e.into()))?;
    Ok(crate::db_diagnosis::Committed {
        value: c.version_id,
        event_id: prepared.event_id,
    })
}
