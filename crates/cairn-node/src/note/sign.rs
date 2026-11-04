//! Signing a progress note: a draft becomes a sealed, human-signed `clinical.note.asserted`.
//!
//! THE ONE-TRANSACTION SHAPE, and why. Sign carries the text on screen. In ONE transaction we
//! (1) save that text into the draft — so the note is what was sent WITH Sign, never an older
//! autosave (Review Focus 4) — (2) spend the draft, (3) build the note from the SPENT row, and
//! (4) submit it through the strict door. Either all of it commits or none of it does: a refused
//! sign leaves the draft exactly as it was, and a draft can never be both spent and unsigned
//! (db/065's commit-time trigger refuses a spend with no note of the same chart behind it).
//!
//! Text is escaped here, once, at signing (`EscapedNarrative::of_typed`): drafts hold the raw text, and
//! the floor's markup refusals are for raw-SQL clients, never for a clinician's typing.
use cairn_event::note::{
    note_payload, note_t_effective, render_note_twin, EscapedNarrative, NoteAssertion, StatedTime,
    NOTE_EVENT_TYPE, NOTE_SCHEMA_VERSION,
};
use cairn_event::{EventBody, Hlc, SigningKey};
use chrono::{DateTime, SecondsFormat, Utc};
use uuid::Uuid;

use super::draft::{save_draft, spend_draft, SaveOutcome, SpentDraft};
use crate::db_diagnosis::Committed;
use crate::medication::sealed_submit::{prepare_sealed, Prepared};
use crate::medication::AuthorParams;

/// What the window sends with Sign.
pub struct SignNoteInput<'a> {
    pub draft: Uuid,
    /// The chart on screen. The draft must belong to it (else `Elsewhere`).
    pub patient: Uuid,
    pub terminal: &'a str,
    /// The RAW text on screen, as typed.
    pub narrative: &'a str,
    /// The writer's stated time; `None` = not stated (principle 4).
    pub stated: Option<DateTime<Utc>>,
    /// This node's id, the HLC's origin (`identity::load_local`'s `node_id_hex`).
    pub node_origin: &'a str,
}

/// Why a sign wrote nothing — typed, so the window can word each case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignRefusal {
    /// Nothing but whitespace was typed.
    Empty,
    /// This draft was already signed.
    Spent,
    /// The draft belongs to another chart or terminal than the one signing it.
    Elsewhere,
    /// The patient has no chart on this node.
    NoChart,
}

/// RFC 3339 UTC, to the microsecond — the precision Postgres stores and the floor's
/// `cairn_t_effective` accepts (at most six fractional digits).
fn rfc3339(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Micros, true)
}

/// Assemble the CLEAR, device-shaped note body; `prepare_sealed` rewrites it to human authorship
/// and seals it. Pure. `a.narrative` must already be escaped. Public so a test can build the
/// node-only note a peer might send.
pub fn note_body(
    event_id: Uuid,
    patient: Uuid,
    node_kid: &str,
    hlc: Hlc,
    a: &NoteAssertion,
) -> EventBody {
    EventBody {
        event_id: event_id.to_string(),
        patient_id: patient.to_string(),
        event_type: NOTE_EVENT_TYPE.into(),
        schema_version: NOTE_SCHEMA_VERSION.into(),
        hlc,
        t_effective: Some(note_t_effective(a)),
        signer_key_id: node_kid.into(),
        contributors: serde_json::json!([{"actor_id": node_kid, "role": "recorded"}]),
        payload: note_payload(a),
        attachments: vec![],
        plaintext_twin: Some(render_note_twin(a)),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    }
}

/// Build the note from the SPENT draft row (ruling R1): its text escaped here, its recording
/// times as the database stamped them, `recording_ended` = the last real keystroke.
fn body_from_spent(event_id: Uuid, spent: &SpentDraft, node_kid: &str, hlc: Hlc) -> EventBody {
    let stated = spent.stated_time.map(rfc3339);
    let started = rfc3339(spent.recording_started);
    let ended = rfc3339(spent.last_keystroke);
    let a = NoteAssertion {
        narrative: EscapedNarrative::of_typed(&spent.narrative),
        stated: match &stated {
            Some(t) => StatedTime::Stated(t),
            None => StatedTime::NotStated,
        },
        recording_started: &started,
        recording_ended: &ended,
        addendum_to: None,
    };
    note_body(event_id, spent.patient_id, node_kid, hlc, &a)
}

/// The refusal a non-`Saved` save answer means; `None` for `Saved`.
fn refusal_for(outcome: SaveOutcome) -> Option<SignRefusal> {
    match outcome {
        SaveOutcome::Saved => None,
        SaveOutcome::Spent => Some(SignRefusal::Spent),
        SaveOutcome::Elsewhere => Some(SignRefusal::Elsewhere),
        SaveOutcome::NoChart => Some(SignRefusal::NoChart),
    }
}

/// Sign the note in `input` as `author`, spending its draft. `Ok(Ok(_))` names the note's event;
/// `Ok(Err(_))` is a refusal that wrote nothing; `Err` is a database failure or a floor refusal,
/// which also wrote nothing — unless the commit's answer was lost, which `commit_failure` marks.
pub async fn sign_note(
    client: &mut tokio_postgres::Client,
    node_sk: &SigningKey,
    input: &SignNoteInput<'_>,
    author: &AuthorParams<'_>,
) -> anyhow::Result<Result<Committed<Uuid>, SignRefusal>> {
    // Before any database call: an empty note is the clinician's to fix, not the floor's.
    if input.narrative.trim().is_empty() {
        return Ok(Err(SignRefusal::Empty));
    }
    let event_id = Uuid::now_v7();
    // `next_hlc` takes a `Client`, so the tick runs before the transaction. A refused sign
    // leaves a gap in the HLC, which is allowed (it is monotonic, not dense).
    let hlc = crate::db::next_hlc(client, input.node_origin).await?;
    let node_kid = hex::encode(node_sk.verifying_key().to_bytes());

    // Every early return below drops `tx` uncommitted, which rolls the whole sign back.
    let tx = client.transaction().await?;
    let saved = save_draft(
        &tx,
        input.draft,
        input.patient,
        input.terminal,
        input.narrative,
        input.stated,
    )
    .await?;
    if let Some(refusal) = refusal_for(saved) {
        return Ok(Err(refusal));
    }
    // Saved in this transaction under the door's per-draft lock, so the spend finds it; `None`
    // would mean it vanished in between, which is a spend by someone else.
    let Some(spent) = spend_draft(&tx, input.draft, event_id).await? else {
        return Ok(Err(SignRefusal::Spent));
    };
    let Prepared {
        signed_bytes, dek, ..
    } = prepare_sealed(
        &tx,
        node_sk,
        body_from_spent(event_id, &spent, &node_kid, hlc),
        Some(author),
    )
    .await?;
    // NO savepoint (no nested `transaction()`) around this submit, deliberately. db/065's
    // commit-time trigger accepts the spend only if the note row's xmin is THIS transaction's
    // id; a row inserted inside a subtransaction carries the subtransaction's id instead, and
    // the commit would refuse a perfectly good sign.
    tx.execute(
        "SELECT submit_event($1, NULL, NULL, $2)",
        &[&signed_bytes, &dek.as_bytes().as_slice()],
    )
    .await?;
    tx.commit()
        .await
        .map_err(|e| crate::db_diagnosis::commit_failure(event_id, e.into()))?;
    Ok(Ok(Committed {
        value: event_id,
        event_id,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spent(stated: Option<&str>) -> SpentDraft {
        SpentDraft {
            patient_id: Uuid::nil(),
            narrative: "BP <90 [L] arm (R)".into(),
            stated_time: stated.map(|s| s.parse().unwrap()),
            recording_started: "2026-11-03T01:00:00.123456Z".parse().unwrap(),
            last_keystroke: "2026-11-03T01:05:00Z".parse().unwrap(),
        }
    }

    fn hlc() -> Hlc {
        Hlc {
            wall: 1,
            counter: 0,
            node_origin: "n".into(),
        }
    }

    #[test]
    fn the_body_is_the_spent_row_escaped_once() {
        let b = body_from_spent(Uuid::nil(), &spent(None), "K", hlc());
        assert_eq!(b.payload["narrative"], "BP \\<90 [L] arm (R)");
        assert_eq!(b.plaintext_twin.as_deref(), Some("BP <90 [L] arm (R)"));
        assert_eq!(
            b.payload["recording_started"],
            "2026-11-03T01:00:00.123456Z"
        );
        assert_eq!(b.payload["recording_ended"], "2026-11-03T01:05:00.000000Z");
        assert_eq!(b.payload["effective_basis"], "recording-started");
        assert_eq!(
            b.t_effective.as_deref(),
            Some("2026-11-03T01:00:00.123456Z")
        );
    }

    #[test]
    fn a_stated_time_is_the_envelope_time() {
        let b = body_from_spent(
            Uuid::nil(),
            &spent(Some("2026-11-03T03:00:00Z")),
            "K",
            hlc(),
        );
        assert_eq!(b.payload["effective_basis"], "stated");
        assert_eq!(
            b.t_effective.as_deref(),
            Some("2026-11-03T03:00:00.000000Z")
        );
    }

    #[test]
    fn only_saved_proceeds() {
        assert_eq!(refusal_for(SaveOutcome::Saved), None);
        assert_eq!(refusal_for(SaveOutcome::Spent), Some(SignRefusal::Spent));
        assert_eq!(
            refusal_for(SaveOutcome::Elsewhere),
            Some(SignRefusal::Elsewhere)
        );
        assert_eq!(
            refusal_for(SaveOutcome::NoChart),
            Some(SignRefusal::NoChart)
        );
    }
}
