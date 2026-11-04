//! A progress note's DRAFT — the Rust face of db/066's doors.
//!
//! Since slice 3b-1 a draft is a HEADER (its chart, the terminal holding it, its times) plus an
//! ordered list of PARTS, one writer per part: several people may write into one note before one
//! of them signs it, and each person's words stay theirs. A save rewrites the saver's own tail
//! part, or appends a new part; no call here can address another writer's part.
//!
//! A part holds the RAW text as typed (the window escapes markup only when it signs, ADR-0041), is
//! stamped with recording times by the database's clock (the window never sends one), and the
//! whole draft belongs to one chart on one terminal — until a [`take_draft`] moves it. These calls
//! are thin on purpose: every rule lives in the doors, so a raw-SQL client meets the same rules
//! this code does.
use chrono::{DateTime, Utc};
use tokio_postgres::GenericClient;
use uuid::Uuid;

/// What a save answered. Every outcome but `Saved` means NOTHING was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveOutcome {
    /// The text (and stated time) is stored; a new draft was created if none existed.
    Saved,
    /// The draft was already signed: a late debounced save must not resurrect it.
    Spent,
    /// The draft is still on this chart, but ANOTHER terminal holds it now (it was taken there).
    Moved,
    /// The draft belongs to another chart.
    Elsewhere,
    /// The patient has no chart on this node (no event names them).
    NoChart,
    /// The writer named is not an enrolled member (and is not "unknown", `None`).
    NoSuchWriter,
}

/// Map the save door's answer word to an outcome. An unknown word is an ERROR, never a default:
/// a future door outcome this build predates must not be read as "saved".
fn outcome_from_word(word: &str) -> anyhow::Result<SaveOutcome> {
    match word {
        "saved" => Ok(SaveOutcome::Saved),
        "spent" => Ok(SaveOutcome::Spent),
        "moved" => Ok(SaveOutcome::Moved),
        "elsewhere" => Ok(SaveOutcome::Elsewhere),
        "no-chart" => Ok(SaveOutcome::NoChart),
        "no-such-writer" => Ok(SaveOutcome::NoSuchWriter),
        other => {
            anyhow::bail!("the note draft save answered {other:?}, which this build does not know")
        }
    }
}

/// Save `writer`'s text — `narrative` (raw, as typed) and their stated time (`None` = not
/// stated) — into draft `draft` of `patient`, held by `terminal`. `writer` is a vault staff id
/// (the door normalises case and spaces) or `None` for a writer not known. Creates the draft on
/// first save. The door rewrites `writer`'s own tail part, or appends a new part, and moves
/// `last_keystroke` only when the text or stated time really changed.
pub async fn save_part(
    client: &(impl GenericClient + Sync),
    draft: Uuid,
    patient: Uuid,
    terminal: &str,
    writer: Option<&str>,
    narrative: &str,
    stated: Option<DateTime<Utc>>,
) -> anyhow::Result<SaveOutcome> {
    let word: String = client
        .query_one(
            "SELECT cairn_note_draft_part_save($1::text::uuid, $2::text::uuid, $3, $4, $5, $6)",
            &[
                &draft.to_string(),
                &patient.to_string(),
                &terminal,
                &writer,
                &narrative,
                &stated,
            ],
        )
        .await?
        .get(0);
    outcome_from_word(&word)
}

/// One writer's part of an open draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftPart {
    /// 1, 2, 3 … in writing order.
    pub seq: i32,
    /// The writer's staff id; `None` = the writer is not known.
    pub writer: Option<String>,
    /// The writer's display name (their staff id if the vault has none); `None` when unknown.
    pub writer_name: Option<String>,
    /// The RAW text as typed.
    pub narrative: String,
    pub stated_time: Option<DateTime<Utc>>,
    pub recording_started: DateTime<Utc>,
    pub last_keystroke: DateTime<Utc>,
}

/// One open draft, as the "resume a draft" list shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteDraft {
    pub draft_id: Uuid,
    pub terminal_id: String,
    /// When its first part was begun.
    pub recording_started: DateTime<Utc>,
    /// The latest real change over all its parts.
    pub last_keystroke: DateTime<Utc>,
    /// Its parts in writing order (never empty: a draft is created with its part 1).
    pub parts: Vec<DraftPart>,
}

/// The open (unsigned) drafts on `patient`'s chart, oldest first, each with its parts. The door
/// answers one row per part, already ordered by draft and then part, so consecutive rows with
/// the same draft id are one draft.
pub async fn open_drafts(
    client: &(impl GenericClient + Sync),
    patient: Uuid,
) -> anyhow::Result<Vec<NoteDraft>> {
    let rows = client
        .query(
            "SELECT draft_id::text, terminal_id, seq, writer, writer_name, narrative, stated_time, \
                    recording_started, last_keystroke \
               FROM cairn_note_draft_parts($1::text::uuid)",
            &[&patient.to_string()],
        )
        .await?;
    let mut drafts: Vec<NoteDraft> = Vec::new();
    for r in &rows {
        let draft_id: Uuid = r.get::<_, String>(0).parse()?;
        let part = DraftPart {
            seq: r.get(2),
            writer: r.get(3),
            writer_name: r.get(4),
            narrative: r.get(5),
            stated_time: r.get(6),
            recording_started: r.get(7),
            last_keystroke: r.get(8),
        };
        match drafts.last_mut() {
            Some(d) if d.draft_id == draft_id => {
                d.recording_started = d.recording_started.min(part.recording_started);
                d.last_keystroke = d.last_keystroke.max(part.last_keystroke);
                d.parts.push(part);
            }
            _ => drafts.push(NoteDraft {
                draft_id,
                terminal_id: r.get(1),
                recording_started: part.recording_started,
                last_keystroke: part.last_keystroke,
                parts: vec![part],
            }),
        }
    }
    Ok(drafts)
}

/// What a take ("continue this draft at this terminal") answered. Every outcome but `Taken`
/// means nothing changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TakeOutcome {
    /// The draft is now held by this terminal; the terminal it left answers `Moved` to its saves.
    Taken,
    /// The draft was already signed.
    Spent,
    /// The draft belongs to another chart.
    Elsewhere,
    /// No such draft (never saved, or discarded).
    Gone,
}

/// Map the take door's answer word; an unknown word is an error, as for a save.
fn take_from_word(word: &str) -> anyhow::Result<TakeOutcome> {
    match word {
        "taken" => Ok(TakeOutcome::Taken),
        "spent" => Ok(TakeOutcome::Spent),
        "elsewhere" => Ok(TakeOutcome::Elsewhere),
        "gone" => Ok(TakeOutcome::Gone),
        other => {
            anyhow::bail!("the note draft take answered {other:?}, which this build does not know")
        }
    }
}

/// Move draft `draft` of `patient` to `terminal`: one statement on the node holding the draft.
/// Only the draft's terminal changes — no text, time or part.
pub async fn take_draft(
    client: &(impl GenericClient + Sync),
    draft: Uuid,
    patient: Uuid,
    terminal: &str,
) -> anyhow::Result<TakeOutcome> {
    let word: String = client
        .query_one(
            "SELECT cairn_note_draft_take($1::text::uuid, $2::text::uuid, $3)",
            &[&draft.to_string(), &patient.to_string(), &terminal],
        )
        .await?
        .get(0);
    take_from_word(&word)
}

/// One non-empty part as the spend door handed it back: what a note event is built from.
pub(crate) struct SpentPart {
    pub patient_id: Uuid,
    /// The writer's staff id; `None` = not known. Decides the part's contributor set.
    pub writer: Option<String>,
    pub narrative: String,
    pub stated_time: Option<DateTime<Utc>>,
    pub recording_started: DateTime<Utc>,
    pub last_keystroke: DateTime<Utc>,
}

/// Spend draft `draft` into note event `event` (part 1's): the door returns the draft's
/// NON-empty parts in writing order, deletes the draft and records the spend, and a deferred
/// trigger refuses the COMMIT unless `event` is a note of the same chart submitted in the same
/// transaction. Empty means NOTHING was spent: the draft is absent, already spent, or has no
/// non-empty part.
pub(crate) async fn spend_parts(
    client: &(impl GenericClient + Sync),
    draft: Uuid,
    event: Uuid,
) -> anyhow::Result<Vec<SpentPart>> {
    let rows = client
        .query(
            "SELECT patient_id::text, writer, narrative, stated_time, recording_started, \
                    last_keystroke \
               FROM cairn_note_draft_spend_parts($1::text::uuid, $2::text::uuid)",
            &[&draft.to_string(), &event.to_string()],
        )
        .await?;
    rows.iter()
        .map(|r| {
            Ok(SpentPart {
                patient_id: r.get::<_, String>(0).parse()?,
                writer: r.get(1),
                narrative: r.get(2),
                stated_time: r.get(3),
                recording_started: r.get(4),
                last_keystroke: r.get(5),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_door_word_maps_and_an_unknown_one_is_an_error() {
        for (word, outcome) in [
            ("saved", SaveOutcome::Saved),
            ("spent", SaveOutcome::Spent),
            ("moved", SaveOutcome::Moved),
            ("elsewhere", SaveOutcome::Elsewhere),
            ("no-chart", SaveOutcome::NoChart),
            ("no-such-writer", SaveOutcome::NoSuchWriter),
        ] {
            assert_eq!(outcome_from_word(word).unwrap(), outcome, "{word}");
        }
        assert!(outcome_from_word("merged").is_err());
        for (word, outcome) in [
            ("taken", TakeOutcome::Taken),
            ("spent", TakeOutcome::Spent),
            ("elsewhere", TakeOutcome::Elsewhere),
            ("gone", TakeOutcome::Gone),
        ] {
            assert_eq!(take_from_word(word).unwrap(), outcome, "{word}");
        }
        assert!(take_from_word("moved").is_err());
    }
}
