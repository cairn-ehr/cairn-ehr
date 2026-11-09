//! Signing an allergy entry, a "none known" statement, or one of the two overlays — a de-label by
//! challenge and an entered-in-error withdrawal (ADR-0053: per-write human authorship). A
//! correction is an entry whose `corrects` names the line it replaces, so it signs as an entry.
//!
//! All build the body the way `note::sign::note_body` does: no `t_effective` (an allergy's time is
//! its onset, inside the payload) and no safety signal (design section 4), then hand it to
//! `seal_sign_submit`, which rewrites the contributors to `[human authored, node recorded]`, signs
//! and submits through the strict door. This file commits nothing itself, so it is not a
//! `MARKED_COMMIT_SITES` entry; the commit is `sealed_submit.rs`'s.
use cairn_event::allergy::{
    allergy_none_payload, allergy_payload, render_allergy_none_twin, render_allergy_twin,
    AllergyEntry, NoneScope, ALLERGY_EVENT_TYPE, ALLERGY_NONE_EVENT_TYPE,
    ALLERGY_NONE_SCHEMA_VERSION, ALLERGY_SCHEMA_VERSION,
};
use cairn_event::allergy_overlay::{
    allergy_challenge_payload, allergy_retraction_payload, render_allergy_challenge_twin,
    render_allergy_retraction_twin, AllergyChallenge, AllergyRetraction,
    ALLERGY_CHALLENGE_EVENT_TYPE, ALLERGY_CHALLENGE_SCHEMA_VERSION, ALLERGY_RETRACTION_EVENT_TYPE,
    ALLERGY_RETRACTION_SCHEMA_VERSION,
};
use cairn_event::{EventBody, Hlc, SigningKey};
use uuid::Uuid;

use crate::medication::sealed_submit::seal_sign_submit;
use crate::medication::AuthorParams;

/// A device-shaped body (the node as sole `recorded` contributor); `seal_sign_submit` adds the human.
fn body(
    patient: Uuid,
    node_kid: &str,
    hlc: Hlc,
    (event_type, schema_version): (&str, &str),
    payload: serde_json::Value,
    twin: String,
) -> EventBody {
    EventBody {
        event_id: Uuid::now_v7().to_string(),
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

/// Sign one allergy entry on `patient`'s chart under `author`; returns the new event's id.
pub async fn sign_allergy(
    client: &mut tokio_postgres::Client,
    node_sk: &SigningKey,
    node_kid: &str,
    node_origin: &str,
    patient: Uuid,
    entry: &AllergyEntry<'_>,
    author: &AuthorParams<'_>,
) -> anyhow::Result<Uuid> {
    let hlc = crate::db::next_hlc(client, node_origin).await?;
    let b = body(
        patient,
        node_kid,
        hlc,
        (ALLERGY_EVENT_TYPE, ALLERGY_SCHEMA_VERSION),
        allergy_payload(entry),
        render_allergy_twin(entry),
    );
    seal_sign_submit(client, node_sk, b, Some(author), None).await
}

/// Sign a "none known" statement of `scope` on `patient`'s chart; its "asked" time is its HLC wall.
pub async fn sign_allergy_none(
    client: &mut tokio_postgres::Client,
    node_sk: &SigningKey,
    node_kid: &str,
    node_origin: &str,
    patient: Uuid,
    scope: NoneScope,
    author: &AuthorParams<'_>,
) -> anyhow::Result<Uuid> {
    let hlc = crate::db::next_hlc(client, node_origin).await?;
    let b = body(
        patient,
        node_kid,
        hlc,
        (ALLERGY_NONE_EVENT_TYPE, ALLERGY_NONE_SCHEMA_VERSION),
        allergy_none_payload(scope),
        render_allergy_none_twin(scope),
    );
    seal_sign_submit(client, node_sk, b, Some(author), None).await
}

/// Sign a de-label by challenge on `patient`'s chart: "tolerated this substance on this date". The
/// floor (db/075) refuses a target that is not an allergy entry on the same chart.
pub async fn sign_allergy_challenge(
    client: &mut tokio_postgres::Client,
    node_sk: &SigningKey,
    node_kid: &str,
    node_origin: &str,
    patient: Uuid,
    challenge: &AllergyChallenge<'_>,
    author: &AuthorParams<'_>,
) -> anyhow::Result<Uuid> {
    let hlc = crate::db::next_hlc(client, node_origin).await?;
    let b = body(
        patient,
        node_kid,
        hlc,
        (
            ALLERGY_CHALLENGE_EVENT_TYPE,
            ALLERGY_CHALLENGE_SCHEMA_VERSION,
        ),
        allergy_challenge_payload(challenge),
        render_allergy_challenge_twin(challenge),
    );
    seal_sign_submit(client, node_sk, b, Some(author), None).await
}

/// Sign an entered-in-error withdrawal on `patient`'s chart. Its target may be an entry, a "none
/// known" statement or a de-label on the same chart, never another withdrawal (db/075).
pub async fn sign_allergy_retraction(
    client: &mut tokio_postgres::Client,
    node_sk: &SigningKey,
    node_kid: &str,
    node_origin: &str,
    patient: Uuid,
    retraction: &AllergyRetraction<'_>,
    author: &AuthorParams<'_>,
) -> anyhow::Result<Uuid> {
    let hlc = crate::db::next_hlc(client, node_origin).await?;
    let b = body(
        patient,
        node_kid,
        hlc,
        (
            ALLERGY_RETRACTION_EVENT_TYPE,
            ALLERGY_RETRACTION_SCHEMA_VERSION,
        ),
        allergy_retraction_payload(retraction),
        render_allergy_retraction_twin(retraction),
    );
    seal_sign_submit(client, node_sk, b, Some(author), None).await
}
