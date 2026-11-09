//! Row builders for the overlay and check tests (`thread_tests`, `view_overlay_tests`,
//! `check_tests`). Test-only.
//!
//! Every row is on chart `id(CHART)`, signed by "Dr Ada" under a standing key, at noon UTC on a
//! November 2026 day (the same local day at +10:00). A test changes only the field it is about.
use super::read::{
    AllergyRow, ChallengeRow, Coding, KeyState, NoneRow, PatientAllergies, RetractionRow,
};
use chrono::{DateTime, FixedOffset, TimeZone, Utc};
use uuid::Uuid;

pub const CHART: u128 = 1000;

pub fn bne() -> FixedOffset {
    FixedOffset::east_opt(10 * 3600).unwrap()
}
pub fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}
/// Noon UTC on a November 2026 day.
pub fn at(d: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, d, 12, 0, 0).unwrap()
}
fn hlc(d: u32) -> (i64, i32) {
    (at(d).timestamp_millis(), 0)
}

pub fn entry(n: u128, ak: &str, term: &str, kind: &str, cat: &str, day: u32) -> AllergyRow {
    AllergyRow {
        event_id: id(n),
        chart: id(CHART),
        allergen_kind: ak.into(),
        term: term.into(),
        kind: kind.into(),
        category: cat.into(),
        reaction_text: None,
        onset_precision: "not-known".into(),
        onset_value: None,
        source: "seen-documented".into(),
        recorded_at: at(day),
        hlc: hlc(day),
        signer: Some("Dr Ada".into()),
        corrects: None,
        key: KeyState::Standing,
        coding: None,
    }
}

/// A penicillin drug allergy with the given reaction.
pub fn pen(n: u128, cat: &str, day: u32) -> AllergyRow {
    entry(n, "drug", "penicillin", "allergy", cat, day)
}

/// `row`, now correcting event `target`.
pub fn correcting(mut row: AllergyRow, target: u128) -> AllergyRow {
    row.corrects = Some(id(target));
    row
}

/// `row`, now coded to drug `code` (computed, `id(code)`) under `system`; the display is its term.
pub fn coded(mut row: AllergyRow, system: &str, code: u128) -> AllergyRow {
    row.coding = Some(Coding {
        system: system.into(),
        code: id(code).to_string(),
        display: row.term.clone(),
    });
    row
}

pub fn none(n: u128, scope: &str, day: u32) -> NoneRow {
    NoneRow {
        event_id: id(n),
        chart: id(CHART),
        scope: scope.into(),
        recorded_at: at(day),
        hlc: hlc(day),
        signer: Some("Dr Ada".into()),
        key: KeyState::Standing,
    }
}

/// "Tolerated amoxicillin", Mar 2022, no note.
pub fn challenge(n: u128, target: u128, day: u32) -> ChallengeRow {
    ChallengeRow {
        event_id: id(n),
        chart: id(CHART),
        target: id(target),
        tolerated: "amoxicillin".into(),
        date_precision: "month".into(),
        date_value: Some("2022-03".into()),
        text: None,
        recorded_at: at(day),
        hlc: hlc(day),
        signer: Some("Dr Ada".into()),
        key: KeyState::Standing,
    }
}

pub fn retraction(n: u128, target: u128, day: u32) -> RetractionRow {
    RetractionRow {
        event_id: id(n),
        chart: id(CHART),
        target: id(target),
        text: None,
        recorded_at: at(day),
        hlc: hlc(day),
        signer: Some("Dr Ada".into()),
        key: KeyState::Standing,
    }
}

pub fn pa(
    entries: Vec<AllergyRow>,
    none: Vec<NoneRow>,
    challenges: Vec<ChallengeRow>,
    retractions: Vec<RetractionRow>,
) -> PatientAllergies {
    PatientAllergies {
        entries,
        none,
        challenges,
        retractions,
        doubted_charts: vec![],
        unreadable: 0,
    }
}
