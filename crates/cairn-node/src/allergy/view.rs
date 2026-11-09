//! The allergy header strip and the expanded lists, as a PURE function of the record (allergy
//! slices 1 and 2). No database, no clock: the caller passes the local UTC offset.
//!
//! Reading order for a junior: [`build_view`] first asks [`resolve`] which entries are current
//! (corrections, de-labels and withdrawals folded into threads), then splits the live heads into
//! lists, decides each list's status ([`ListStatus`]), turns every head into an [`EntryLine`], words
//! the historical and entered-in-error lines, and finally the `strip` module composes the one-line
//! strip from those. Vocabulary stays strings (a peer's newer word must still read), so every
//! `match` here has an "unknown" arm.
use super::history_words::{dash_note, delabel_tail, historical_text, none_history};
use super::read::{AllergyRow, NoneRow, PatientAllergies, RetractionRow};
use super::strip::{strip_text, Strip};
use super::thread::{by_words, key_marker, resolve, Head};
use cairn_event::allergy::entry_words_dated;
use chrono::{DateTime, FixedOffset, NaiveDate, Utc};
use uuid::Uuid;

/// What one list (drug / other) says when it has no entries to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListStatus {
    /// At least one allergy entry exists.
    Entries,
    /// A "none known" statement stands, asked on this local day.
    NoneKnown { asked: NaiveDate },
    /// Nobody has asked.
    NotYetAsked,
}

/// One line of an expanded list (`cairn-gui-tauri`'s `allergies::pane` serialises it to the window).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryLine {
    pub event_id: Uuid,
    pub text: String,
    /// A drug/other entry of kind `allergy` (the red sticker); adverse effects are not.
    pub prominent: bool,
    pub chart: Uuid,
    pub doubted_chart: bool,
    /// Who signed, with the key's marker: "Dr Ada (signer unconfirmed)" or "Dr Ada (key had been
    /// repudiated)" (plan D4). `None` when this node cannot name the key.
    pub signer: Option<String>,
    /// The terminal's local day it was recorded (the caller's offset, R3).
    pub recorded: NaiveDate,
    /// This line's older versions, "Earlier: … (recorded 8 Nov 2026 by Dr Ada)", newest first,
    /// each ending with any counted overlay naming it (" — entered in error …" / " — de-labelled
    /// …: tolerated …", ruling R-4c).
    pub earlier: Vec<String>,
    /// On a historical line: the standing de-label that made it so (the window withdraws it).
    pub challenge: Option<Uuid>,
}

/// One "Entered in error: …" line, with the older versions a withdrawn entry had replaced
/// ("Earlier: …", newest first; empty for a withdrawn de-label or "none known" statement).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnteredInError {
    pub text: String,
    pub earlier: Vec<String>,
}

/// One "none known" line: its words, and the statement it reads (the window withdraws it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoneLine {
    pub event_id: Uuid,
    pub text: String,
    /// The statement is on a chart whose link to this one is in doubt (its text says so too): the
    /// window offers no act on it (review C1 — a possible stranger's line is never struck here).
    pub doubted_chart: bool,
}

/// A none line compares to its words, so a test can say `assert_eq!(v.standing_none, ["…"])`.
impl PartialEq<&str> for NoneLine {
    fn eq(&self, words: &&str) -> bool {
        self.text == *words
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllergyView {
    pub strip: String,
    pub drug: ListStatus,
    pub other: ListStatus,
    /// Drug allergies (sorted), then drug rows of an unknown kind (not prominent).
    pub drug_allergies: Vec<EntryLine>,
    /// Other allergies (sorted), then other/unknown-allergen rows of an unknown kind.
    pub other_allergies: Vec<EntryLine>,
    pub adverse_effects: Vec<EntryLine>,
    /// "Previously recorded: …": statements overridden in every list they cover, superseded, or
    /// on a doubted chart (those end with the doubt mark and never stand for this patient).
    pub previously_none: Vec<NoneLine>,
    /// The statement(s) a "none known" list rests on, with who signed (paper's initialled NKDA):
    /// "No known allergies — asked 8 Nov 2026 by Dr Bea". Drug first when two stand.
    pub standing_none: Vec<NoneLine>,
    /// Allergy events this terminal holds but cannot read (`PatientAllergies::unreadable`).
    pub unreadable: usize,
    /// De-labelled lines: "Historically reported: … Challenged: tolerated …". Never prominent.
    pub historical: Vec<EntryLine>,
    /// Withdrawn entries, de-labels and "none known" statements: "Entered in error: …".
    pub entered_in_error: Vec<EnteredInError>,
    /// De-labels and withdrawals that count for nothing, and why: "Not counted: …".
    pub ignored_overlays: Vec<String>,
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
pub(super) const DOUBT: &str = " (on a chart whose link to this one is in doubt)";
/// Appended to every live head whose allergen has another, disagreeing, live account (plan D5).
const DIFFERING: &str = " — differing accounts";

/// `8 Nov 2026`.
pub(super) fn day_text(d: NaiveDate) -> String {
    use chrono::Datelike;
    format!("{} {} {}", d.day(), MONTHS[d.month0() as usize], d.year())
}

/// The onset as the clinician reads it (`14 Mar 2019`, `Mar 2019`, `2019`); `None` when unknown.
/// A value that does not parse (a peer's newer shape) reads as itself.
pub(super) fn onset_text(precision: &str, value: Option<&str>) -> Option<String> {
    let v = value?;
    let month = |m: &str| {
        m.parse::<usize>()
            .ok()
            .filter(|m| (1..=12).contains(m))
            .map(|m| MONTHS[m - 1])
    };
    let parts: Vec<&str> = v.split('-').collect();
    let shaped = match (precision, parts.as_slice()) {
        ("year", [y]) => Some((*y).to_string()),
        ("month", [y, m]) => month(m).map(|m| format!("{m} {y}")),
        ("day", [y, m, d]) => match (month(m), d.parse::<u32>()) {
            (Some(m), Ok(d)) => Some(format!("{d} {m} {y}")),
            _ => None,
        },
        _ => None,
    };
    Some(shaped.unwrap_or_else(|| v.to_string()))
}

/// The row's wording plus its onset as a clinician reads it (`Mar 2019`; the signed twin carries
/// the ISO onset, R17).
pub fn entry_line_text(row: &AllergyRow) -> String {
    let onset = onset_text(&row.onset_precision, row.onset_value.as_deref());
    let words = entry_words_dated(
        &row.kind,
        &row.source,
        &row.term,
        &row.category,
        row.reaction_text.as_deref(),
        onset.as_deref(),
    );
    format!("{words}{}", allergen_kind_mark(row))
}

/// `" (food)"` for a peer's newer allergen kind, which files under Other but must say what it
/// is; nothing for `drug` and `other`.
pub(super) fn allergen_kind_mark(row: &AllergyRow) -> String {
    match row.allergen_kind.as_str() {
        "drug" | "other" => String::new(),
        k => format!(" ({k})"),
    }
}

/// Sort rank of a reaction: anaphylaxis, not-known, angioedema, urticaria, rash, other, GI upset,
/// unknown peer word last.
fn category_rank(c: &str) -> u8 {
    match c {
        "anaphylaxis" => 0,
        "not-known" => 1,
        "angioedema" => 2,
        "urticaria" => 3,
        "rash" => 4,
        "other" => 5,
        "gi-upset" => 6,
        _ => 7,
    }
}

/// Rank, then newest first, then event id (a total, deterministic order).
fn order_key(r: &AllergyRow) -> (u8, std::cmp::Reverse<(i64, i32)>, Uuid) {
    (
        category_rank(&r.category),
        std::cmp::Reverse(r.hlc),
        r.event_id,
    )
}

/// Heads in list order (`order_key` of each head's row).
fn sorted<'h, 'a>(mut heads: Vec<&'h Head<'a>>) -> Vec<&'h Head<'a>> {
    heads.sort_by_key(|h| order_key(h.row));
    heads
}

/// The newest of the given none rows (HLC, then id).
fn newest<'a>(rows: impl Iterator<Item = &'a NoneRow>) -> Option<&'a NoneRow> {
    rows.max_by_key(|n| (n.hlc, n.event_id))
}

pub(super) fn local_day(t: chrono::DateTime<chrono::Utc>, offset: FixedOffset) -> NaiveDate {
    t.with_timezone(&offset).date_naive()
}

/// Build the whole view over every linked chart's rows (doubt is carried by `a.doubted_charts`).
///
/// [`resolve`] first folds corrections, de-labels and withdrawals into threads; slice 1's list,
/// override and strip logic then runs over the LIVE heads only. A de-labelled head is listed
/// under `historical` and overrides nothing; a withdrawn one is only "Entered in error".
pub fn build_view(a: &PatientAllergies, offset: FixedOffset) -> AllergyView {
    let r = resolve(a);
    let day = |t: DateTime<Utc>| day_text(local_day(t, offset));
    let drug_rows: Vec<&Head> = r
        .live
        .iter()
        .filter(|h| h.row.allergen_kind == "drug")
        .collect();
    // Anything that is not "drug" (including a peer's newer allergen kind, e.g. "food") is an OTHER allergy.
    let other_rows: Vec<&Head> = r
        .live
        .iter()
        .filter(|h| h.row.allergen_kind != "drug")
        .collect();
    let drug_has = drug_rows.iter().any(|h| h.row.kind == "allergy");
    let other_has = other_rows.iter().any(|h| h.row.kind == "allergy");

    // Validity is PER LIST (design §3.4: an allergy overrides "none known" "in its list"; ruling
    // R16): an "all" statement stands for drugs while no drug allergy exists, and for the other
    // list while no other allergy exists. An unknown scope stands for nothing (history only).
    // A doubted chart's statement stands for nothing either: a stranger's NKDA would under-warn
    // (decision 11 shows a doubted chart's entries because over-warning is the safe error). Nor
    // does a suspect key's — since repudiated, or doubtful (plan D4, review I1: such a key never
    // takes a warning away). A withdrawn
    // statement is not even history: it is only "Entered in error".
    let doubted = |n: &NoneRow| a.doubted_charts.contains(&n.chart);
    let can_stand = |n: &NoneRow| !doubted(n) && !n.key.is_suspect();
    let nones: Vec<&NoneRow> = a
        .none
        .iter()
        .filter(|n| !r.withdrawn_none.iter().any(|w| w.event_id == n.event_id))
        .collect();
    let drug_none = newest(
        nones
            .iter()
            .copied()
            .filter(|n| can_stand(n) && !drug_has && (n.scope == "drug" || n.scope == "all")),
    );
    let other_none = newest(
        nones
            .iter()
            .copied()
            .filter(|n| can_stand(n) && !other_has && n.scope == "all"),
    );
    // Both lists rest on one "all" statement: it reads as "No known allergies".
    let one_all_row = drug_none
        .zip(other_none)
        .is_some_and(|(d, o)| d.event_id == o.event_id);

    let status = |has: bool, n: Option<&NoneRow>| {
        if has {
            ListStatus::Entries
        } else if let Some(n) = n {
            ListStatus::NoneKnown {
                asked: local_day(n.recorded_at, offset),
            }
        } else {
            ListStatus::NotYetAsked
        }
    };
    let drug = status(drug_has, drug_none);
    let other = status(other_has, other_none);

    let used = |n: &NoneRow| {
        drug_none.map(|d| d.event_id) == Some(n.event_id)
            || other_none.map(|d| d.event_id) == Some(n.event_id)
    };
    let mut unused: Vec<&NoneRow> = nones.iter().copied().filter(|n| !used(n)).collect();
    unused.sort_by_key(|n| std::cmp::Reverse((n.hlc, n.event_id)));
    let previously_none = unused
        .iter()
        .map(|n| NoneLine {
            event_id: n.event_id,
            text: none_history("Previously recorded", n, offset, doubted(n)),
            doubted_chart: doubted(n),
        })
        .collect();
    let standing = |what: &str, n: &NoneRow| NoneLine {
        event_id: n.event_id,
        doubted_chart: doubted(n),
        text: format!(
            "{what} — asked {}{}",
            day(n.recorded_at),
            by_words(n.signer.as_deref(), n.key)
        ),
    };
    let standing_none: Vec<NoneLine> = match (drug_none, other_none) {
        (Some(d), _) if one_all_row => vec![standing("No known allergies", d)],
        (d, o) => d
            .map(|d| standing("No known drug allergies", d))
            .into_iter()
            .chain(o.map(|o| standing("No known other allergies", o)))
            .collect(),
    };

    let doubt = |r: &AllergyRow| {
        if a.doubted_charts.contains(&r.chart) {
            DOUBT
        } else {
            ""
        }
    };
    // An older version's line. A counted overlay naming THAT version is said on it (ruling
    // R-4c), with its reason or note: it covers the version only, never the later correction
    // above it, but the clinician's signed act must not vanish from sight.
    let earlier_text = |e: &AllergyRow| {
        let overlay = if let Some(w) = r.withdrawals.get(&e.event_id) {
            format!(
                " — entered in error {}{}{}",
                day(w.recorded_at),
                by_words(w.signer.as_deref(), w.key),
                dash_note(w.text.as_deref())
            )
        } else if let Some(c) = r.de_labels.get(&e.event_id) {
            delabel_tail(c, offset)
        } else {
            String::new()
        };
        format!(
            "Earlier: {} (recorded {}{}){overlay}",
            entry_line_text(e),
            day(e.recorded_at),
            by_words(e.signer.as_deref(), e.key)
        )
    };
    // One expanded-list line: `text` is the caller's (a head's words, or a de-labelled line's).
    let line = |r: &AllergyRow, text: String, earlier: &[&AllergyRow]| EntryLine {
        event_id: r.event_id,
        text,
        prominent: r.kind == "allergy",
        chart: r.chart,
        doubted_chart: a.doubted_charts.contains(&r.chart),
        signer: r
            .signer
            .as_deref()
            .map(|s| format!("{s}{}", key_marker(r.key))),
        recorded: local_day(r.recorded_at, offset),
        earlier: earlier.iter().map(|e| earlier_text(e)).collect(),
        challenge: None,
    };
    let head_line = |h: &Head| {
        let differing = if h.differing { DIFFERING } else { "" };
        let text = format!("{}{}{differing}", entry_line_text(h.row), doubt(h.row));
        line(h.row, text, &h.earlier)
    };
    // Allergies first, then rows of a kind this node does not know (listed, never prominent).
    let list = |rows: &[&Head]| -> Vec<EntryLine> {
        let (known, unknown): (Vec<&Head>, Vec<&Head>) = rows
            .iter()
            .copied()
            .filter(|h| h.row.kind != "adverse-effect")
            .partition(|h| h.row.kind == "allergy");
        sorted(known)
            .into_iter()
            .chain(sorted(unknown))
            .map(head_line)
            .collect()
    };
    let drug_allergies = list(&drug_rows);
    let other_allergies = list(&other_rows);
    let adverse_effects: Vec<EntryLine> = sorted(
        r.live
            .iter()
            .filter(|h| h.row.kind == "adverse-effect")
            .collect(),
    )
    .into_iter()
    .map(head_line)
    .collect();
    let historical: Vec<EntryLine> = r
        .historical
        .iter()
        .map(|h| {
            let text = format!(
                "{}{}",
                historical_text(h.head, h.challenge, offset),
                doubt(h.head)
            );
            EntryLine {
                prominent: false,
                challenge: Some(h.challenge.event_id),
                ..line(h.head, text, &h.earlier)
            }
        })
        .collect();
    // "(withdrawn 10 Nov 2026 by Dr Ada) — {reason}": who struck it, when, and why.
    let withdrawn = |by: &RetractionRow| {
        format!(
            "(withdrawn {}{}){}",
            day(by.recorded_at),
            by_words(by.signer.as_deref(), by.key),
            dash_note(by.text.as_deref())
        )
    };
    // A withdrawn entry keeps its doubt mark and its older versions (A in A←B, B withdrawn), and
    // the standing de-label it carried, if any (a de-labelled line struck in one act, ADR-0097 decision 19).
    let struck_entries = r.withdrawn.iter().map(|w| EnteredInError {
        text: format!(
            "Entered in error: {}{} {}{}",
            entry_line_text(w.row),
            doubt(w.row),
            withdrawn(w.by),
            r.de_labels
                .get(&w.row.event_id)
                .map(|c| delabel_tail(c, offset))
                .unwrap_or_default()
        ),
        earlier: w.earlier.iter().map(|e| earlier_text(e)).collect(),
    });
    let struck_others = r
        .withdrawn_challenges
        .iter()
        .map(|w| {
            format!(
                "Entered in error: de-label — tolerated {}{} {}",
                w.row.tolerated,
                dash_note(w.row.text.as_deref()),
                withdrawn(w.by)
            )
        })
        .chain(r.withdrawn_none.iter().map(|n| {
            let reason = r.withdrawals.get(&n.event_id).map(|w| w.text.as_deref());
            format!(
                "{}{}",
                none_history("Entered in error", n, offset, doubted(n)),
                dash_note(reason.flatten())
            )
        }))
        .map(|text| EnteredInError {
            text,
            earlier: vec![],
        });
    let entered_in_error = struck_entries.chain(struck_others).collect();

    let strip = strip_text(&Strip {
        drug: &drug,
        other: &other,
        one_all_row,
        drug_from_all: drug_none.is_some_and(|n| n.scope == "all"),
        drug_lines: &prominent_texts(&drug_allergies),
        other_lines: &prominent_texts(&other_allergies),
        adverse: adverse_effects.len(),
        historical: historical.len(),
        unrecognised: r
            .live
            .iter()
            .filter(|h| h.row.kind != "allergy" && h.row.kind != "adverse-effect")
            .count(),
        unreadable: a.unreadable,
    });
    AllergyView {
        strip,
        drug,
        other,
        drug_allergies,
        other_allergies,
        adverse_effects,
        previously_none,
        standing_none,
        unreadable: a.unreadable,
        historical,
        entered_in_error,
        ignored_overlays: r.ignored_overlays.clone(),
    }
}

/// Texts of the lines that may reach the strip (allergies only; unknown kinds never do).
fn prominent_texts(lines: &[EntryLine]) -> Vec<&str> {
    lines
        .iter()
        .filter(|l| l.prominent)
        .map(|l| l.text.as_str())
        .collect()
}

#[cfg(test)]
#[path = "view_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "view_overlay_tests.rs"]
mod overlay_tests;

#[cfg(test)]
#[path = "view_history_tests.rs"]
mod history_tests;
