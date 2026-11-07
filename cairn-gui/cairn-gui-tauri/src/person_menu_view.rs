//! Every sentence the person menu says, as pure functions (sign-in slice 3c-3c).
//!
//! The menu is the answer to "what is waiting for me?": notes whose signature the named person
//! owes, notes they must co-sign, and their supervisees' recent notes. The node classifies the
//! rows (`cairn_node::note::menu`); this module only WORDS them, so each sentence is pinned in
//! `person_menu_tests.rs` with no database and no webview. The JS draws these strings verbatim.
use crate::notes::commit::due_words;
use cairn_node::note::menu::{MenuItem, PersonMenu, Why, REVIEW_DAYS};
use cairn_node::note::menu_read::MenuRead;
use cairn_node::note::view::instant;
use cairn_patient_search::Candidate;
use chrono::{DateTime, TimeZone, Utc};
use std::collections::HashMap;
use uuid::Uuid;

pub const OWN_HEADING: &str = "Your signature";
pub const COSIGN_HEADING: &str = "Needs your co-signature";
pub const EMPTY_GROUP: &str = "Nothing here.";
pub const PROGRESS_NOTE: &str = "Progress note";
pub const DRAFT_NOTE: &str = "Draft note";
pub const UNSIGNED: &str = "Went in unsigned — sign it";
pub const REPUDIATED: &str = "Counts as unsigned — check it is yours, then sign or correct it";
pub const OPEN_NOTE: &str = "Open note";
/// "Open note" pressed on a row the re-read list no longer holds.
pub const NO_LONGER_WAITS: &str =
    "That note no longer waits in your list — the list was read again.";
/// A writer (or supervisor) whose `display_name` the node does not hold.
pub const UNNAMED_WRITER: &str = "a writer this node cannot name";
/// A row's chart that the display read could not name (should not happen: the read never drops
/// an id, it names it "(registration not yet received here)").
pub const UNNAMED_PATIENT: &str = "a chart this node cannot name";
/// The picker could not read the counts (shown in `#whos-here-status`).
pub const COUNTS_FAILED: &str = "Waiting counts could not be read.";

/// The review group's heading, naming the window it looks back over.
pub fn review_heading() -> String {
    format!("Your supervisees' recent notes (last {REVIEW_DAYS} days)")
}

/// One row, as the page draws it.
#[derive(Debug, serde::Serialize)]
pub struct MenuRowView {
    pub patient: String,
    pub what: String,
    pub when: String,
    pub state: String,
    /// The button's visible words, "Open note".
    pub open_words: String,
    /// The button's accessible name, "Open note: {patient}, {when}".
    pub open_label: String,
    pub patient_id: String,
    /// `"part:<id>"` or `"draft:<id>"`: what the opened chart should scroll to.
    pub focus: String,
}

#[derive(Debug, serde::Serialize)]
pub struct MenuGroupView {
    pub heading: String,
    /// `Some("Nothing here.")` when there are no rows.
    pub empty: Option<String>,
    pub rows: Vec<MenuRowView>,
}

#[derive(Debug, serde::Serialize)]
#[serde(tag = "kind")]
#[allow(clippy::large_enum_variant)] // built once per read, serialised at once
pub enum PersonMenuView {
    /// Nobody is named in who's here: the page hides the menu.
    Nobody,
    Menu {
        count: usize,
        /// The bar's text, "Waiting for you: 3".
        count_words: String,
        /// The bar's accessible name.
        count_label: String,
        own: MenuGroupView,
        cosign: MenuGroupView,
        review: MenuGroupView,
    },
}

/// One picker entry's count: `suffix` is the words appended to the person's entry.
#[derive(Debug, serde::Serialize)]
pub struct StaffCountView {
    pub staff_id: String,
    pub count: usize,
    pub suffix: String,
}

/// The error the picker's counts answer when their read fails: the clinician-facing sentence is
/// [`COUNTS_FAILED`]; whether a retry can help (`retry`) is kept from the read's own error. The
/// explained cause is logged by the caller, as no `ErrorView` has a slot for it. Pure.
pub fn counts_failed_view(cause: crate::funnel::view::ErrorView) -> crate::funnel::view::ErrorView {
    crate::funnel::view::ErrorView {
        text: COUNTS_FAILED.into(),
        retry: cause.retry,
    }
}

/// The picker's counts, worded: `" · 3 waiting"` for a positive count, nothing for zero.
pub fn counts_view(counts: Vec<(String, usize)>) -> Vec<StaffCountView> {
    counts
        .into_iter()
        .map(|(staff_id, count)| StaffCountView {
            staff_id,
            count,
            suffix: if count > 0 {
                format!(" · {count} waiting")
            } else {
                String::new()
            },
        })
        .collect()
}

/// "A", "A or B", "A, B or C".
fn join_or(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} or {last}", init.join(", ")),
    }
}

/// `actor`'s display name, or [`UNNAMED_WRITER`].
fn name(read: &MenuRead, actor: &str) -> String {
    read.name_of(actor).unwrap_or(UNNAMED_WRITER).to_string()
}

/// The row's state sentence.
fn state_words<Tz: TimeZone>(
    item: &MenuItem,
    read: &MenuRead,
    tz: &Tz,
    now: DateTime<Utc>,
) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let writer = || {
        item.writer
            .as_deref()
            .map_or(UNNAMED_WRITER.to_string(), |a| name(read, a))
    };
    match &item.why {
        Why::Draft { due, stuck, .. } => {
            let today = now.with_timezone(tz).date_naive();
            // The menu has no node-clock `overdue` or window-side `failed` (see the brief).
            due_words(*due, *due <= now, *stuck, false, tz, today)
        }
        Why::WentInUnsigned => UNSIGNED.into(),
        Why::KeyRepudiated => REPUDIATED.into(),
        Why::NeedsCoSign { others } => {
            let mut names: Vec<String> = others.iter().map(|a| name(read, a)).collect();
            names.sort();
            let also = if names.is_empty() {
                String::new()
            } else {
                format!(" (you or {})", join_or(&names))
            };
            format!("By {} — needs your co-signature{also}", writer())
        }
        Why::Review => format!("By {} — not yet reviewed", writer()),
    }
}

/// One menu item as a row: the patient's name from `patients`, the item's time, what kind of note
/// it is, the state sentence, and the button's label and focus target. Pure.
fn row_view<Tz: TimeZone>(
    item: &MenuItem,
    read: &MenuRead,
    patients: &HashMap<Uuid, Candidate>,
    tz: &Tz,
    now: DateTime<Utc>,
) -> MenuRowView
where
    Tz::Offset: std::fmt::Display,
{
    let today = now.with_timezone(tz).date_naive();
    let patient = patients
        .get(&item.chart)
        .map_or(UNNAMED_PATIENT, |c| c.display_name.as_str())
        .to_string();
    let when = instant(item.when, tz, today);
    let (what, focus) = match &item.why {
        Why::Draft { draft_id, .. } => (DRAFT_NOTE, format!("draft:{draft_id}")),
        _ => (
            PROGRESS_NOTE,
            item.part.map_or(String::new(), |p| format!("part:{p}")),
        ),
    };
    MenuRowView {
        open_words: OPEN_NOTE.into(),
        open_label: format!("{OPEN_NOTE}: {patient}, {when}"),
        patient,
        what: what.into(),
        state: state_words(item, read, tz, now),
        when,
        patient_id: item.chart.to_string(),
        focus,
    }
}

/// One group: its heading, the "Nothing here." line when it has no items, and a row per item. Pure.
fn group_view<Tz: TimeZone>(
    heading: String,
    items: &[MenuItem],
    read: &MenuRead,
    patients: &HashMap<Uuid, Candidate>,
    tz: &Tz,
    now: DateTime<Utc>,
) -> MenuGroupView
where
    Tz::Offset: std::fmt::Display,
{
    MenuGroupView {
        heading,
        empty: items.is_empty().then(|| EMPTY_GROUP.to_string()),
        rows: items
            .iter()
            .map(|i| row_view(i, read, patients, tz, now))
            .collect(),
    }
}

/// The whole menu, worded. `patients` holds a candidate per chart the rows name. Pure.
pub fn menu_view<Tz: TimeZone>(
    menu: &PersonMenu,
    read: &MenuRead,
    patients: &HashMap<Uuid, Candidate>,
    tz: &Tz,
    now: DateTime<Utc>,
) -> PersonMenuView
where
    Tz::Offset: std::fmt::Display,
{
    let count = menu.count();
    let g = |h: &str, i: &[MenuItem]| group_view(h.to_string(), i, read, patients, tz, now);
    PersonMenuView::Menu {
        count,
        count_words: format!("Waiting for you: {count}"),
        count_label: format!("Waiting for you: {count} — open the list"),
        own: g(OWN_HEADING, &menu.own),
        cosign: g(COSIGN_HEADING, &menu.cosign),
        review: group_view(review_heading(), &menu.review, read, patients, tz, now),
    }
}
