//! Every sentence the front door's possible-duplicate tray shows, as pure functions (repair path
//! R5b, #680; design page "R5b — the worklist, designed 2026-10-08").
//!
//! The tray is the records clerk's possible-duplicate tray: a collapsed `<details>` whose summary
//! counts the open pairs of records. It may be HIDDEN only when that is the truth — none open on a
//! node whose check is current; every failed read is a worded line (R5a's rule, one tray over).
use crate::duplicates::view::{EntryFlags, ACCEPTED_HEADING, DISPUTED_NOTE, HEADING, VETO_NOTE};
use crate::funnel::rows::PersonRowView;
use cairn_node::duplicate_check::{status_line, CheckState};
use serde::Serialize;

/// At most this many entries are drawn (each costs two record reads); the rest are counted.
pub const MAX_SHOWN: usize = 20;
pub const NEWER_LABEL: &str = "Registered more recently";
pub const OLDER_LABEL: &str = "Already on file";
pub const STRONG_NOTE: &str = "The matcher rates this a strong match.";
/// Why a side has no row although its record was read: no chart of it could be shown.
pub const NO_CHARTS: &str = "no charts were found for it.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TrayCountView {
    /// The `<summary>` text; `None` hides the tray (checked, none open, node current).
    pub summary: Option<String>,
    /// Why "0" or a count may be incomplete (R4's sentence); `None` on a Current node.
    pub status_line: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SideView {
    pub row: Option<PersonRowView>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorklistEntryView {
    pub heading: String,
    pub newer_label: String,
    pub newer: SideView,
    pub older_label: String,
    pub older: SideView,
    pub notes: Vec<String>,
    /// The chart Review opens (the newer record's); `None` when that side could not be read, so
    /// it is not in `AppState::shown` and Review could only be refused.
    pub open_chart: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorklistView {
    pub entries: Vec<WorklistEntryView>,
    pub more: Option<String>,
    pub error: Option<String>,
}

/// The summary and status line. **Pure.**
pub fn tray_count_view(
    count: Result<usize, String>,
    status: Result<CheckState, String>,
) -> TrayCountView {
    let status_line = match &status {
        Ok(CheckState::Current { .. }) => None,
        Ok(state) => Some(status_line(state)),
        Err(e) => Some(format!("Duplicate check status unknown: {e}")),
    };
    match count {
        Err(e) => TrayCountView {
            summary: Some(format!("Possible duplicates — could not be checked: {e}")),
            status_line,
        },
        Ok(0) if status_line.is_none() => TrayCountView {
            summary: None,
            status_line: None,
        },
        Ok(n) => TrayCountView {
            summary: Some(format!("Possible duplicates ({n})")),
            status_line,
        },
    }
}

/// "N more …" for entries beyond those shown. **Pure.**
pub fn more_line(shown: usize, total: usize) -> Option<String> {
    match total.saturating_sub(shown) {
        0 => None,
        1 => Some("1 more possible duplicate, older than these.".into()),
        n => Some(format!("{n} more possible duplicates, older than these.")),
    }
}

/// One side of an entry. **Pure.** A side with no row is worded, never left blank: the clerk
/// must not read an empty side as "nothing on file".
fn side(read: Result<Option<PersonRowView>, String>) -> SideView {
    match read {
        Ok(Some(row)) => SideView {
            row: Some(row),
            error: None,
        },
        Ok(None) => SideView {
            row: None,
            error: Some(format!("This record could not be read here: {NO_CHARTS}")),
        },
        Err(e) => SideView {
            row: None,
            error: Some(format!("This record could not be read here: {e}")),
        },
    }
}

/// One entry. **Pure.** `open_chart` is `Some` only when the newer side was read.
pub fn entry_view(
    flags: EntryFlags,
    band: &str,
    newer: Result<Option<PersonRowView>, String>,
    older: Result<Option<PersonRowView>, String>,
    open_chart: Option<String>,
) -> WorklistEntryView {
    let mut notes = vec![];
    if band == "auto_candidate" {
        notes.push(STRONG_NOTE.into());
    }
    if flags.vetoed {
        notes.push(VETO_NOTE.into());
    }
    if flags.disputed {
        notes.push(DISPUTED_NOTE.into());
    }
    WorklistEntryView {
        heading: if flags.accepted {
            ACCEPTED_HEADING
        } else {
            HEADING
        }
        .into(),
        newer_label: NEWER_LABEL.into(),
        newer: side(newer),
        older_label: OLDER_LABEL.into(),
        older: side(older),
        notes,
        open_chart,
    }
}

/// The list, or why it could not be read. **Pure.** `Ok` carries `(entries, total)`.
pub fn worklist_view(read: Result<(Vec<WorklistEntryView>, usize), String>) -> WorklistView {
    match read {
        Err(e) => WorklistView {
            entries: vec![],
            more: None,
            error: Some(format!("Could not read the possible duplicates: {e}")),
        },
        Ok((entries, total)) => WorklistView {
            more: more_line(entries.len(), total),
            entries,
            error: None,
        },
    }
}

#[cfg(test)]
#[path = "view_tests.rs"]
mod tests;
