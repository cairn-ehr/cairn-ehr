//! A progress note's DRAFT — the Rust face of db/065's doors.
//!
//! A draft holds the RAW text as typed (the window escapes markup only when it signs, ADR-0041),
//! is stamped with recording times by the database's clock (the window never sends one), and
//! belongs to one chart on one terminal. These calls are thin on purpose: every rule lives in
//! the doors, so a raw-SQL client meets the same rules this code does.
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
    /// The draft belongs to another chart or another terminal.
    Elsewhere,
    /// The patient has no chart on this node (no event names them).
    NoChart,
}

/// Map the door's answer word to an outcome. An unknown word is an ERROR, never a default:
/// a future door outcome this build predates must not be read as "saved".
fn outcome_from_word(word: &str) -> anyhow::Result<SaveOutcome> {
    match word {
        "saved" => Ok(SaveOutcome::Saved),
        "spent" => Ok(SaveOutcome::Spent),
        "elsewhere" => Ok(SaveOutcome::Elsewhere),
        "no-chart" => Ok(SaveOutcome::NoChart),
        other => {
            anyhow::bail!("the note draft save answered {other:?}, which this build does not know")
        }
    }
}

/// Save `narrative` (raw, as typed) and the writer's stated time (`None` = not stated) into
/// draft `draft` of `patient`, held by `terminal`. Creates the draft on first save. The door
/// moves `last_keystroke` only when the text or stated time really changed.
pub async fn save_draft(
    client: &(impl GenericClient + Sync),
    draft: Uuid,
    patient: Uuid,
    terminal: &str,
    narrative: &str,
    stated: Option<DateTime<Utc>>,
) -> anyhow::Result<SaveOutcome> {
    let word: String = client
        .query_one(
            "SELECT cairn_note_draft_save($1::text::uuid, $2::text::uuid, $3, $4, $5)",
            &[
                &draft.to_string(),
                &patient.to_string(),
                &terminal,
                &narrative,
                &stated,
            ],
        )
        .await?
        .get(0);
    outcome_from_word(&word)
}

/// One open draft, as the "resume a draft" list shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteDraft {
    pub draft_id: Uuid,
    pub terminal_id: String,
    /// The RAW text as typed.
    pub narrative: String,
    pub stated_time: Option<DateTime<Utc>>,
    pub recording_started: DateTime<Utc>,
    pub last_keystroke: DateTime<Utc>,
}

/// The open (unsigned) drafts on `patient`'s chart, oldest first.
pub async fn open_drafts(
    client: &(impl GenericClient + Sync),
    patient: Uuid,
) -> anyhow::Result<Vec<NoteDraft>> {
    let rows = client
        .query(
            "SELECT draft_id::text, terminal_id, narrative, stated_time, recording_started, \
                    last_keystroke \
               FROM cairn_note_drafts($1::text::uuid)",
            &[&patient.to_string()],
        )
        .await?;
    rows.iter()
        .map(|r| {
            Ok(NoteDraft {
                draft_id: r.get::<_, String>(0).parse()?,
                terminal_id: r.get(1),
                narrative: r.get(2),
                stated_time: r.get(3),
                recording_started: r.get(4),
                last_keystroke: r.get(5),
            })
        })
        .collect()
}

/// A draft as the spend door handed it back: the row the note is built from.
pub(crate) struct SpentDraft {
    pub patient_id: Uuid,
    pub narrative: String,
    pub stated_time: Option<DateTime<Utc>>,
    pub recording_started: DateTime<Utc>,
    pub last_keystroke: DateTime<Utc>,
}

/// Spend draft `draft` into note event `event`: the door deletes the draft and records the
/// spend, and a deferred trigger refuses the COMMIT unless `event` is a note of the same chart
/// submitted in the same transaction. `None` when the draft is absent or already spent.
pub(crate) async fn spend_draft(
    client: &(impl GenericClient + Sync),
    draft: Uuid,
    event: Uuid,
) -> anyhow::Result<Option<SpentDraft>> {
    let rows = client
        .query(
            "SELECT patient_id::text, narrative, stated_time, recording_started, last_keystroke \
               FROM cairn_note_draft_spend($1::text::uuid, $2::text::uuid)",
            &[&draft.to_string(), &event.to_string()],
        )
        .await?;
    let Some(r) = rows.first() else {
        return Ok(None);
    };
    Ok(Some(SpentDraft {
        patient_id: r.get::<_, String>(0).parse()?,
        narrative: r.get(1),
        stated_time: r.get(2),
        recording_started: r.get(3),
        last_keystroke: r.get(4),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_door_word_maps_and_an_unknown_one_is_an_error() {
        assert_eq!(outcome_from_word("saved").unwrap(), SaveOutcome::Saved);
        assert_eq!(outcome_from_word("spent").unwrap(), SaveOutcome::Spent);
        assert_eq!(
            outcome_from_word("elsewhere").unwrap(),
            SaveOutcome::Elsewhere
        );
        assert_eq!(outcome_from_word("no-chart").unwrap(), SaveOutcome::NoChart);
        assert!(outcome_from_word("merged").is_err());
    }
}
