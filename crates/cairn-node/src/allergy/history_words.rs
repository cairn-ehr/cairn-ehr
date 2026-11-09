//! The wording of the allergy list's HISTORY lines — a de-labelled line, and a "none known"
//! statement that no longer stands or was withdrawn (split out of `view.rs` to keep both files
//! short). Pure; `view::build_view` is the only caller.
use super::read::{AllergyRow, ChallengeRow, NoneRow};
use super::thread::by_words;
use super::view::{allergen_kind_mark, day_text, local_day, onset_text, DOUBT};
use chrono::FixedOffset;

/// A de-label's date as the clinician reads it, with its leading space (`" Mar 2022"`), or
/// nothing when not known.
pub(super) fn challenge_date(c: &ChallengeRow) -> String {
    onset_text(&c.date_precision, c.date_value.as_deref())
        .map(|d| format!(" {d}"))
        .unwrap_or_default()
}

/// A "none known" statement as history: `"{prefix}: no known allergies (asked 8 Nov 2026 by Dr
/// Bea)"`, ending with the doubt mark when its chart's link is in doubt. `prefix` is "Previously
/// recorded" (it stands for no list) or "Entered in error" (it was withdrawn).
pub(super) fn none_history(
    prefix: &str,
    n: &NoneRow,
    offset: FixedOffset,
    doubted: bool,
) -> String {
    let what = match n.scope.as_str() {
        "all" => "no known allergies".to_string(),
        "drug" => "no known drug allergies".to_string(),
        s => format!("none known ({s})"),
    };
    format!(
        "{prefix}: {what} (asked {}{}){}",
        day_text(local_day(n.recorded_at, offset)),
        by_words(n.signer.as_deref(), n.key),
        if doubted { DOUBT } else { "" }
    )
}

/// A de-labelled line: `"Historically reported: penicillin allergy. Challenged: tolerated
/// amoxicillin Mar 2022 — {note}"`, a newer allergen kind marked after the kind (`peanut allergy
/// (food)`). The date reads as an onset does; an unknown one is left out.
pub(super) fn historical_text(r: &AllergyRow, c: &ChallengeRow) -> String {
    let kind = match r.kind.as_str() {
        "allergy" => "allergy",
        "adverse-effect" => "adverse effect",
        k => k,
    };
    let note = c
        .text
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(|t| format!(" — {t}"))
        .unwrap_or_default();
    format!(
        "Historically reported: {} {kind}{}. Challenged: tolerated {}{}{note}",
        r.term,
        allergen_kind_mark(r),
        c.tolerated,
        challenge_date(c)
    )
}
