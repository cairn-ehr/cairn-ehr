//! "Co-sign as reviewed" (sign-in slice 3c-3a, design §3.3): a supervisor countersigns the parts
//! their supervisees signed, on a note that owed no co-signature — on paper, initialling a
//! registrar's entry as read.
//!
//! `note_review_cosign_impl` is `sign_off::note_sign_off_impl` step for step (ADR-0092 decision
//! 14): the chart on screen, fixture mode, the ids, then the key, ONE node call
//! (`note::review::review_note`), an unknown outcome (#814), and a timing sample under Sign
//! off's kind (`note-sign-off`: the same gesture shape, so no new timing kind). The node re-reads
//! the note and refuses `Changed` unless it holds exactly the parts the window showed.
//!
//! Where they mean different things, they say different things: a review co-signs only the
//! parts the presser's supervisees signed, so none of its sentences says "signed off" or "every
//! part". WHO is reviewing is the released KEY's person, never the who's-here claim
//! (ADR-0092 decision 16): the claim only decides whether the button is offered
//! (`CompoundView::can_review`). Every sentence is a constant, pinned in `notes_review_tests.rs`.
use super::sign_off::sign_off_ids;
use super::{NoteSignReport, FIXTURE};
use crate::commands::record_timing;
use crate::funnel::view::{ErrorView, Recorded};
use crate::link::judgement_view::{refused, stale};
use crate::signer::Credential;
use crate::state::{AppState, Now};
use crate::write_failure::{key_locked, node_cannot_write, node_failure};
use cairn_node::note::review::review_note;
use cairn_node::note::vouch::{SignOffInput, SignOffRefusal};
use std::time::Instant;
use uuid::Uuid;

/// What the clinician reads when the review was recorded. Role-neutral: where the floor admits
/// only an attested vouch the part reads " · signed off by", not " · co-signed by" (ADR-0094 (l)).
pub(crate) const REVIEWED: &str =
    "Your review signature was recorded — the chart shows which parts it covers.";
/// … when the note holds other parts than the window showed: nothing was signed.
const CHANGED: &str = "This note changed since it was shown — read the chart again, then co-sign \
     it. Nothing was signed.";
/// … when the KEY's person supervises no writer whose part here awaits a review (they reviewed it
/// already, in another window; or the strip released someone else's key than who's here names).
const NOTHING_TO_REVIEW: &str =
    "There is nothing here for you to co-sign as reviewed. Nothing was signed.";
/// What a failed review leads with, before the node's own sentence.
const REVIEW_FAILED: &str = "Could not co-sign this note: ";

/// Co-sign the compound note whose root is `root_event_id` on the chart on screen as reviewed.
/// `shown_parts` (JS `shownParts`): the note's part ids exactly as the pane showed them.
#[tauri::command]
pub async fn note_review_cosign(
    state: tauri::State<'_, AppState>,
    patient_id: String,
    root_event_id: String,
    shown_parts: Vec<String>,
    credential: Option<Credential>,
) -> Result<NoteSignReport, ErrorView> {
    note_review_cosign_impl(
        &state,
        &patient_id,
        &root_event_id,
        &shown_parts,
        credential,
    )
    .await
}

/// A review's refusal as the webview reads it: both mean the pane is behind the record (or the
/// key is not a supervisor's here), so both ask for a re-read; nothing was written. Pure.
pub(crate) fn review_refusal_view(refusal: SignOffRefusal) -> ErrorView {
    match refusal {
        SignOffRefusal::Changed => stale(CHANGED),
        SignOffRefusal::NothingToSign => stale(NOTHING_TO_REVIEW),
    }
}

/// A review the node failed (nothing written): the node's sentence after [`REVIEW_FAILED`], with
/// `node_failure`'s retry. Pure.
pub(crate) fn review_failure(e: &anyhow::Error) -> ErrorView {
    let view = node_failure(e);
    ErrorView {
        text: format!("{REVIEW_FAILED}{}", view.text),
        ..view
    }
}

/// What a review whose commit got no ordinary answer says (#814), `sign_off_unknown_words`'
/// shape in a co-signature's words: `event` the first vouch's id, `cause` the failure. Pure.
pub(crate) fn review_unknown_words(event: Uuid, cause: &str) -> String {
    format!(
        "Whether this note was co-signed is NOT known — no ordinary answer came back while \
         co-signing it (event {event}: {cause}). Once the window can reach the node again, the \
         chart shows whether it was co-signed."
    )
}

/// Co-sign the note as reviewed (see the module doc for the order). ONE node call, so one
/// outcome: co-signed, refused (nothing written), NOT known (#814), or a failure in the node's
/// own words, led by [`REVIEW_FAILED`].
pub async fn note_review_cosign_impl(
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
        review_note(&mut db, &input, &author)
            .await
            .map_err(|e| db.explain(e))
    };
    let (text, behind_the_record) = match result {
        Ok(Ok(_vouches)) => (REVIEWED.to_string(), None),
        Ok(Err(refusal)) => return Err(review_refusal_view(refusal)),
        Err(e) => match cairn_node::db_diagnosis::commit_outcome_unknown_event(&e) {
            Some(event) => (
                review_unknown_words(event, &cairn_gui_live::cause_of(&e)),
                Some(Recorded::Unknown),
            ),
            None => return Err(review_failure(&e)),
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
#[path = "notes_review_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "notes_review_db_tests.rs"]
mod db_tests;
