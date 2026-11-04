//! The note pane's Discard (sign-in slice 3b-2b): throw away the draft on screen, leaving a signed
//! record that it existed (`cairn_node::note::discard`).
//!
//! `note_discard_impl` follows `notes::note_sign_impl` step for step, because a discard is
//! signed by the same person at the same gesture: the chart on screen, fixture mode, every
//! verdict on the gesture BEFORE the key (a passphrase that discards nothing is a step paper
//! never has), the terminal, the key, then the one-transaction node call. It differs in two ways:
//! an empty editor is allowed (discarding an empty draft is quiet and real, and the node answers
//! `Empty`), and the words say "discarded", never "signed". Every sentence is a constant or a
//! pure function so the wording is pinned without a database (`notes_discard_tests.rs`).
use super::time::{parse_stated, year_the_record_holds};
use super::words::{DISCARDED, GONE, NO_CHART, SPENT};
use super::{terminal_of, NoteSignReport, FIXTURE};
use crate::commands::record_timing;
use crate::funnel::view::{ErrorView, Recorded};
use crate::link::judgement_view::{refused, stale};
use crate::signer::Credential;
use crate::state::{AppState, Now};
use crate::write_failure::{key_locked, node_cannot_write, node_failure};
use cairn_node::note::discard::{DiscardOutcome, DiscardRefusal};
use chrono::{DateTime, Utc};
use std::time::Instant;
use uuid::Uuid;

/// What the clinician reads when the discard was recorded.
pub(crate) const RECORDED: &str =
    "Discarded. The draft is gone from the chart; the record keeps what it said.";
/// … when the draft was empty: nothing to keep, so nothing was recorded.
pub(crate) const EMPTY_DISCARDED: &str = "Discarded. The draft was empty, so nothing was kept.";
/// … when a colleague wrote part of the draft: only its writer may throw their part away.
const NOT_YOURS: &str = "Not discarded: someone else wrote part of this draft, and only its \
     writer may throw their part away. The draft's own line says whether, and when, it goes into the \
     record by itself.";
/// … when the draft is another chart's.
const ELSEWHERE: &str = "Not discarded: this draft belongs to another chart — nothing was thrown \
     away; your text is still here.";
/// … when another terminal holds the draft now.
const MOVED: &str = "Not discarded: this note is now being continued at another terminal — \
     nothing was thrown away; your text is still here.";
/// … when the on-screen writer, or the writer of a part above, is not an enrolled member.
const NO_SUCH_WRITER: &str =
    "Not discarded: the name at this terminal is not an enrolled member — \
     say who's here again; your text is still here.";
/// … when the quarantine timer already put the draft into the record unsigned.
const COMMITTED: &str = "Not discarded: this note had already gone into the record unsigned.";
/// … when this staff ID's role may not act clinically.
const NO_SUCH_SIGNER: &str = "Not discarded: this staff ID may not discard a clinical note.";
/// The prefix every refusal borrowed from the sign's wording carries.
const NOT_DISCARDED: &str = "Not discarded: ";

/// Discard the draft on screen (the webview sends the same object as for Sign).
#[tauri::command]
pub async fn note_discard(
    state: tauri::State<'_, AppState>,
    patient_id: String,
    draft_id: String,
    narrative: String,
    stated_time: Option<String>,
    writer: Option<String>,
    credential: Option<Credential>,
) -> Result<NoteSignReport, ErrorView> {
    note_discard_impl(
        &state,
        &patient_id,
        &draft_id,
        &narrative,
        stated_time.as_deref(),
        writer.as_deref(),
        credential,
    )
    .await
}

/// A refusal as the webview reads it. `Spent`, `Committed` and `Discarded` mean the pane is
/// behind the record, so they ask for a re-read (`stale`); the rest are verdicts a re-read cannot
/// mend. Pure.
pub(crate) fn discard_refusal_view(refusal: DiscardRefusal) -> ErrorView {
    let not_discarded = |why: &str| refused(format!("{NOT_DISCARDED}{why}"));
    match refusal {
        DiscardRefusal::Spent => stale(format!("{NOT_DISCARDED}{SPENT}")),
        DiscardRefusal::Committed => stale(COMMITTED),
        DiscardRefusal::Discarded => stale(DISCARDED),
        DiscardRefusal::Elsewhere => refused(ELSEWHERE),
        DiscardRefusal::Moved => refused(MOVED),
        DiscardRefusal::NoChart => not_discarded(NO_CHART),
        DiscardRefusal::NoSuchWriter => refused(NO_SUCH_WRITER),
        DiscardRefusal::Gone => not_discarded(GONE),
        DiscardRefusal::NotYours => refused(NOT_YOURS),
        DiscardRefusal::NoSuchSigner => refused(NO_SUCH_SIGNER),
    }
}

/// The verdicts on the gesture that are said before the key is asked for: an unreadable draft id
/// or stated time. (Unlike a sign, an empty editor is fine.) Pure.
pub(crate) fn discard_checks_before_the_key(
    draft_id: &str,
    stated_time: Option<&str>,
) -> Result<(Uuid, Option<DateTime<Utc>>), ErrorView> {
    let draft = draft_id.parse().map_err(|_| refused("not a draft id"))?;
    let stated = parse_stated(stated_time).map_err(refused)?;
    if let (Some(s), Some(t)) = (stated_time, stated) {
        year_the_record_holds(s, t).map_err(refused)?;
    }
    Ok((draft, stated))
}

/// Discard draft `draft_id` of the chart on screen, carrying the text on screen into the audit
/// copy (as Sign carries it). See the module doc; the steps are `note_sign_impl`'s.
pub async fn note_discard_impl(
    state: &AppState,
    patient_id: &str,
    draft_id: &str,
    narrative: &str,
    stated_time: Option<&str>,
    writer: Option<&str>,
    credential: Option<Credential>,
) -> Result<NoteSignReport, ErrorView> {
    let patient = state.displayed_patient(patient_id).await.map_err(refused)?;
    if state.is_mock() {
        return Err(refused(FIXTURE));
    }
    let (draft, stated) = discard_checks_before_the_key(draft_id, stated_time)?;
    let terminal = terminal_of(state)?;
    let (human_sk, human_kid) = state
        .signer_for_act(credential, Now::read())
        .await
        .map_err(|r| key_locked(&r))?;
    let node_sk = state
        .node_sk
        .as_ref()
        .ok_or_else(|| node_cannot_write("no node key"))?;

    let started = Instant::now();
    let result = {
        let mut db = state
            .db
            .as_ref()
            .ok_or_else(|| node_cannot_write("no database connection"))?
            .lock()
            .await?;
        let input = cairn_node::note::sign::SignNoteInput {
            draft,
            patient,
            terminal,
            writer,
            narrative,
            stated,
            node_origin: &state.node_origin,
        };
        let author = cairn_node::medication::AuthorParams {
            human_sk: &human_sk,
            human_kid: &human_kid,
        };
        cairn_node::note::discard::discard_draft(&mut db, node_sk, &input, &author)
            .await
            .map_err(|e| db.explain(e))
    };
    // ONE write, so one outcome: discarded, refused (nothing written), NOT known (#814), or an
    // ordinary failure returned as the node's own sentence.
    let (text, behind_the_record) = match result {
        Ok(Ok(DiscardOutcome::Recorded(_))) => (RECORDED.to_string(), None),
        Ok(Ok(DiscardOutcome::Empty)) => (EMPTY_DISCARDED.to_string(), None),
        Ok(Err(refusal)) => return Err(discard_refusal_view(refusal)),
        Err(e) => match cairn_node::db_diagnosis::commit_outcome_unknown_event(&e) {
            Some(event) => (
                format!(
                    "Whether this draft was discarded is NOT known — no ordinary answer came \
                     back while saving it (event {event}: {}). Once the window can reach the \
                     node again, the chart shows whether the discard is there.",
                    cairn_gui_live::cause_of(&e)
                ),
                Some(Recorded::Unknown),
            ),
            None => return Err(node_failure(&e)),
        },
    };
    // A gesture with an unknown outcome is no timing sample (#818 review).
    if behind_the_record.is_none() {
        let elapsed_ms = started.elapsed().as_millis().min(i32::MAX as u128) as i32;
        record_timing(state, "note-discard", 1, elapsed_ms).await;
    }
    Ok(NoteSignReport {
        text,
        behind_the_record,
    })
}

#[cfg(test)]
#[path = "notes_discard_tests.rs"]
mod tests;
