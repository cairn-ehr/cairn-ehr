//! The wording of the allergy list's HISTORY lines — a de-labelled line, and a "none known"
//! statement that no longer stands or was withdrawn (split out of `view.rs` to keep both files
//! short). Pure; `view::build_view` is the only caller.
use super::read::{AllergyRow, ChallengeRow, NoneRow};
use super::thread::{by_words, by_words_inside};
use super::view::{allergen_kind_mark, day_text, local_day, onset_text, DOUBT};
use chrono::FixedOffset;

/// A de-label's date as the clinician reads it, with its leading space (`" Mar 2022"`), or
/// nothing when not known.
pub(super) fn challenge_date(c: &ChallengeRow) -> String {
    onset_text(&c.date_precision, c.date_value.as_deref())
        .map(|d| format!(" {d}"))
        .unwrap_or_default()
}

/// A signed act's free text as a tail, `" — wrong patient"`: trimmed, and nothing when blank or
/// absent. A withdrawal's reason and a de-label's note are shown wherever their act is worded.
pub(super) fn dash_note(text: Option<&str>) -> String {
    text.map(str::trim)
        .filter(|t| !t.is_empty())
        .map(|t| format!(" — {t}"))
        .unwrap_or_default()
}

/// A standing de-label said after a line it covers but that is not shown as historical (an
/// older version's "Earlier:" line, ruling R-4c; or a de-labelled line since struck as entered
/// in error): `" — de-labelled 9 Nov 2026 by Dr Ada: tolerated amoxicillin Mar 2022 — {note}"`.
pub(super) fn delabel_tail(c: &ChallengeRow, offset: FixedOffset) -> String {
    format!(
        " — de-labelled {}{}: tolerated {}{}{}",
        day_text(local_day(c.recorded_at, offset)),
        by_words(c.signer.as_deref(), c.key),
        c.tolerated,
        challenge_date(c),
        dash_note(c.text.as_deref())
    )
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
/// amoxicillin Mar 2022 (de-labelled 9 Nov 2026 by Dr Ada) — {note}"`, a newer allergen kind
/// marked after the kind (`peanut allergy (food)`). The date reads as an onset does; an unknown
/// one is left out. The bracket names who lifted the label and the day they signed it (paper's
/// initialled "tolerated amoxil 03/22"), with the key's marker inside it ([`by_words_inside`]).
pub(super) fn historical_text(r: &AllergyRow, c: &ChallengeRow, offset: FixedOffset) -> String {
    let kind = match r.kind.as_str() {
        "allergy" => "allergy",
        "adverse-effect" => "adverse effect",
        k => k,
    };
    format!(
        "Historically reported: {} {kind}{}. Challenged: tolerated {}{} (de-labelled {}{}){}",
        r.term,
        allergen_kind_mark(r),
        c.tolerated,
        challenge_date(c),
        day_text(local_day(c.recorded_at, offset)),
        by_words_inside(c.signer.as_deref(), c.key),
        dash_note(c.text.as_deref())
    )
}
