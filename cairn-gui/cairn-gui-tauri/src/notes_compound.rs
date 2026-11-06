//! A note and all its later parts as the pane shows them: ONE note (sign-in slice 3c-1).
//!
//! A multi-writer draft signs as one event per writer's part, and a clinician can add to a note
//! already on the chart ("Add to this note"); on paper, all of that is one page someone wrote
//! more on. The node groups the events (`cairn_node::note::compound::compounds`); this module
//! words a group for the window: the parts' texts run together as one text, headed by the root's
//! byline, each later part preceded by a seam line saying who added it and when it applies, with a
//! "{n} parts" disclosure holding each part's own heading when there is more than one. Pure
//! functions, so every sentence is pinned without a database or a webview.
use super::parts::{note_row_view, NoteRowView};
use cairn_node::note::compound::{compound_state, state_words, CompoundNote};
use cairn_node::note::read::NoteRow;
use cairn_node::note::view::instant;
use cairn_node::note::vouch::{addendum_covers, needs_sign_off};
use chrono::{NaiveDate, TimeZone};
use uuid::Uuid;

/// Heads a compound whose root adds to a note this read does not hold (any of
/// `CompoundNote::orphan`'s causes): the clinician must not read it as a note of its own.
pub(crate) const ORPHAN_LEAD: &str = "Adds to a note not shown here · ";

/// One compound note, as the pane shows it. The `text`/`byline`/`byline_spoken`/`when` names are
/// a one-part note's own (`NoteRowView`), so a webview that only knows notes still shows it.
// `Default` only so a drift guard's literal may leave the fields it does not read.
#[derive(Debug, Default, serde::Serialize)]
pub struct CompoundView {
    /// The root note's event id — what an addendum draft names as its target.
    pub root_event_id: String,
    /// Every part's text, in part order, one blank line between: the note as it reads. The window
    /// draws a one-part note from it (`noteItem`); a longer one from `segments`, which add the
    /// seams. Kept whole for any reader that wants the note's text in one piece.
    pub text: String,
    /// The root's byline (`view::part_byline`), led by [`ORPHAN_LEAD`] for an orphan.
    pub byline: String,
    /// The byline as a screen reader should say it, led the same way.
    pub byline_spoken: String,
    /// When the root applies (`view::time_words`).
    pub when: String,
    /// What the parts' signatures say together — "every part signed", "includes an unsigned
    /// part", … (`compound::state_words`); `None` for a one-part note, whose byline says it.
    pub state: Option<String>,
    /// The disclosure's label, "3 parts"; `None` for a one-part note (there is nothing to open).
    pub parts_label: Option<String>,
    /// Each part as its own note, root first; empty for a one-part note.
    pub parts: Vec<NoteRowView>,
    /// The continuous text of a note of two or more parts, one segment per part, root first,
    /// each later one led by its seam; empty for a one-part note, which is drawn from `text`.
    pub segments: Vec<SegmentView>,
    /// Whether "Add to this note" is offered: the root is on the chart on screen. A linked chart's
    /// note is not — the draft would live on a chart this pane does not list.
    pub can_add: bool,
    /// What an addendum to this note says it adds to (`adding_to_shown`): the line "Add to this
    /// note" opens the editor with, the very words the draft's `DraftView::adding_to` says once
    /// the pane is read again.
    pub adds_line: String,
    /// Whether "Sign off this note" is offered: no one has yet signed every part
    /// (`vouch::needs_sign_off`). A linked chart's note too: each vouch files under its part's
    /// own chart, and no draft is involved.
    pub can_sign_off: bool,
    /// Every part's event id, in part order: what "Sign off this note" sends back, so the node
    /// refuses (`Changed`) if the note now holds any other.
    pub shown_parts: Vec<String>,
    /// The line below "Adding to …" when an addendum's signature would also cover the who's-here
    /// claim's own unsigned parts of this note ([`covers_of`]); `None` when there are none. The
    /// draft's `DraftView::covers_line` says the same once the pane is read again.
    pub covers_line: Option<String>,
    /// Those parts' event ids: what Sign sends as `also_vouches`. Empty with no line.
    pub covers: Vec<String>,
}

/// One part's place in a compound's continuous text.
#[derive(Debug, serde::Serialize)]
pub struct SegmentView {
    /// "Added: signed by Dr Bo · 03:10 (stated)" — the part's own byline and time, in its own
    /// heading's words ([`seam_words`]); `None` for the root, which the compound's heading heads.
    pub seam: Option<String>,
    /// The seam as a screen reader should say it ("written by unverified Nurse A", #973).
    pub seam_spoken: Option<String>,
    /// The part's text exactly as typed; the webview sets it as `textContent`.
    pub text: String,
}

/// The seam before a later part: "Added: " and the part's heading as the disclosure shows it
/// (`byline · when`), so a reader of the continuous text sees, at the join, that another writer —
/// or the same one, later — wrote on, and when that part applies. "Added" claims only that the
/// part was added to the note (it is an addendum event), never that it was written later in time:
/// a stated time may precede the root's. Pure.
fn seam_words(byline: &str, when: &str) -> String {
    format!("Added: {byline} · {when}")
}

/// The segments of a compound of `views` (its parts, root first); empty for one part. Pure.
fn segments(views: &[NoteRowView]) -> Vec<SegmentView> {
    if views.len() < 2 {
        return vec![];
    }
    views
        .iter()
        .enumerate()
        .map(|(i, v)| SegmentView {
            seam: (i > 0).then(|| seam_words(&v.byline, &v.when)),
            seam_spoken: (i > 0).then(|| seam_words(&v.byline_spoken, &v.when)),
            text: v.text.clone(),
        })
        .collect()
}

/// What an addendum's signature also covers for the person `claim_actor` (the who's-here claim's
/// actor, `None` with no claim or one this node cannot place): the parts of `c` they drafted and
/// nobody signed (`vouch::addendum_covers`), as the strip's line — each part named by its time,
/// `views` being `c`'s parts worded — and their ids. `(None, [])` when there are none: the line
/// is information shown only when there is something to say, never a question. Pure.
fn covers_of(
    c: &CompoundNote,
    views: &[NoteRowView],
    claim_actor: Option<&str>,
) -> (Option<String>, Vec<String>) {
    let covered = claim_actor.map_or_else(Vec::new, |a| addendum_covers(c.parts(), a));
    if covered.is_empty() {
        return (None, vec![]);
    }
    let times: Vec<&str> = c
        .parts()
        .iter()
        .zip(views)
        .filter(|(p, _)| covered.contains(&p.event_id))
        .map(|(_, v)| v.when.as_str())
        .collect();
    let s = if covered.len() == 1 { "" } else { "s" };
    let line = format!(
        "Your signature also covers your earlier unsigned part{s} of this note: {}",
        times.join("; ")
    );
    (Some(line), covered.iter().map(Uuid::to_string).collect())
}

/// Compound `c` worded for the pane, `opened` being the chart on screen, `claim_actor` the
/// who's-here claim's actor ([`covers_of`]), dated against `today` in zone `tz`. Pure.
pub(crate) fn compound_view<Tz: TimeZone>(
    c: &CompoundNote,
    opened: Uuid,
    claim_actor: Option<&str>,
    tz: &Tz,
    today: NaiveDate,
) -> CompoundView
where
    Tz::Offset: std::fmt::Display,
{
    let views: Vec<NoteRowView> = c
        .parts()
        .iter()
        .map(|p| note_row_view(p, tz, today))
        .collect();
    let n = views.len();
    let lead = if c.is_orphan() { ORPHAN_LEAD } else { "" };
    // `compounds` never makes an empty compound: `views[0]` is the root.
    let byline = format!("{lead}{}", views[0].byline);
    let byline_spoken = format!("{lead}{}", views[0].byline_spoken);
    let when = views[0].when.clone();
    let adds_line = adding_to_shown(&c.parts()[0], &when, tz, today);
    let text = c
        .parts()
        .iter()
        .map(|p| p.text.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    let segments = segments(&views);
    let (covers_line, covers) = covers_of(c, &views, claim_actor);
    CompoundView {
        root_event_id: c.root().to_string(),
        text,
        byline,
        byline_spoken,
        when,
        state: state_words(&compound_state(c.parts())),
        parts_label: (n >= 2).then(|| format!("{n} parts")),
        parts: if n >= 2 { views } else { vec![] },
        segments,
        can_add: c.chart() == opened,
        adds_line,
        can_sign_off: needs_sign_off(c.parts()),
        shown_parts: c.parts().iter().map(|p| p.event_id.to_string()).collect(),
        covers_line,
        covers,
    }
}

/// The "Adding to…" line for a draft adding to the note whose root is `root` and whose words say
/// it applies `when`. A stated time names the note: "Adding to the note of 3 Nov 03:00 (stated)".
/// With none stated, `when` would read "no stated time · writing began …" — run into the draft's
/// own "started …" and alike for every unstated note — so the line names when its writing began
/// instead: "Adding to the note begun 3 Nov 02:00". Pure.
fn adding_to_shown<Tz: TimeZone>(root: &NoteRow, when: &str, tz: &Tz, today: NaiveDate) -> String
where
    Tz::Offset: std::fmt::Display,
{
    if root.effective_basis == "stated" {
        format!("Adding to the note of {when}")
    } else {
        format!(
            "Adding to the note begun {}",
            instant(root.recording_started, tz, today)
        )
    }
}

#[cfg(test)]
#[path = "notes_compound_tests.rs"]
mod tests;
