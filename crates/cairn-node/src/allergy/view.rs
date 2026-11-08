//! The allergy header strip and the expanded lists, as a PURE function of the record (allergy
//! slice 1). No database, no clock: the caller passes the local UTC offset.
//!
//! Reading order for a junior: [`build_view`] splits the rows into lists, decides each list's
//! status ([`ListStatus`]), turns every row into an [`EntryLine`], and finally [`strip_text`]
//! composes the one-line strip from those. Vocabulary stays strings (a peer's newer word must
//! still read), so every `match` here has an "unknown" arm.
use super::read::{AllergyRow, NoneRow, PatientAllergies};
use cairn_event::allergy::entry_words_dated;
use chrono::{FixedOffset, NaiveDate};
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

/// One line of an expanded list (Task 5 serialises this to the window).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryLine {
    pub event_id: Uuid,
    pub text: String,
    /// A drug/other entry of kind `allergy` (the red sticker); adverse effects are not.
    pub prominent: bool,
    pub chart: Uuid,
    pub doubted_chart: bool,
    pub signer: Option<String>,
    /// Node-local day it was recorded.
    pub recorded: NaiveDate,
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
    pub previously_none: Vec<String>,
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const DOUBT: &str = " (on a chart whose link to this one is in doubt)";

/// `8 Nov 2026`.
fn day_text(d: NaiveDate) -> String {
    use chrono::Datelike;
    format!("{} {} {}", d.day(), MONTHS[d.month0() as usize], d.year())
}

/// The onset as the clinician reads it (`14 Mar 2019`, `Mar 2019`, `2019`); `None` when unknown.
/// A value that does not parse (a peer's newer shape) reads as itself.
fn onset_text(precision: &str, value: Option<&str>) -> Option<String> {
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

/// The row's wording plus its onset (the twin has no dates; this layer adds them).
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
    // A peer's newer allergen kind (e.g. "food") files under Other but says what it is.
    match row.allergen_kind.as_str() {
        "drug" | "other" => words,
        k => format!("{words} ({k})"),
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

fn sorted(mut rows: Vec<&AllergyRow>) -> Vec<&AllergyRow> {
    rows.sort_by_key(|r| order_key(r));
    rows
}

/// The newest of the given none rows (HLC, then id).
fn newest<'a>(rows: impl Iterator<Item = &'a NoneRow>) -> Option<&'a NoneRow> {
    rows.max_by_key(|n| (n.hlc, n.event_id))
}

fn local_day(t: chrono::DateTime<chrono::Utc>, offset: FixedOffset) -> NaiveDate {
    t.with_timezone(&offset).date_naive()
}

/// Build the whole view over every linked chart's rows (doubt is carried by `a.doubted_charts`).
pub fn build_view(a: &PatientAllergies, offset: FixedOffset) -> AllergyView {
    let drug_rows: Vec<&AllergyRow> = a
        .entries
        .iter()
        .filter(|r| r.allergen_kind == "drug")
        .collect();
    // Anything that is not "drug" (including a peer's newer allergen kind, e.g. "food") is an OTHER allergy.
    let other_rows: Vec<&AllergyRow> = a
        .entries
        .iter()
        .filter(|r| r.allergen_kind != "drug")
        .collect();
    let drug_has = drug_rows.iter().any(|r| r.kind == "allergy");
    let other_has = other_rows.iter().any(|r| r.kind == "allergy");

    // A none row is ONE statement: an entry in any list it covers overrides it everywhere.
    let valid = |n: &&NoneRow| match n.scope.as_str() {
        "drug" => !drug_has,
        "all" => !drug_has && !other_has,
        _ => false,
    };
    let drug_none = newest(
        a.none
            .iter()
            .filter(valid)
            .filter(|n| n.scope == "drug" || n.scope == "all"),
    );
    let other_none = newest(a.none.iter().filter(valid).filter(|n| n.scope == "all"));

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
    let mut unused: Vec<&NoneRow> = a.none.iter().filter(|n| !used(n)).collect();
    unused.sort_by_key(|n| std::cmp::Reverse((n.hlc, n.event_id)));
    let previously_none = unused
        .iter()
        .map(|n| {
            let what = match n.scope.as_str() {
                "all" => "no known allergies".to_string(),
                "drug" => "no known drug allergies".to_string(),
                s => format!("none known ({s})"),
            };
            format!(
                "Previously recorded: {what} (asked {})",
                day_text(local_day(n.recorded_at, offset))
            )
        })
        .collect();

    let line = |r: &AllergyRow| EntryLine {
        event_id: r.event_id,
        text: format!(
            "{}{}",
            entry_line_text(r),
            if a.doubted_charts.contains(&r.chart) {
                DOUBT
            } else {
                ""
            }
        ),
        prominent: r.kind == "allergy",
        chart: r.chart,
        doubted_chart: a.doubted_charts.contains(&r.chart),
        signer: r.signer.clone(),
        recorded: local_day(r.recorded_at, offset),
    };
    // Allergies first, then rows of a kind this node does not know (listed, never prominent).
    let list = |rows: &[&AllergyRow]| -> Vec<EntryLine> {
        let (known, unknown): (Vec<&AllergyRow>, Vec<&AllergyRow>) = rows
            .iter()
            .copied()
            .filter(|r| r.kind != "adverse-effect")
            .partition(|r| r.kind == "allergy");
        sorted(known)
            .into_iter()
            .chain(sorted(unknown))
            .map(line)
            .collect()
    };
    let drug_allergies = list(&drug_rows);
    let other_allergies = list(&other_rows);
    let adverse_effects: Vec<EntryLine> = sorted(
        a.entries
            .iter()
            .filter(|r| r.kind == "adverse-effect")
            .collect(),
    )
    .into_iter()
    .map(line)
    .collect();

    let strip = strip_text(
        &drug,
        &other,
        drug_none
            .zip(other_none)
            .is_some_and(|(d, o)| d.event_id == o.event_id),
        &prominent_texts(&drug_allergies),
        &prominent_texts(&other_allergies),
        adverse_effects.len(),
        a.entries
            .iter()
            .filter(|r| r.kind != "allergy" && r.kind != "adverse-effect")
            .count(),
    );
    AllergyView {
        strip,
        drug,
        other,
        drug_allergies,
        other_allergies,
        adverse_effects,
        previously_none,
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

/// Compose the one-line strip: entries within a list join with "; ", sections with " · ".
/// `one_all_row`: both lists are covered by the same "all" statement.
fn strip_text(
    drug: &ListStatus,
    other: &ListStatus,
    one_all_row: bool,
    drug_lines: &[&str],
    other_lines: &[&str],
    adverse: usize,
    unrecognised: usize,
) -> String {
    let mut parts: Vec<String> = Vec::new();
    match (drug, other) {
        (ListStatus::Entries, _) => parts.push(drug_lines.join("; ")),
        (ListStatus::NoneKnown { asked }, ListStatus::NoneKnown { .. }) if one_all_row => {
            parts.push(format!("No known allergies (asked {})", day_text(*asked)))
        }
        (ListStatus::NoneKnown { asked }, _) => {
            parts.push(format!("NKDA (asked {})", day_text(*asked)))
        }
        (ListStatus::NotYetAsked, ListStatus::NotYetAsked) => {
            parts.push("Allergies not yet asked".into())
        }
        (ListStatus::NotYetAsked, _) => parts.push("Drug allergies not yet asked".into()),
    }
    match (drug, other) {
        (_, ListStatus::Entries) => {
            // On the strip "Other:" already says it is an allergy; other wordings stay in full.
            let lines: Vec<&str> = other_lines
                .iter()
                .map(|l| l.strip_prefix("Allergy: ").unwrap_or(l))
                .collect();
            parts.push(format!("Other: {}", lines.join("; ")))
        }
        // Drug and other were each asked, by different statements.
        (ListStatus::NoneKnown { .. }, ListStatus::NoneKnown { asked }) if !one_all_row => parts
            .push(format!(
                "no known other allergies (asked {})",
                day_text(*asked)
            )),
        (ListStatus::NoneKnown { .. }, ListStatus::NotYetAsked) => {
            parts.push("other allergies: not yet asked".into())
        }
        _ => {}
    }
    if adverse > 0 {
        parts.push(format!(
            "+{adverse} adverse effect{}",
            if adverse == 1 { "" } else { "s" }
        ));
    }
    if unrecognised > 0 {
        parts.push(format!(
            "+{unrecognised} unrecognised entr{}",
            if unrecognised == 1 { "y" } else { "ies" }
        ));
    }
    parts.join(" · ")
}

#[cfg(test)]
#[path = "view_tests.rs"]
mod tests;
