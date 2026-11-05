//! Signing a progress note: a draft becomes sealed, human-signed `clinical.note.asserted` events,
//! ONE PER WRITER'S PART (sign-in slice 3b-1).
//!
//! THE ONE-TRANSACTION SHAPE, and why. Sign carries the text on screen. In ONE transaction we
//! (1) save that text into the draft as the on-screen writer's part — so the note is what was
//! sent WITH Sign, never an older autosave (Review Focus 4) — (2) spend the draft, which hands
//! back its non-empty parts in writing order, (3) build one note per part from the SPENT rows,
//! and (4) submit each through the strict door. Either all of it commits or none of it does: a
//! refused part 3 takes parts 1 and 2 with it, a refused sign leaves the draft exactly as it was,
//! and a draft can never be both spent and unsigned (db/065's commit-time trigger refuses a spend
//! with no note of the same chart behind it).
//!
//! WHO EACH PART SAYS WROTE IT (design §3.4). The first non-blank part is the note; each later
//! non-blank part is an addendum to the part signed just before it (`refs: addendum-to`). The signer's OWN part — decided by key,
//! the writer's current vault key equal to the signer's (ruling R-B) — signs as 3a's did,
//! `{signer, authored}`. Anyone else's part is `{writer, drafted} + {signer, attested}`: the
//! signer answers for words they did not write. A part by a writer not known names no writer at
//! all (principle 4), and a named writer whose member row has gone refuses the whole sign
//! (`NoSuchWriter`) rather than quietly becoming "not known".
//!
//! Text is escaped here, once, at signing (`EscapedNarrative::of_typed`): drafts hold the raw text, and
//! the floor's markup refusals are for raw-SQL clients, never for a clinician's typing.
use cairn_event::contributor::with_attesting_signer;
use cairn_event::note::{
    note_payload, note_t_effective, render_note_twin, EscapedNarrative, NoteAssertion, StatedTime,
    NOTE_EVENT_TYPE, NOTE_SCHEMA_VERSION,
};
use cairn_event::{EventBody, Hlc, SigningKey};
use chrono::{DateTime, SecondsFormat, Utc};
use tokio_postgres::GenericClient;
use uuid::Uuid;

use super::draft::{draft_addendum_to, save_part, spend_parts, SaveOutcome, SpentPart};
use crate::db_diagnosis::Committed;
use crate::medication::sealed_submit::{prepare_sealed, prepare_sealed_as, Prepared};
use crate::medication::AuthorParams;

/// What the window sends with Sign.
pub struct SignNoteInput<'a> {
    pub draft: Uuid,
    /// The chart on screen. The draft must belong to it (else `Elsewhere`).
    pub patient: Uuid,
    pub terminal: &'a str,
    /// Who is writing the part on screen: a vault staff id, or `None` for a writer not known.
    pub writer: Option<&'a str>,
    /// The RAW text on screen, as typed.
    pub narrative: &'a str,
    /// The writer's stated time; `None` = not stated (principle 4).
    pub stated: Option<DateTime<Utc>>,
    /// This node's id, the HLC's origin (`identity::load_local`'s `node_id_hex`).
    pub node_origin: &'a str,
    /// The note this draft adds to (the ROOT of the compound pressed on, slice 3c-1); `None` = a
    /// new note. It only picks the save door: the signed event's target is read back from the
    /// draft's header, never taken from here.
    pub addendum_to: Option<Uuid>,
}

/// Why a sign wrote nothing — typed, so the window can word each case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignRefusal {
    /// No part of the draft holds anything but whitespace.
    Empty,
    /// This draft was already signed.
    Spent,
    /// The draft belongs to another chart than the one signing it.
    Elsewhere,
    /// The draft is on this chart, but another terminal holds it now.
    Moved,
    /// The patient has no chart on this node.
    NoChart,
    /// A writer named (on screen, or by one of the draft's parts) is not an enrolled member.
    NoSuchWriter,
    /// The quarantine timer already put the draft into the record unsigned (slice 3b-2a).
    Committed,
    /// The draft was discarded (slice 3b-2b): it is no longer open.
    Discarded,
    /// The note this addendum adds to is not a note of this chart on this node (slice 3c-1).
    NoSuchNote,
}

/// RFC 3339 UTC, to the microsecond — the precision Postgres stores and the floor's
/// `cairn_t_effective` accepts (at most six fractional digits). Shared with `discard.rs`.
pub(super) fn rfc3339(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Micros, true)
}

/// Assemble the CLEAR, device-shaped note body; the sign rewrites it to human authorship and
/// seals it. Pure. `a.narrative` must already be escaped. Public so a test can build the
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

/// Build the note from a SPENT draft part (ruling R1): its text escaped here, its recording
/// times as the database stamped them, `recording_ended` = the part's last real keystroke.
/// `addendum_to` is the event of the part signed just before this one; for the first non-blank
/// part, the root its draft's header adds to (`None` for a new note). Shared with the quarantine
/// commit (`commit.rs`), which builds the same body.
pub(super) fn body_from_spent(
    event_id: Uuid,
    spent: &SpentPart,
    node_kid: &str,
    hlc: Hlc,
    addendum_to: Option<Uuid>,
) -> EventBody {
    let stated = spent.stated_time.map(rfc3339);
    let started = rfc3339(spent.recording_started);
    let ended = rfc3339(spent.last_keystroke);
    let addendum_to = addendum_to.map(|id| id.to_string());
    let a = NoteAssertion {
        narrative: EscapedNarrative::of_typed(&spent.narrative),
        stated: match &stated {
            Some(t) => StatedTime::Stated(t),
            None => StatedTime::NotStated,
        },
        recording_started: &started,
        recording_ended: &ended,
        addendum_to: addendum_to.as_deref(),
    };
    note_body(event_id, spent.patient_id, node_kid, hlc, &a)
}

/// The refusal a non-`Saved` save answer means; `None` for `Saved`.
fn refusal_for(outcome: SaveOutcome) -> Option<SignRefusal> {
    match outcome {
        SaveOutcome::Saved => None,
        SaveOutcome::Spent => Some(SignRefusal::Spent),
        SaveOutcome::Elsewhere => Some(SignRefusal::Elsewhere),
        SaveOutcome::Moved => Some(SignRefusal::Moved),
        SaveOutcome::NoChart => Some(SignRefusal::NoChart),
        SaveOutcome::NoSuchWriter => Some(SignRefusal::NoSuchWriter),
        SaveOutcome::Committed => Some(SignRefusal::Committed),
        SaveOutcome::Discarded => Some(SignRefusal::Discarded),
        SaveOutcome::NoSuchNote => Some(SignRefusal::NoSuchNote),
    }
}

/// Whose words a part is, as its contributor set must say.
#[derive(Debug, PartialEq, Eq)]
enum PartAuthor {
    /// The signer wrote it: `{signer, authored}` (3a's `with_human_author`).
    Own,
    /// Someone else wrote it (`Some` = their key) or nobody knows who (`None`): the signer
    /// attests it.
    Attested { writer_kid: Option<String> },
}

/// Decide a part's authorship from its writer (`None` = not known), that writer's CURRENT vault
/// key (`cairn_vault_member_key`, `None` = no such member) and the signer's key. Pure. By KEY,
/// never by comparing staff ids (ruling R-B): the key is what the signature proves. A named
/// writer with no key has left the vault, and is refused rather than read as "not known".
fn part_author(
    writer: Option<&str>,
    writer_kid: Option<String>,
    signer_kid: &str,
) -> Result<PartAuthor, SignRefusal> {
    match (writer, writer_kid) {
        (None, _) => Ok(PartAuthor::Attested { writer_kid: None }),
        (Some(_), None) => Err(SignRefusal::NoSuchWriter),
        (Some(_), Some(kid)) if kid == signer_kid => Ok(PartAuthor::Own),
        (Some(_), Some(kid)) => Ok(PartAuthor::Attested {
            writer_kid: Some(kid),
        }),
    }
}

/// The current signing key of vault member `writer` (`None` for no such member), when a part
/// names one; `None` without a query for a writer not known. Shared with `commit.rs`.
pub(super) async fn writer_key(
    client: &(impl GenericClient + Sync),
    writer: Option<&str>,
) -> anyhow::Result<Option<String>> {
    let Some(w) = writer else { return Ok(None) };
    Ok(client
        .query_one("SELECT cairn_vault_member_key($1)", &[&w])
        .await?
        .get(0))
}

/// Seal and sign one part's clear body as `who` says: the signer's own part through
/// `prepare_sealed` (which applies `with_human_author` once), another's through
/// `with_attesting_signer` and `prepare_sealed_as`, signed by the signer either way.
async fn seal_part(
    client: &(impl GenericClient + Sync),
    node_sk: &SigningKey,
    body: EventBody,
    who: PartAuthor,
    author: &AuthorParams<'_>,
) -> anyhow::Result<Prepared> {
    match who {
        PartAuthor::Own => prepare_sealed(client, node_sk, body, Some(author)).await,
        PartAuthor::Attested { writer_kid } => {
            let body = with_attesting_signer(body, author.human_kid, writer_kid.as_deref());
            prepare_sealed_as(client, body, author.human_sk).await
        }
    }
}

/// Sign the draft in `input` as `author`, one event per non-empty part, spending the draft.
/// `Ok(Ok(_))` names the parts' events in writing order (`event_id` = the first non-blank
/// part's, the note);
/// `Ok(Err(_))` is a refusal that wrote nothing; `Err` is a database failure or a floor
/// refusal, which also wrote nothing — unless the commit's answer was lost, which
/// `commit_failure` marks.
pub async fn sign_note(
    client: &mut tokio_postgres::Client,
    node_sk: &SigningKey,
    input: &SignNoteInput<'_>,
    author: &AuthorParams<'_>,
) -> anyhow::Result<Result<Committed<Vec<Uuid>>, SignRefusal>> {
    let first = Uuid::now_v7();
    let node_kid = hex::encode(node_sk.verifying_key().to_bytes());

    // Every early return below drops `tx` uncommitted, which rolls the whole sign back.
    let tx = client.transaction().await?;
    let saved = save_part(
        &tx,
        input.draft,
        input.patient,
        input.terminal,
        input.writer,
        input.narrative,
        input.stated,
        input.addendum_to,
    )
    .await?;
    if let Some(refusal) = refusal_for(saved) {
        return Ok(Err(refusal));
    }
    // What the first part adds to, from the HEADER (the spend below deletes it): read now, while
    // this transaction holds the draft's lock. `None` = a new note.
    let root = draft_addendum_to(&tx, input.draft).await?;
    // `Saved` means the draft is unspent, and this transaction holds its per-draft lock until it
    // ends, so nobody can spend it in between: no part back means every part is blank — or, for
    // an addendum whose first text was blank, that db/070 made no header at all. Either way there
    // is nothing to sign.
    let parts = spend_parts(&tx, input.draft, first).await?;
    if parts.is_empty() {
        return Ok(Err(SignRefusal::Empty));
    }
    let mut ids: Vec<Uuid> = Vec::with_capacity(parts.len());
    for part in &parts {
        let kid = writer_key(&tx, part.writer.as_deref()).await?;
        let who = match part_author(part.writer.as_deref(), kid, author.human_kid) {
            Ok(who) => who,
            Err(refusal) => return Ok(Err(refusal)),
        };
        let id = if ids.is_empty() {
            first
        } else {
            Uuid::now_v7()
        };
        // One tick per part, INSIDE the transaction: a refused sign rolls its ticks back too.
        let hlc = crate::db::next_hlc(&tx, input.node_origin).await?;
        // The first part adds to the header's root (if any); each later one to the part before.
        let body = body_from_spent(id, part, &node_kid, hlc, ids.last().copied().or(root));
        let Prepared {
            signed_bytes, dek, ..
        } = seal_part(&tx, node_sk, body, who, author).await?;
        // NO savepoint (no nested `transaction()`) around this submit, deliberately. db/065's
        // commit-time trigger accepts the spend only if the note row's xmin is THIS
        // transaction's id; a row inserted inside a subtransaction carries the
        // subtransaction's id instead, and the commit would refuse a perfectly good sign.
        tx.execute(
            "SELECT submit_event($1, NULL, NULL, $2)",
            &[&signed_bytes, &dek.as_bytes().as_slice()],
        )
        .await?;
        ids.push(id);
    }
    tx.commit()
        .await
        .map_err(|e| crate::db_diagnosis::commit_failure(first, e.into()))?;
    Ok(Ok(Committed {
        value: ids,
        event_id: first,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spent(stated: Option<&str>) -> SpentPart {
        SpentPart {
            patient_id: Uuid::nil(),
            writer: None,
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
        let b = body_from_spent(Uuid::nil(), &spent(None), "K", hlc(), None);
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
            None,
        );
        assert_eq!(b.payload["effective_basis"], "stated");
        assert_eq!(
            b.t_effective.as_deref(),
            Some("2026-11-03T03:00:00.000000Z")
        );
    }

    #[test]
    fn a_later_part_is_an_addendum_to_the_one_before() {
        let before = Uuid::now_v7();
        let b = body_from_spent(Uuid::nil(), &spent(None), "K", hlc(), Some(before));
        assert_eq!(
            b.payload["refs"],
            serde_json::json!([{"rel": "addendum-to", "event": before.to_string()}])
        );
    }

    /// Ruling R-B: "own" is decided by KEY; a named writer with no key is refused, never
    /// read as a writer not known.
    #[test]
    fn a_parts_author_is_decided_by_key() {
        let kid = |k: &str| Some(k.to_string());
        assert_eq!(part_author(Some("me"), kid("S"), "S"), Ok(PartAuthor::Own));
        assert_eq!(
            part_author(Some("nurse-a"), kid("W"), "S"),
            Ok(PartAuthor::Attested {
                writer_kid: kid("W")
            })
        );
        assert_eq!(
            part_author(None, None, "S"),
            Ok(PartAuthor::Attested { writer_kid: None })
        );
        assert_eq!(
            part_author(Some("gone"), None, "S"),
            Err(SignRefusal::NoSuchWriter)
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
        assert_eq!(refusal_for(SaveOutcome::Moved), Some(SignRefusal::Moved));
        assert_eq!(
            refusal_for(SaveOutcome::NoSuchWriter),
            Some(SignRefusal::NoSuchWriter)
        );
        assert_eq!(
            refusal_for(SaveOutcome::Committed),
            Some(SignRefusal::Committed)
        );
        assert_eq!(
            refusal_for(SaveOutcome::Discarded),
            Some(SignRefusal::Discarded)
        );
        assert_eq!(
            refusal_for(SaveOutcome::NoSuchNote),
            Some(SignRefusal::NoSuchNote)
        );
    }
}
