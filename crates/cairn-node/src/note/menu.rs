//! The person menu's three groups (sign-in slice 3c-3c): what waits for ONE named clinician.
//! Pure: the caller reads the parts, drafts and supervision record in one snapshot and hands
//! them here.
//!
//! Every group is built from the predicates the chart's own byline uses, never from a second
//! reading of the same facts, so the menu and a note can never disagree about what is unsigned:
//!
//! 1. **Your signature** — an open draft of yours; a part that went in unsigned and is still
//!    unsigned (`part_is_signed`, so a colleague's sign-off removes it); a part signed with a
//!    key since repudiated that its real person has not re-signed (`counts_as_unsigned`,
//!    removed only by their own re-sign).
//! 2. **Needs your co-signature** — `cosign::awaits_cosign`, for a writer who has you among
//!    their supervisors NOW. The requirement is judged at the part's time, so lifting it later
//!    does not drop a part signed under it.
//! 3. **Your supervisees' recent notes** — `review::reviewable`, within [`REVIEW_DAYS`].
//!
//! [`PersonMenu::count`] is groups 1 and 2 only. Group 3 is never counted: nothing is owed
//! there, and a number that grows with every supervisee's note would train people to ignore it.
//!
//! A part is listed in at most one group, checked in the order 1, 2, 3.
use std::cmp::Reverse;

use chrono::{DateTime, Duration, Utc};
use uuid::Uuid;

use super::cosign::awaits_cosign;
use super::read::NoteRow;
use super::review::reviewable;
use super::supervision::writer_actor;
use super::vouch::{counts_as_unsigned, part_is_signed, same_actor};

/// How long a supervisee's note stays in group 3.
pub const REVIEW_DAYS: i64 = 7;

/// The named clinician: their actor (how notes name people), their vault staff id (how drafts
/// name their writer), and the keys they hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    pub actor: String,
    pub staff_id: String,
    pub display_name: Option<String>,
    pub keys: Vec<String>,
}

/// An open draft part, as read from the drafts door.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuDraft {
    /// Normalised vault staff id of the part's writer.
    pub writer: String,
    pub draft_id: Uuid,
    pub chart: Uuid,
    pub due: DateTime<Utc>,
    pub stuck: bool,
}

/// Why a row is listed; the window words it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Why {
    Draft {
        draft_id: Uuid,
        due: DateTime<Utc>,
        stuck: bool,
    },
    WentInUnsigned,
    KeyRepudiated,
    /// `others`: the writer's other current supervisors (actors, sorted); names resolve later.
    NeedsCoSign {
        others: Vec<String>,
    },
    Review,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItem {
    pub chart: Uuid,
    /// `None` for a draft.
    pub part: Option<Uuid>,
    /// A part's `recorded_at`, or a draft's `due`.
    pub when: DateTime<Utc>,
    /// The writer's actor, for groups 2 and 3.
    pub writer: Option<String>,
    pub why: Why,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PersonMenu {
    pub own: Vec<MenuItem>,
    pub cosign: Vec<MenuItem>,
    pub review: Vec<MenuItem>,
}

impl PersonMenu {
    /// What the bar and picker show: owed signatures and co-signatures. Never the review group.
    pub fn count(&self) -> usize {
        self.own.len() + self.cosign.len()
    }
}

/// Group 1 for a part: went in unsigned and still is, or counts as unsigned under `me`'s key.
fn own_why(p: &NoteRow, me: &Person) -> Option<Why> {
    let drafted_by_me = p
        .drafted_actor
        .as_deref()
        .is_some_and(|d| same_actor(d, &me.actor));
    if p.signed_by.is_none() && !part_is_signed(p) && drafted_by_me {
        return Some(Why::WentInUnsigned);
    }
    let signed_by_me = p
        .signed_by
        .as_ref()
        .is_some_and(|s| same_actor(&s.actor, &me.actor));
    (counts_as_unsigned(p) && signed_by_me).then_some(Why::KeyRepudiated)
}

/// Group 2 for a part: awaiting a co-sign, written by someone else who has `me` as supervisor now.
fn cosign_why(p: &NoteRow, me: &Person) -> Option<Why> {
    let by_someone_else = writer_actor(p).is_some_and(|w| !same_actor(w, &me.actor));
    let mine = p
        .writer_supervisors
        .iter()
        .any(|s| same_actor(s, &me.actor));
    if !(awaits_cosign(p) && by_someone_else && mine) {
        return None;
    }
    let mut others: Vec<String> = p
        .writer_supervisors
        .iter()
        .filter(|s| !same_actor(s, &me.actor))
        .cloned()
        .collect();
    others.sort();
    Some(Why::NeedsCoSign { others })
}

/// Sort the three groups out of already-read facts. An empty `me.actor` (no one is named) gets
/// an empty menu.
pub fn classify(
    me: &Person,
    parts: &[NoteRow],
    drafts: &[MenuDraft],
    now: DateTime<Utc>,
) -> PersonMenu {
    let mut menu = PersonMenu::default();
    if me.actor.is_empty() {
        return menu;
    }
    let mut mine: Vec<&MenuDraft> = drafts
        .iter()
        .filter(|d| !me.staff_id.is_empty() && d.writer == me.staff_id)
        .collect();
    mine.sort_by_key(|d| d.due);
    menu.own = mine
        .into_iter()
        .map(|d| MenuItem {
            chart: d.chart,
            part: None,
            when: d.due,
            writer: None,
            why: Why::Draft {
                draft_id: d.draft_id,
                due: d.due,
                stuck: d.stuck,
            },
        })
        .collect();

    let window_start = now - Duration::days(REVIEW_DAYS);
    let mut sorted: Vec<&NoteRow> = parts.iter().collect();
    sorted.sort_by_key(|p| Reverse((p.recorded_at, p.event_id)));
    for p in sorted {
        let item = |why, writer| MenuItem {
            chart: p.chart,
            part: Some(p.event_id),
            when: p.recorded_at,
            writer,
            why,
        };
        let writer = || writer_actor(p).map(str::to_string);
        if let Some(why) = own_why(p, me) {
            menu.own.push(item(why, None));
        } else if let Some(why) = cosign_why(p, me) {
            menu.cosign.push(item(why, writer()));
        } else if reviewable(p, &me.actor) && p.recorded_at >= window_start {
            menu.review.push(item(Why::Review, writer()));
        }
    }
    menu
}

#[cfg(test)]
#[path = "menu_tests.rs"]
mod tests;
