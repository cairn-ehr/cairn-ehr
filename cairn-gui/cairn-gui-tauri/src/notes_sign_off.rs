//! "Sign off this note" (sign-in slice 3c-2): a supervisor — or anyone — signs every part of a
//! note they have not yet signed, in one press, as a paper co-signature at the foot of the page.
//!
//! `note_sign_off_impl` follows `notes::note_sign_impl`'s order, because it is a signature at a
//! gesture too: the chart on screen first, then fixture mode, then every verdict on the gesture
//! (here, only whether the ids the webview sent can be read), and only then the key — a passphrase
//! that signs nothing is a step paper never has. The node (`note::vouch::sign_off_note`) re-reads
//! the note in its own transaction and refuses (`Changed`) unless it holds exactly the parts the
//! window showed (`CompoundView::shown_parts`), so a part that landed or went after the clinician
//! read the note is never signed unseen. Every sentence is a constant, pinned in
//! `notes_sign_off_tests.rs`.
use super::{parse_note_ids, NoteSignReport, FIXTURE};
use crate::commands::record_timing;
use crate::funnel::view::{ErrorView, Recorded};
use crate::link::judgement_view::{refused, stale};
use crate::signer::Credential;
use crate::state::{AppState, Now};
use crate::write_failure::{key_locked, node_cannot_write, node_failure};
use cairn_node::note::vouch::{sign_off_note, SignOffInput, SignOffRefusal};
use std::time::Instant;
use uuid::Uuid;

/// What the clinician reads when the sign-off was recorded.
pub(crate) const SIGNED_OFF: &str =
    "Signed off — every part of this note now carries your signature.";
/// … when the note holds other parts than the window showed (a colleague's addendum landed, or a
/// part was erased): nothing was signed, and the re-read shows the note as it now is.
const CHANGED: &str = "This note changed since it was shown — read the chart again, then sign it \
     off. Nothing was signed.";
/// … when the presser has already signed every part (in another window, or a student who signed
/// it all and still needs a supervisor).
const NOTHING_TO_SIGN: &str =
    "You have already signed every part of this note. Nothing more was signed.";

/// Sign off the compound note whose root is `root_event_id` on the chart on screen.
/// `shown_parts` (JS `shownParts`): the note's part ids exactly as the pane showed them
/// (`CompoundView::shown_parts`).
#[tauri::command]
pub async fn note_sign_off(
    state: tauri::State<'_, AppState>,
    patient_id: String,
    root_event_id: String,
    shown_parts: Vec<String>,
    credential: Option<Credential>,
) -> Result<NoteSignReport, ErrorView> {
    note_sign_off_impl(
        &state,
        &patient_id,
        &root_event_id,
        &shown_parts,
        credential,
    )
    .await
}

/// A sign-off's refusal as the webview reads it. Both mean the pane on screen is behind the
/// record, so both ask for a re-read (`stale`); nothing was written either way. Exhaustive, so a
/// new refusal must be worded here. Pure.
pub(crate) fn sign_off_refusal_view(refusal: SignOffRefusal) -> ErrorView {
    match refusal {
        SignOffRefusal::Changed => stale(CHANGED),
        SignOffRefusal::NothingToSign => stale(NOTHING_TO_SIGN),
    }
}

/// The root and shown part ids read from what the webview sent, or "not a note id" — said before
/// the key. Pure.
pub(crate) fn sign_off_ids(root: &str, shown: &[String]) -> Result<(Uuid, Vec<Uuid>), ErrorView> {
    let root = root.parse().map_err(|_| refused("not a note id"))?;
    Ok((root, parse_note_ids(Some(shown))?))
}

/// What a sign-off whose commit got no ordinary answer says (#814): `event` the first vouch's id,
/// `cause` the failure. Never "the note is there": the note exists either way. Pure.
pub(crate) fn sign_off_unknown_words(event: Uuid, cause: &str) -> String {
    format!(
        "Whether this note was signed off is NOT known — no ordinary answer came back while \
         signing it off (event {event}: {cause}). Once the window can reach the node again, the \
         chart shows whether it was signed off."
    )
}

/// Sign off the note (see the module doc for the order of the checks). ONE node call, so one
/// outcome: signed off, refused (nothing written), NOT known (#814: the commit's answer was lost,
/// so the pane may be behind the record), or an ordinary failure in the node's own words.
pub async fn note_sign_off_impl(
    state: &AppState,
    patient_id: &str,
    root_event_id: &str,
    shown_parts: &[String],
    credential: Option<Credential>,
) -> Result<NoteSignReport, ErrorView> {
    let patient = state.displayed_patient(patient_id).await.map_err(refused)?;
    if state.is_mock() {
        return Err(refused(FIXTURE));
    }
    let (root, shown) = sign_off_ids(root_event_id, shown_parts)?;
    let (human_sk, human_kid) = state
        .signer_for_act(credential, Now::read())
        .await
        .map_err(|r| key_locked(&r))?;

    let started = Instant::now();
    let result = {
        let mut db = state
            .db
            .as_ref()
            .ok_or_else(|| node_cannot_write("no database connection"))?
            .lock()
            .await?;
        let input = SignOffInput {
            patient,
            root,
            shown_parts: &shown,
            node_origin: &state.node_origin,
        };
        let author = cairn_node::medication::AuthorParams {
            human_sk: &human_sk,
            human_kid: &human_kid,
        };
        sign_off_note(&mut db, &input, &author)
            .await
            .map_err(|e| db.explain(e))
    };
    let (text, behind_the_record) = match result {
        Ok(Ok(_vouches)) => (SIGNED_OFF.to_string(), None),
        Ok(Err(refusal)) => return Err(sign_off_refusal_view(refusal)),
        Err(e) => match cairn_node::db_diagnosis::commit_outcome_unknown_event(&e) {
            Some(event) => (
                sign_off_unknown_words(event, &cairn_gui_live::cause_of(&e)),
                Some(Recorded::Unknown),
            ),
            None => return Err(node_failure(&e)),
        },
    };
    // A gesture with an unknown outcome is no timing sample (#818 review).
    if behind_the_record.is_none() {
        let elapsed_ms = started.elapsed().as_millis().min(i32::MAX as u128) as i32;
        record_timing(state, "note-sign-off", 1, elapsed_ms).await;
    }
    Ok(NoteSignReport {
        text,
        behind_the_record,
    })
}

#[cfg(test)]
#[path = "notes_sign_off_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "notes_sign_off_db_tests.rs"]
mod db_tests;
