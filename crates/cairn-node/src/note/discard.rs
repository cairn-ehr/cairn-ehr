//! The audited DISCARD of a draft progress note (sign-in slice 3b-2b, ADR-0089 decision 15).
//!
//! On paper you may tear up your OWN unsigned scribble, never a colleague's. But a Cairn draft is
//! autosaved and may have been read by someone else, so "it never existed" would be a quiet
//! falsehood: a discard is a signed `clinical.note-discard.asserted` saying the draft existed and
//! what it said. It is never a note (it never enters `clinical_note`).
//!
//! THE ONE-TRANSACTION SHAPE is `sign::sign_note`'s, step for step, and for the same reasons:
//! (1) save the text on screen as the on-screen writer's part, so the audit copy holds what was
//! on screen when Discard was pressed, never an older autosave; (2) call db/069's door, which
//! applies the rule (every non-blank part is the discarder's own or by a writer not known),
//! consumes the draft and hands back its non-blank parts plus the discard instant it minted under
//! the draft's lock; (3) build ONE event from those rows, its `t_effective` exactly that instant;
//! (4) submit it with NO savepoint and commit. db/065's commit-time trigger refuses the COMMIT
//! unless this transaction inserted a discard event on this chart, signed by the discarder's key,
//! at that instant — which is what stops a raw-SQL client naming a colleague's key.
//!
//! An all-blank draft is different: nothing to audit, so the door deletes it with no spend, and
//! we COMMIT with no event (`DiscardOutcome::Empty`).
use cairn_event::note::{EscapedNarrative, StatedTime};
use cairn_event::note_discard::{
    discard_payload, render_discard_twin, DiscardedPart, DiscardedWriter, NOTE_DISCARD_EVENT_TYPE,
    NOTE_DISCARD_SCHEMA_VERSION,
};
use cairn_event::{EventBody, Hlc, SigningKey};
use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::draft::{save_part, SaveOutcome};
use super::sign::{rfc3339, SignNoteInput};
use crate::db_diagnosis::Committed;
use crate::medication::sealed_submit::{prepare_sealed, Prepared};
use crate::medication::AuthorParams;

/// Why a discard wrote nothing — typed, so the window can word each case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscardRefusal {
    /// The draft was already signed.
    Spent,
    /// The quarantine timer already put the draft into the record unsigned.
    Committed,
    /// The draft was already discarded.
    Discarded,
    /// The draft belongs to another chart than the one on screen.
    Elsewhere,
    /// The draft is on this chart, but another terminal holds it now.
    Moved,
    /// The patient has no chart on this node.
    NoChart,
    /// The writer named on screen is not an enrolled member.
    NoSuchWriter,
    /// A non-blank part was written by someone else (by key, ruling R-B), or by a named writer
    /// who has no key any more: only your own scribble may be thrown away.
    NotYours,
    /// The discarder's key is not a member whose role may act clinically.
    NoSuchSigner,
    /// No such draft.
    Gone,
}

/// What a discard did.
#[derive(Debug)]
pub enum DiscardOutcome {
    /// The draft is gone and this signed discard event records what it said.
    Recorded(Committed<Uuid>),
    /// Every part was blank: the draft was deleted, with no event (nothing to audit).
    Empty,
}

/// The refusal a non-`Saved` save answer means; `None` for `Saved`.
fn refusal_for(outcome: SaveOutcome) -> Option<DiscardRefusal> {
    match outcome {
        SaveOutcome::Saved => None,
        SaveOutcome::Spent => Some(DiscardRefusal::Spent),
        SaveOutcome::Moved => Some(DiscardRefusal::Moved),
        SaveOutcome::Elsewhere => Some(DiscardRefusal::Elsewhere),
        SaveOutcome::NoChart => Some(DiscardRefusal::NoChart),
        SaveOutcome::NoSuchWriter => Some(DiscardRefusal::NoSuchWriter),
        SaveOutcome::Committed => Some(DiscardRefusal::Committed),
        SaveOutcome::Discarded => Some(DiscardRefusal::Discarded),
    }
}

/// Map a refusal word of db/069's door. An unknown word is an ERROR, never a default: a future
/// door outcome this build predates must not be read as any refusal (or as a yes). `ok` and
/// `empty` are not refusals and are handled before this is called.
fn refusal_from_word(word: &str) -> anyhow::Result<DiscardRefusal> {
    Ok(match word {
        "spent" => DiscardRefusal::Spent,
        "committed" => DiscardRefusal::Committed,
        "discarded" => DiscardRefusal::Discarded,
        "moved" => DiscardRefusal::Moved,
        "elsewhere" => DiscardRefusal::Elsewhere,
        "gone" => DiscardRefusal::Gone,
        "not-yours" => DiscardRefusal::NotYours,
        "no-such-signer" => DiscardRefusal::NoSuchSigner,
        other => {
            anyhow::bail!("the note discard answered {other:?}, which this build does not know")
        }
    })
}

/// One `ok` row of the door, its times already in RFC 3339 for the payload.
struct DoorPart {
    by_discarder: bool,
    narrative: String,
    stated: Option<String>,
    started: String,
    ended: String,
}

/// The CLEAR, device-shaped discard body; `prepare_sealed` rewrites it to the discarder's
/// authorship and seals it. Pure. Each narrative is escaped here, once (drafts hold raw text).
fn discard_body(
    event_id: Uuid,
    patient: Uuid,
    node_kid: &str,
    hlc: Hlc,
    discarded_at: DateTime<Utc>,
    rows: &[DoorPart],
) -> EventBody {
    let parts: Vec<DiscardedPart> = rows
        .iter()
        .map(|r| DiscardedPart {
            writer: if r.by_discarder {
                DiscardedWriter::Discarder
            } else {
                DiscardedWriter::NotKnown
            },
            narrative: EscapedNarrative::of_typed(&r.narrative),
            stated: match &r.stated {
                Some(t) => StatedTime::Stated(t),
                None => StatedTime::NotStated,
            },
            recording_started: &r.started,
            recording_ended: &r.ended,
        })
        .collect();
    EventBody {
        event_id: event_id.to_string(),
        patient_id: patient.to_string(),
        event_type: NOTE_DISCARD_EVENT_TYPE.into(),
        schema_version: NOTE_DISCARD_SCHEMA_VERSION.into(),
        hlc,
        // Exactly the door's instant: db/065's trigger compares it to the spend's spent_at.
        t_effective: Some(rfc3339(discarded_at)),
        signer_key_id: node_kid.into(),
        contributors: serde_json::json!([{"actor_id": node_kid, "role": "recorded"}]),
        payload: discard_payload(&parts),
        attachments: vec![],
        plaintext_twin: Some(render_discard_twin(&parts)),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    }
}

/// Discard the draft in `input` as `author`, in one transaction (see the module doc).
/// `Ok(Ok(Recorded))` names the discard event; `Ok(Ok(Empty))` means an all-blank draft was
/// deleted; `Ok(Err(_))` is a refusal that wrote nothing; `Err` is a database failure or a floor
/// refusal, which also wrote nothing — unless the commit's answer was lost, which
/// `db_diagnosis::commit_failure` marks.
pub async fn discard_draft(
    client: &mut tokio_postgres::Client,
    node_sk: &SigningKey,
    input: &SignNoteInput<'_>,
    author: &AuthorParams<'_>,
) -> anyhow::Result<Result<DiscardOutcome, DiscardRefusal>> {
    let event_id = Uuid::now_v7();
    let node_kid = hex::encode(node_sk.verifying_key().to_bytes());

    // Every early return below drops `tx` uncommitted, which rolls the whole discard back.
    let tx = client.transaction().await?;
    let saved = save_part(
        &tx,
        input.draft,
        input.patient,
        input.terminal,
        input.writer,
        input.narrative,
        input.stated,
    )
    .await?;
    if let Some(refusal) = refusal_for(saved) {
        return Ok(Err(refusal));
    }
    let rows = tx
        .query(
            "SELECT outcome, by_discarder, narrative, stated_time, recording_started, \
                    last_keystroke, discarded_at \
               FROM cairn_note_draft_discard($1::text::uuid, $2::text::uuid, $3, \
                                             $4::text::uuid, $5)",
            &[
                &input.draft.to_string(),
                &input.patient.to_string(),
                &input.terminal,
                &event_id.to_string(),
                &author.human_kid,
            ],
        )
        .await?;
    let Some(first) = rows.first() else {
        anyhow::bail!("the note discard answered no row");
    };
    match first.get::<_, &str>(0) {
        "ok" => {}
        "empty" => {
            // The door deleted the all-blank draft; the deletion is the whole act. No event.
            tx.commit().await?;
            return Ok(Ok(DiscardOutcome::Empty));
        }
        word => return Ok(Err(refusal_from_word(word)?)),
    }
    let discarded_at: DateTime<Utc> = first.get(6);
    let parts: Vec<DoorPart> = rows
        .iter()
        .map(|r| DoorPart {
            by_discarder: r.get(1),
            narrative: r.get(2),
            stated: r.get::<_, Option<DateTime<Utc>>>(3).map(rfc3339),
            started: rfc3339(r.get(4)),
            ended: rfc3339(r.get(5)),
        })
        .collect();
    // One tick INSIDE the transaction: a refused discard rolls its tick back too.
    let hlc = crate::db::next_hlc(&tx, input.node_origin).await?;
    let body = discard_body(
        event_id,
        input.patient,
        &node_kid,
        hlc,
        discarded_at,
        &parts,
    );
    let Prepared {
        signed_bytes, dek, ..
    } = prepare_sealed(&tx, node_sk, body, Some(author)).await?;
    // NO savepoint around this submit (see the module doc): db/065's trigger compares the
    // event row's xmin to THIS transaction's id, which a subtransaction would not carry.
    tx.execute(
        "SELECT submit_event($1, NULL, NULL, $2)",
        &[&signed_bytes, &dek.as_bytes().as_slice()],
    )
    .await?;
    tx.commit()
        .await
        .map_err(|e| crate::db_diagnosis::commit_failure(event_id, e.into()))?;
    Ok(Ok(DiscardOutcome::Recorded(Committed {
        value: event_id,
        event_id,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusal_words_map_every_door_word() {
        for (word, refusal) in [
            ("spent", DiscardRefusal::Spent),
            ("committed", DiscardRefusal::Committed),
            ("discarded", DiscardRefusal::Discarded),
            ("moved", DiscardRefusal::Moved),
            ("elsewhere", DiscardRefusal::Elsewhere),
            ("gone", DiscardRefusal::Gone),
            ("not-yours", DiscardRefusal::NotYours),
            ("no-such-signer", DiscardRefusal::NoSuchSigner),
        ] {
            assert_eq!(refusal_from_word(word).unwrap(), refusal, "{word}");
        }
        assert_eq!(refusal_for(SaveOutcome::Saved), None);
        assert_eq!(
            refusal_for(SaveOutcome::Discarded),
            Some(DiscardRefusal::Discarded)
        );
        assert_eq!(
            refusal_for(SaveOutcome::NoChart),
            Some(DiscardRefusal::NoChart)
        );
    }

    #[test]
    fn an_unknown_word_is_an_error() {
        // `ok` and `empty` are not refusals: reaching this map with one is a bug, not a default.
        for word in ["merged", "ok", "empty", ""] {
            assert!(refusal_from_word(word).is_err(), "{word:?}");
        }
    }
}
