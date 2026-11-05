//! A note and all its later parts as the pane shows them: ONE note (sign-in slice 3c-1).
//!
//! A multi-writer draft signs as one event per writer's part, and a clinician can add to a note
//! already on the chart ("Add to this note"); on paper, all of that is one page someone wrote
//! more on. The node groups the events (`cairn_node::note::compound::compounds`); this module
//! words a group for the window: the parts' texts run together as one text, under the root's
//! heading, with a "{n} parts" disclosure holding each part's own heading when there is more
//! than one. Pure functions, so every sentence is pinned without a database or a webview.
use super::parts::{note_row_view, NoteRowView};
use cairn_node::note::compound::{compound_state, state_words, CompoundNote};
use chrono::{NaiveDate, TimeZone};
use uuid::Uuid;

/// Heads a compound whose root adds to a note this read does not hold (withheld on a doubted
/// linked chart, or kept off the screen): the clinician must not read it as a note of its own.
pub(crate) const ORPHAN_LEAD: &str = "Adds to a note not shown here · ";

/// One compound note, as the pane shows it. The `text`/`byline`/`byline_spoken`/`when` names are
/// a one-part note's own (`NoteRowView`), so a webview that only knows notes still shows it.
#[derive(Debug, serde::Serialize)]
pub struct CompoundView {
    /// The root note's event id — what an addendum draft names as its target.
    pub root_event_id: String,
    /// Every part's text, in part order, one blank line between: the note as it reads.
    pub text: String,
    /// The root's byline (`view::byline_words`), led by [`ORPHAN_LEAD`] for an orphan.
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
    /// Whether "Add to this note" is offered: the root is on the chart on screen. A linked chart's
    /// note is not — the draft would live on a chart this pane does not list.
    pub can_add: bool,
}

/// Compound `c` worded for the pane, `opened` being the chart on screen, dated against `today`
/// in zone `tz`. Pure.
pub(crate) fn compound_view<Tz: TimeZone>(
    c: &CompoundNote,
    opened: Uuid,
    tz: &Tz,
    today: NaiveDate,
) -> CompoundView
where
    Tz::Offset: std::fmt::Display,
{
    let views: Vec<NoteRowView> = c
        .parts
        .iter()
        .map(|p| note_row_view(p, tz, today))
        .collect();
    let n = views.len();
    let lead = if c.orphan { ORPHAN_LEAD } else { "" };
    // `compounds` never makes an empty compound: `views[0]` is the root.
    let byline = format!("{lead}{}", views[0].byline);
    let byline_spoken = format!("{lead}{}", views[0].byline_spoken);
    let when = views[0].when.clone();
    let text = c
        .parts
        .iter()
        .map(|p| p.text.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    CompoundView {
        root_event_id: c.root.to_string(),
        text,
        byline,
        byline_spoken,
        when,
        state: state_words(compound_state(&c.parts)).map(str::to_string),
        parts_label: (n >= 2).then(|| format!("{n} parts")),
        parts: if n >= 2 { views } else { vec![] },
        can_add: c.chart == opened,
    }
}

#[cfg(test)]
#[path = "notes_compound_tests.rs"]
mod tests;
