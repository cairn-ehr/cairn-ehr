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
use super::{parse_target, terminal_of, NoteSignReport, FIXTURE};
use crate::commands::record_timing;
use crate::funnel::view::{ErrorView, Recorded, Retry};
use crate::link::judgement_view::{refused, stale};
use crate::signer::{Credential, KeyRefusal};
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
/// … when the draft was an addendum the node never saved: its note is not shown on this chart,
/// so no part reached the node and there was nothing to keep (`DiscardOutcome::NeverSaved`).
pub(crate) const NEVER_SAVED_DISCARDED: &str = "Discarded. The note this adds to is not shown on \
     this chart, so nothing of this draft had been saved, and nothing was kept.";
/// … when a colleague wrote part of the draft: only its writer may throw their part away.
const NOT_YOURS: &str = "Not discarded: someone else wrote part of this draft, and only its \
     writer may throw their part away — nothing was thrown away; your text is still here.";
/// … when the draft is another chart's.
const ELSEWHERE: &str = "Not discarded: this draft belongs to another chart — nothing was thrown \
     away; your text is still here.";
/// … when another terminal holds the draft now.
const MOVED: &str = "Not discarded: this draft is now being continued at another terminal — \
     nothing was thrown away; your text is still here.";
/// … when the on-screen writer is not an enrolled member (the save of the text on screen refused
/// it). A part above whose writer has no key is `NOT_YOURS`: db/069 compares by key.
const NO_SUCH_WRITER: &str =
    "Not discarded: the name at this terminal is not an enrolled member — \
     say who's here again; your text is still here.";
/// … when the quarantine timer already put the draft into the record unsigned.
const COMMITTED: &str = "Not discarded: this note had already gone into the record unsigned.";
/// … when this staff ID's role may not act clinically.
const NO_SUCH_SIGNER: &str = "Not discarded: this staff ID may not discard a clinical draft.";
/// The lead of a refusal, or of any failure that wrote nothing (`led_failure`).
const NOT_DISCARDED: &str = "Not discarded: ";
/// The lead of a failure whose commit may have landed: the discard may or may not be recorded.
const DISCARD_UNKNOWN: &str = "Whether this draft was discarded is NOT known: ";

/// Discard the draft on screen (the webview sends the same object as for Sign, `addendumTo`
/// included: a Discard inside the debounce saves the text as the addendum it is first).
#[allow(clippy::too_many_arguments)] // the Sign object's fields, one argument each, as Tauri maps them
#[tauri::command]
pub async fn note_discard(
    state: tauri::State<'_, AppState>,
    patient_id: String,
    draft_id: String,
    narrative: String,
    stated_time: Option<String>,
    writer: Option<String>,
    credential: Option<Credential>,
    addendum_to: Option<String>,
) -> Result<NoteSignReport, ErrorView> {
    note_discard_impl(
        &state,
        &patient_id,
        &draft_id,
        &narrative,
        stated_time.as_deref(),
        writer.as_deref(),
        credential,
        addendum_to.as_deref(),
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

/// A key refusal worded for a discard (PR #984 review): `KeyRefusal::sentence` is Sign's ("…to
/// sign", "Nothing was signed"), and a clinician throwing a draft away must not read that the
/// press signs it. Only the sentences that name signing are reworded; "Nothing was …" is dropped,
/// because `led_failure` already leads with "Not discarded". Every variant is spelled out (#412),
/// so a new one must be worded here too. The retry is the key's own (`key_locked`).
pub(crate) fn discard_key_refused(refusal: &KeyRefusal) -> ErrorView {
    let text = match refusal {
        KeyRefusal::NeedCredential => {
            "Enter your staff ID and passphrase to discard this draft.".to_string()
        }
        KeyRefusal::NotForClinicalActs => {
            "A superuser cannot discard a clinical draft. Enter your own staff ID.".to_string()
        }
        KeyRefusal::NotCurrent => "Your key is no longer enrolled here. Ask a superuser or an \
             enroller to re-key you."
            .to_string(),
        KeyRefusal::Expired => "No superuser has confirmed your key, and this site stops \
             unconfirmed keys after a set time. Ask a superuser to confirm your key."
            .to_string(),
        KeyRefusal::OverPowered => "This workstation is connected to the node with more access \
             than a window may have, so it cannot discard. Ask your IT support to run cairn-node \
             provision-window-role and start this window with its login."
            .to_string(),
        KeyRefusal::Fault => "Your passphrase could not be checked. Try again, and if it fails \
             again, ask your IT support."
            .to_string(),
        KeyRefusal::Wrong { .. }
        | KeyRefusal::Replaced { .. }
        | KeyRefusal::Waiting { .. }
        | KeyRefusal::Unknown => refusal.sentence(),
    };
    ErrorView {
        text,
        retry: key_locked(refusal).retry,
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

/// A failure that is not one of the node's worded refusals — an outage, a node that cannot write,
/// a verdict before the key — led by what it means for the discard, chosen by its retry class: a
/// commit that may have landed says the discard is NOT known; anything else, "Not discarded".
/// The window shows Rust's sentence as it is (notes_discard.js adds no lead), so without this a
/// bare "connection reset" would not say whether the draft is gone. A text already led is left
/// alone, so the lead is never doubled. Pure.
pub(crate) fn led_failure(failure: ErrorView) -> ErrorView {
    if failure.text.starts_with(NOT_DISCARDED) || failure.text.starts_with(DISCARD_UNKNOWN) {
        return failure;
    }
    let lead = match failure.retry {
        Retry::AfterReread(Recorded::Unknown) => DISCARD_UNKNOWN,
        Retry::Now | Retry::AfterOperator | Retry::Never | Retry::AfterReread(Recorded::No) => {
            NOT_DISCARDED
        }
    };
    ErrorView {
        text: format!("{lead}{}", failure.text),
        retry: failure.retry,
    }
}

/// Discard draft `draft_id` of the chart on screen, carrying the text on screen into the audit
/// copy (as Sign carries it). See the module doc; the steps are `note_sign_impl`'s. The node's
/// own refusals are worded whole (`discard_refusal_view`); every other failure is led by what it
/// means for the discard (`led_failure`).
#[allow(clippy::too_many_arguments)] // the Sign object's fields, one argument each, as Tauri maps them
pub async fn note_discard_impl(
    state: &AppState,
    patient_id: &str,
    draft_id: &str,
    narrative: &str,
    stated_time: Option<&str>,
    writer: Option<&str>,
    credential: Option<Credential>,
    addendum_to: Option<&str>,
) -> Result<NoteSignReport, ErrorView> {
    let attempt = discard_attempt(
        state,
        patient_id,
        draft_id,
        narrative,
        stated_time,
        writer,
        credential,
        addendum_to,
    );
    match attempt.await {
        Ok(Ok(report)) => Ok(report),
        Ok(Err(refusal)) => Err(discard_refusal_view(refusal)),
        Err(failure) => Err(led_failure(failure)),
    }
}

/// `note_discard_impl`'s steps. The node's refusal comes back apart (`Ok(Err(refusal))`) from
/// every other failure (`Err`), so the caller words each its own way.
#[allow(clippy::too_many_arguments)] // the Sign object's fields, one argument each, as Tauri maps them
async fn discard_attempt(
    state: &AppState,
    patient_id: &str,
    draft_id: &str,
    narrative: &str,
    stated_time: Option<&str>,
    writer: Option<&str>,
    credential: Option<Credential>,
    addendum_to: Option<&str>,
) -> Result<Result<NoteSignReport, DiscardRefusal>, ErrorView> {
    let patient = state.displayed_patient(patient_id).await.map_err(refused)?;
    if state.is_mock() {
        return Err(refused(FIXTURE));
    }
    let (draft, stated) = discard_checks_before_the_key(draft_id, stated_time)?;
    let target = parse_target(addendum_to)?;
    let terminal = terminal_of(state)?;
    let (human_sk, human_kid) = state
        .signer_for_act(credential, Now::read())
        .await
        .map_err(|r| discard_key_refused(&r))?;
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
            addendum_to: target,
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
        Ok(Ok(DiscardOutcome::NeverSaved)) => (NEVER_SAVED_DISCARDED.to_string(), None),
        Ok(Err(refusal)) => return Ok(Err(refusal)),
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
    Ok(Ok(NoteSignReport {
        text,
        behind_the_record,
    }))
}

#[cfg(test)]
#[path = "notes_discard_tests.rs"]
mod tests;
