//! The note pane's views, built by pure functions: the signed notes, and each open draft as
//! every writer's part, each under its writer's claimed name, with which part (if any) this
//! terminal's current writer may edit (sign-in slice 3b-1).
//!
//! The rule the floor enforces, mirrored here so the editor opens on the right text: a writer
//! writes only the TAIL part, and only when it is theirs; otherwise their save appends a new
//! part. So only the tail can be `editable`, and only when the draft is held here and the tail's
//! writer is the current claim — a named person by staff id, or "Don't know" on an unknown tail
//! (ruling M3: two unknown writers share it, since the floor cannot tell them apart; the editor
//! must show its text, or an empty editor would REPLACE it on the next save). Pure functions.
use super::time::datetime_local;
use crate::whos_here::{claimed_name, Claim};
use cairn_node::note::draft::{DraftPart, NoteDraft};
use cairn_node::note::read::{NoteRow, PatientNoteList};
use cairn_node::note::view::{byline_words, instant, time_words};
use chrono::{NaiveDate, TimeZone};

/// One signed note, as the pane shows it.
#[derive(Debug, serde::Serialize)]
pub struct NoteRowView {
    pub event_id: String,
    /// The text exactly as its writer typed it. The webview sets it as `textContent`.
    pub text: String,
    /// Who signed it, graded (`view::byline_words`): "signed by …", a claim, or writer not known,
    /// headed "written by ?…" when another writer drafted it.
    pub byline: String,
    /// When it applies (`view::time_words`): a stated time, or when writing began.
    pub when: String,
}

/// What the note pane shows for the chart on screen.
#[derive(Debug, serde::Serialize)]
pub struct NotePane {
    pub notes: Vec<NoteRowView>,
    /// Said when notes on a linked chart are held back because a link is in doubt.
    pub withheld_sentence: Option<String>,
    pub drafts: Vec<DraftView>,
}

/// The read as the pane shows it, in zone `tz` with `today` its date, the editable part chosen for
/// `claim`. Pure.
pub(crate) fn note_pane<Tz: TimeZone>(
    list: PatientNoteList,
    terminal: Option<&str>,
    claim: &Claim,
    tz: &Tz,
    today: NaiveDate,
) -> NotePane
where
    Tz::Offset: std::fmt::Display,
{
    let withheld_sentence = (list.withheld_on_linked > 0).then(|| {
        format!(
            "{} note(s) on this patient's linked charts are not shown: a link in this record is in \
             doubt, so the node cannot yet vouch that they are this patient's. They are shown \
             once a clinician has judged the record's links (\"How these charts are linked\").",
            list.withheld_on_linked
        )
    });
    NotePane {
        notes: list
            .notes
            .iter()
            .map(|n| note_row_view(n, tz, today))
            .collect(),
        withheld_sentence,
        drafts: list
            .drafts
            .iter()
            .map(|d| draft_view(d, terminal, claim, tz, today))
            .collect(),
    }
}

/// One note's words, dated against `today`. Pure.
fn note_row_view<Tz: TimeZone>(note: &NoteRow, tz: &Tz, today: NaiveDate) -> NoteRowView
where
    Tz::Offset: std::fmt::Display,
{
    NoteRowView {
        event_id: note.event_id.to_string(),
        text: note.text.clone(),
        byline: byline_words(&note.grade, note.drafted_by.as_deref()),
        when: time_words(
            &note.effective_basis,
            note.t_effective,
            note.recording_started,
            tz,
            today,
        ),
    }
}

/// One writer's part of an open draft.
#[derive(Debug, serde::Serialize)]
pub struct PartView {
    /// "?Nurse A", or "writer not known" (`whos_here::claimed_name`).
    pub writer_shown: String,
    /// "unverified Nurse A": the accessible name, so the `?` is heard, not just seen.
    pub writer_spoken: String,
    /// The RAW text as typed; the webview sets it as `textContent`.
    pub text: String,
    /// The writer's stated time as the `datetime-local` field holds it (`2026-11-03T03:00`, this
    /// machine's zone; with seconds when it has them), `None` when none was stated.
    pub stated_time: Option<String>,
    /// "started 03:12 · last typed 03:40".
    pub words: String,
    /// Whether the editor opens on this part (see the module doc).
    pub editable: bool,
}

/// One open draft on the chart.
#[derive(Debug, serde::Serialize)]
pub struct DraftView {
    pub draft_id: String,
    /// Whether the draft is held by THIS terminal — the only place it may be edited.
    pub here: bool,
    /// "unsigned draft · started 03:12 · last typed 03:40", plus where it is being written.
    pub words: String,
    /// Its parts in writing order.
    pub parts: Vec<PartView>,
}

/// Is `part` the current claim's? A named writer by staff id (both normalised by the vault), or an
/// unknown writer for "Don't know". Nobody asked owns no part.
fn is_claims(part: &DraftPart, claim: &Claim) -> bool {
    match claim {
        Claim::NotAsked => false,
        Claim::Unknown => part.writer.is_none(),
        Claim::Named { staff_id, .. } => part.writer.as_deref() == Some(staff_id.as_str()),
    }
}

/// One draft's words, its parts, and which part the current claim may edit.
pub(crate) fn draft_view<Tz: TimeZone>(
    draft: &NoteDraft,
    terminal: Option<&str>,
    claim: &Claim,
    tz: &Tz,
    today: NaiveDate,
) -> DraftView
where
    Tz::Offset: std::fmt::Display,
{
    // `None`: this terminal's id is not read yet, so no draft can be known to be this one's.
    let here = terminal == Some(draft.terminal_id.as_str());
    let mut words = format!(
        "unsigned draft · started {} · last typed {}",
        instant(draft.recording_started, tz, today),
        instant(draft.last_keystroke, tz, today)
    );
    if !here {
        words.push_str(" · being written at another terminal");
    }
    let tail = draft.parts.len().saturating_sub(1);
    let parts = draft
        .parts
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let name = claimed_name(p.writer_name.as_deref());
            PartView {
                writer_shown: name.shown,
                writer_spoken: name.spoken,
                text: p.narrative.clone(),
                stated_time: p.stated_time.map(|t| datetime_local(t, tz)),
                words: format!(
                    "started {} · last typed {}",
                    instant(p.recording_started, tz, today),
                    instant(p.last_keystroke, tz, today)
                ),
                editable: here && i == tail && is_claims(p, claim),
            }
        })
        .collect();
    DraftView {
        draft_id: draft.draft_id.to_string(),
        here,
        words,
        parts,
    }
}

/// Whether any part the on-screen text does NOT replace holds text: every part, except the tail
/// when it is `writer`'s own (the editor rewrites that one). Lets a Sign with a blank editor under
/// a written part ask for the key: the node signs it, skipping blank parts.
pub(crate) fn others_have_text(parts: &[DraftPart], writer: Option<&str>) -> bool {
    let replaced = parts
        .last()
        .filter(|tail| tail.writer.as_deref() == writer)
        .map(|tail| tail.seq);
    parts
        .iter()
        .any(|p| Some(p.seq) != replaced && !p.narrative.trim().is_empty())
}

#[cfg(test)]
#[path = "notes_parts_tests.rs"]
mod tests;
