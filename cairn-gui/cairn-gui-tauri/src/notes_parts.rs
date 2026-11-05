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
use super::commit::due_words;
use super::compound::{compound_view, CompoundView};
use super::time::datetime_local;
use crate::whos_here::{claim_view, claimed_name, Claim, ClaimView};
use cairn_node::note::compound::compounds;
use cairn_node::note::draft::{DraftPart, NoteDraft};
use cairn_node::note::read::{DiscardMarker, NoteRow, PatientNoteList};
use cairn_node::note::view::{byline_words, instant, time_words};
use chrono::{NaiveDate, TimeZone};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// One signed note, as the pane shows it.
#[derive(Debug, serde::Serialize)]
pub struct NoteRowView {
    pub event_id: String,
    /// The text exactly as its writer typed it. The webview sets it as `textContent`.
    pub text: String,
    /// Who signed it, graded (`view::byline_words`): "signed by …", a claim, or writer not known,
    /// headed by who typed it when that is not the signer ("written by ?Nurse A", "writer not
    /// known", …).
    pub byline: String,
    /// The byline as a screen reader should say it: "written by unverified Nurse A · …" (#973).
    pub byline_spoken: String,
    /// When it applies (`view::time_words`): a stated time, or when writing began.
    pub when: String,
}

/// What the note pane shows for the chart on screen.
#[derive(Debug, serde::Serialize)]
pub struct NotePane {
    /// The notes, each with all its later parts as ONE note (`notes_compound.rs`), newest first.
    pub notes: Vec<CompoundView>,
    /// Said when notes on a linked chart are held back because a link is in doubt.
    pub withheld_sentence: Option<String>,
    pub drafts: Vec<DraftView>,
    /// Discarded drafts, newest first: a marker BESIDE the notes, never among them (slice 3b-2b).
    pub discards: Vec<DiscardView>,
    /// The claim the `editable` flags were chosen for: the window's editor captures ITS staff id
    /// as the writer, so the part it opens on and the writer it saves as cannot diverge.
    pub claim: ClaimView,
}

/// The read as the pane shows it, in zone `tz` with `today` its date, the editable part chosen for
/// `claim`; `opened`: the chart on screen (only its notes can be added to); `failed`: the drafts
/// whose last commit in this window failed (`commit::due_words`). Pure.
pub(crate) fn note_pane<Tz: TimeZone>(
    list: PatientNoteList,
    opened: Uuid,
    terminal: Option<&str>,
    claim: &Claim,
    failed: &HashSet<Uuid>,
    tz: &Tz,
    today: NaiveDate,
) -> NotePane
where
    Tz::Offset: std::fmt::Display,
{
    let withheld_sentence =
        withheld_words(list.withheld_on_linked, list.discards_withheld_on_linked);
    let grouped = compounds(list.notes);
    let notes: Vec<CompoundView> = grouped
        .iter()
        .map(|c| compound_view(c, opened, tz, today))
        .collect();
    // Every shown part's id → its compound's time: what an addendum draft says it adds to.
    let shown: HashMap<Uuid, String> = grouped
        .iter()
        .zip(&notes)
        .flat_map(|(c, v)| c.parts.iter().map(|p| (p.event_id, v.when.clone())))
        .collect();
    let drafts = list
        .drafts
        .iter()
        .map(|d| {
            let failed = failed.contains(&d.draft_id);
            draft_view(d, terminal, claim, failed, tz, today, &shown)
        })
        .collect();
    NotePane {
        notes,
        withheld_sentence,
        drafts,
        discards: list
            .discards
            .iter()
            .map(|m| discard_view(m, tz, today))
            .collect(),
        claim: claim_view(claim),
    }
}

/// What the pane says when charts of the set are held back because a link is in doubt: `notes`
/// held-back notes and `discards` held-back discard markers. `None` when nothing is held back.
/// The notes sentence is unchanged when no marker is held back; markers alone get the same
/// sentence led by their own count. Pure.
fn withheld_words(notes: usize, discards: usize) -> Option<String> {
    if notes == 0 && discards == 0 {
        return None;
    }
    let lead = if notes > 0 {
        format!("{notes} note(s)")
    } else {
        format!("{discards} discarded draft(s)")
    };
    let mut words = format!(
        "{lead} on this patient's linked charts are not shown: a link in this record is in \
         doubt, so the node cannot yet vouch that they are this patient's. They are shown \
         once a clinician has judged the record's links (\"How these charts are linked\")."
    );
    if notes > 0 && discards > 0 {
        words.push_str(&format!(
            " {discards} discarded draft(s) on those charts are held back for the same reason."
        ));
    }
    Some(words)
}

/// One discarded draft, as the pane lists it: a one-line summary and the text it held.
#[derive(Debug, serde::Serialize)]
pub struct DiscardView {
    pub event_id: String,
    /// "Draft discarded by Nurse A · 3 Nov 03:40".
    pub summary: String,
    pub parts: Vec<DiscardPartView>,
}

/// One part of a discarded draft.
#[derive(Debug, serde::Serialize)]
pub struct DiscardPartView {
    /// "Part 1, written by Nurse A · started 3 Nov 03:12", or "Part 2, writer not known · …".
    pub heading: String,
    /// The text exactly as typed; the webview sets it as `textContent`.
    pub text: String,
}

/// The name a marker uses for a discarder the node cannot name.
const UNNAMED: &str = "someone this node cannot name";

/// One marker's words, dated against `today` in zone `tz`. A part the discarder wrote is
/// headed with the discarder's name (the rule lets only the discarder's own parts be discarded,
/// so "written by" is the discarder); any other part says its writer is not known. Pure.
pub(crate) fn discard_view<Tz: TimeZone>(
    m: &DiscardMarker,
    tz: &Tz,
    today: NaiveDate,
) -> DiscardView
where
    Tz::Offset: std::fmt::Display,
{
    let name = m.discarder.as_deref().unwrap_or(UNNAMED);
    DiscardView {
        event_id: m.event_id.to_string(),
        summary: format!(
            "Draft discarded by {name} · {}",
            instant(m.discarded_at, tz, today)
        ),
        parts: m
            .parts
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let writer = if p.by_discarder {
                    format!("written by {name}")
                } else {
                    "writer not known".to_string()
                };
                DiscardPartView {
                    heading: format!(
                        "Part {}, {writer} · started {}",
                        i + 1,
                        instant(p.recording_started, tz, today)
                    ),
                    text: p.text.clone(),
                }
            })
            .collect(),
    }
}

/// One note's words, dated against `today`. Pure.
pub(crate) fn note_row_view<Tz: TimeZone>(note: &NoteRow, tz: &Tz, today: NaiveDate) -> NoteRowView
where
    Tz::Offset: std::fmt::Display,
{
    let byline = byline_words(&note.grade, &note.writer);
    NoteRowView {
        event_id: note.event_id.to_string(),
        text: note.text.clone(),
        byline: byline.shown,
        byline_spoken: byline.spoken,
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
    /// "unsigned draft · started 03:12 · last typed 03:40", plus where it is being written, and
    /// when it goes into the record unsigned (`commit::due_words`).
    pub words: String,
    /// Its parts in writing order.
    pub parts: Vec<PartView>,
    /// The note this draft adds to (its root's event id), exactly as the node holds it; `None`
    /// for a new note. The webview sends it back with every save, Sign and Discard.
    pub addendum_to: Option<String>,
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

/// The lead of an addendum draft's line, from `shown` (every part the pane shows → its note's
/// time): "Adding to the note of 3 Nov 03:00 · ", or "Adding to a note not shown here · " when the
/// note it adds to is not on screen (withheld on a doubted link). Empty for a new note's draft.
fn adding_to_words(addendum_to: Option<Uuid>, shown: &HashMap<Uuid, String>) -> String {
    match addendum_to {
        None => String::new(),
        Some(target) => match shown.get(&target) {
            Some(when) => format!("Adding to the note of {when} · "),
            None => "Adding to a note not shown here · ".to_string(),
        },
    }
}

/// One draft's words, its parts, and which part the current claim may edit. `failed`: this
/// window's last commit of it failed; `shown`: every note part the pane shows → its note's time
/// (`adding_to_words`).
pub(crate) fn draft_view<Tz: TimeZone>(
    draft: &NoteDraft,
    terminal: Option<&str>,
    claim: &Claim,
    failed: bool,
    tz: &Tz,
    today: NaiveDate,
    shown: &HashMap<Uuid, String>,
) -> DraftView
where
    Tz::Offset: std::fmt::Display,
{
    // `None`: this terminal's id is not read yet, so no draft can be known to be this one's.
    let here = terminal == Some(draft.terminal_id.as_str());
    let mut words = format!(
        "{}unsigned draft · started {} · last typed {}",
        adding_to_words(draft.addendum_to, shown),
        instant(draft.recording_started, tz, today),
        instant(draft.last_keystroke, tz, today)
    );
    if !here {
        words.push_str(" · being written at another terminal");
    }
    words.push_str(" · ");
    words.push_str(&due_words(
        draft.due,
        draft.overdue,
        draft.stuck,
        failed,
        tz,
        today,
    ));
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
        addendum_to: draft.addendum_to.map(|id| id.to_string()),
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
