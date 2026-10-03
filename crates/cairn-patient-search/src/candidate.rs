//! One row of a candidate list — what §5.8 item 1 requires be shown before a chart may be
//! created: photo, age, locale, last visit, and (Cairn's addition) the chart's trust state.
use crate::person::PersonRow;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// §5.7's chart trust states, projection-side contract.
///
/// Load-bearing for search, not decoration: a John Doe registered an hour ago is precisely
/// the chart a clerk must find when the family arrives with a name. A search that hid
/// identity-pending charts would manufacture a duplicate every time an unidentified patient
/// is later named.
///
/// `rename_all = "kebab-case"` keeps the SERIALIZED spelling identical to the §5.7 tokens
/// `as_str` returns (`"under-review"`, not serde's default `"UnderReview"`): `Candidate`
/// crosses a process boundary (the CLI renders it; the future picker window and native API
/// will carry it as JSON), and two spellings of one closed vocabulary is exactly the kind of
/// wire divergence that is free to prevent now and an additive-evolution headache to heal
/// after anything has consumed the other form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TrustState {
    Confirmed,
    Unconfirmed,
    UnderReview,
    /// This node does not hold the chart's registration; it synced ahead, or lies outside
    /// this node's sync scope (R1's `person::trust_of`). Said honestly rather than guessed
    /// at (principle 4): showing "confirmed" for a chart we cannot vouch for would be a lie.
    Unknown,
}

impl TrustState {
    pub fn as_str(self) -> &'static str {
        match self {
            TrustState::Confirmed => "confirmed",
            TrustState::Unconfirmed => "unconfirmed",
            TrustState::UnderReview => "under-review",
            TrustState::Unknown => "unknown",
        }
    }
}

/// An age together with what it was derived from. The basis travels because an age derived
/// from a document-verified DOB and one derived from a clinician's estimate are different
/// claims, and a clerk comparing candidates needs to know which is which (principle 4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Age {
    pub years: u32,
    pub basis: String,
}

/// Gregorian leap-year rule: divisible by 4, except centuries, except every 4th century.
/// Hand-rolled rather than pulling in a date crate (house rule: no new date/time
/// dependency for this task) — the rule is three integer divisions, not worth a dependency.
fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// How many days `month` (1-12) actually has in `year`. `month` is assumed already
/// range-checked by the caller (`parse_ymd` below) — this only resolves the leap-year-dependent
/// case for February.
fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        // Unreachable given `parse_ymd`'s own `1..=12` guard runs before this is ever called;
        // 0 rather than panicking keeps this fn total, so a future caller that skips the
        // guard fails a date-validity comparison rather than crashing the read path.
        _ => 0,
    }
}

/// Parse an ISO `YYYY-MM-DD` date into `(year, month, day)`, or `None` for a partial date,
/// anything unparseable, or a calendrically impossible day (`2026-02-30`).
///
/// Shared by [`age_years`] and the ranking's DOB near-miss (`crate::rank`), so the two can
/// never disagree about what counts as a real date.
pub(crate) fn parse_ymd(s: &str) -> Option<(i32, u32, u32)> {
    let mut it = s.split('-');
    let y = it.next()?.parse::<i32>().ok()?;
    let m = it.next()?.parse::<u32>().ok()?;
    let d = it.next()?.parse::<u32>().ok()?;
    if it.next().is_some() || !(1..=12).contains(&m) {
        return None;
    }
    // Real per-month validation (leap years included), not the old blanket 1..=31:
    // that accepted "2026-02-30" and "2026-04-31" as if every month had 31 days.
    if !(1..=days_in_month(y, m)).contains(&d) {
        return None;
    }
    Some((y, m, d))
}

/// Whole years between two ISO `YYYY-MM-DD` dates, or `None` when that cannot be said
/// honestly.
///
/// Returns `None` for a partial date (`"1980"`), an unparseable one, a birth date after
/// `today`, OR a calendrically impossible one (`"2026-02-30"`, `"2026-04-31"`, a Feb 29 in a
/// non-leap year) — a malformed DOB must yield an honest "no age", never a confident-looking
/// number computed from a date that never happened (principle 4: this age is displayed
/// beside a patient's name on a wrong-chart-prevention surface). It deliberately does NOT
/// fill in a missing month/day either: a year-only DOB silently becoming "1 January" is the
/// same kind of precise untruth. `today` is a parameter so this stays pure and the edge owns
/// the clock.
pub fn age_years(birth_date: &str, today: &str) -> Option<u32> {
    let (by, bm, bd) = parse_ymd(birth_date)?;
    let (ty, tm, td) = parse_ymd(today)?;
    let mut years = ty - by;
    // Not yet had this year's birthday → one fewer whole year.
    if (tm, td) < (bm, bd) {
        years -= 1;
    }
    u32::try_from(years).ok()
}

/// One chart offered to the clerk before they may create a new one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    pub patient_id: Uuid,
    /// The §4.2 display-winner name, or the John Doe callsign.
    pub display_name: String,
    pub age: Option<Age>,
    pub trust: TrustState,
    /// ISO date of the chart's last activity, for "have I seen this person recently?".
    pub last_activity: Option<String>,
    /// A one-line locale hint, INTENDED to disambiguate two people with one name (suburb/
    /// town), not to display a dossier. NOT YET GUARANTEED to be suburb/town-only rather
    /// than a full address — the address data model has no culture-neutral locale-only
    /// facet to draw on today (issue #347); a caller must not assume this is short.
    pub locale: Option<String>,
    /// A content-addressed blob reference, NEVER bytes. Fetching the image is byte-tier
    /// work (ADR-0013) and must not sit on the search latency path.
    pub photo_ref: Option<String>,
}

/// The person rows plus what the node knows it could NOT show.
///
/// R3: the unit a clerk sees is a PERSON (a [`PersonRow`], one or more linked charts), not a
/// chart. What a registration attests to is still the flat list of CHART ids, via
/// [`CandidateList::displayed_charts`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateList {
    pub people: Vec<PersonRow>,
    /// True when the SEARCH was partial: the node could not read something it matched.
    /// ADR-0060 decision 2: partial completion is reported, never implied — a clerk must never
    /// believe an exhaustive search happened when it did not.
    ///
    /// NOT set because a prompt showed fewer rows than matched: that is display truncation,
    /// counted as `cairn_gui_funnel::PromptList::withheld` and never signed. Folding it back
    /// in set this flag on 92% of registrations, so it said nothing (ADR-0075 decision 3,
    /// restoring ADR-0061's meaning).
    pub incomplete: bool,
    /// Human-readable reason, shown beside the list. `Some` whenever `incomplete`.
    pub incomplete_reason: Option<String>,
}

impl CandidateList {
    /// A complete search that matched nothing.
    pub fn empty() -> CandidateList {
        CandidateList {
            people: Vec::new(),
            incomplete: false,
            incomplete_reason: None,
        }
    }

    /// Every chart of every row, in row order (members in their row order). The one flattening
    /// from rows back to charts; callers that mean "charts" use this, never `people`.
    pub fn charts(&self) -> impl Iterator<Item = &Candidate> {
        self.people.iter().flat_map(|row| row.members().iter())
    }

    /// The chart ids a registration attests it was shown — THE flattening
    /// `SearchAttestation::from_displayed` signs. ADR-0076 D6: `displayed` names every member of
    /// every row shown, so a linked row's charts are attested individually.
    pub fn displayed_charts(&self) -> Vec<Uuid> {
        self.charts().map(|c| c.patient_id).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn age_counts_whole_years_and_respects_the_birthday() {
        assert_eq!(age_years("1980-06-15", "2026-06-14"), Some(45));
        assert_eq!(age_years("1980-06-15", "2026-06-15"), Some(46));
        assert_eq!(age_years("1980-06-15", "2026-06-16"), Some(46));
    }

    #[test]
    fn a_partial_or_unparseable_dob_yields_no_age_rather_than_a_guess() {
        // Principle 4: an imprecise near-truth beats a precise untruth. A year-only DOB
        // must NOT silently become "assume 1 January".
        assert_eq!(age_years("1980", "2026-01-01"), None);
        assert_eq!(age_years("", "2026-01-01"), None);
        assert_eq!(age_years("not-a-date", "2026-01-01"), None);
    }

    #[test]
    fn a_future_birth_date_yields_no_age_rather_than_underflowing() {
        assert_eq!(age_years("2030-01-01", "2026-01-01"), None);
    }

    #[test]
    fn a_calendrically_impossible_date_yields_no_age_rather_than_a_confident_wrong_one() {
        // Task 2 review finding, carried into Task 5: the old range checks (1..=12,
        // 1..=31) accepted any day 1-31 for any month, so "2026-02-30" and "2026-04-31"
        // parsed as if they were real dates and produced a confident, wrong-looking age.
        // Principle 4: an imprecise near-truth (no age shown) beats a precise untruth (an
        // age computed from a date that never happened) — this age is displayed right next
        // to a patient's name on a wrong-chart-prevention surface, so a fabricated-looking
        // number here is not a cosmetic bug.
        assert_eq!(
            age_years("2026-02-30", "2026-06-01"),
            None,
            "February never has 30 days, leap year or not"
        );
        assert_eq!(
            age_years("2026-04-31", "2026-06-01"),
            None,
            "April has 30 days, never 31"
        );
    }

    #[test]
    fn a_leap_day_birth_date_is_honoured_only_in_a_leap_year() {
        // 2024 is a leap year (divisible by 4, not by 100) so Feb 29 is real; 2023 is not,
        // so the identical string is calendrically impossible and must yield no age at all
        // rather than silently rolling over to a nearby real date.
        assert_eq!(
            age_years("2024-02-29", "2026-03-01"),
            Some(2),
            "2024-02-29 is a real date (2024 is a leap year)"
        );
        assert_eq!(
            age_years("2023-02-29", "2026-03-01"),
            None,
            "2023-02-29 never happened (2023 is not a leap year)"
        );
    }

    #[test]
    fn an_incomplete_list_survives_a_serde_round_trip_with_its_reason_intact() {
        // Converted from a tautology (final review, minor): the old version asserted the two
        // fields it had just assigned in the line above and could not fail except by not
        // compiling. The round trip is the real contract — `CandidateList` crosses a process
        // boundary (the CLI renders it; the future picker window and native API will carry
        // it), and ADR-0060 decision 2 requires partial completion to be REPORTED, never
        // implied. A `#[serde(skip)]` or a renamed field on the reason would silently drop
        // the "why" and leave a bare `incomplete: true` a clerk cannot act on.
        let list = CandidateList {
            people: vec![],
            incomplete: true,
            incomplete_reason: Some("2 candidates could not be read".into()),
        };
        let round: CandidateList =
            serde_json::from_str(&serde_json::to_string(&list).unwrap()).unwrap();
        assert_eq!(
            round, list,
            "the reason must survive the wire, not just the flag"
        );
    }

    #[test]
    fn unknown_trust_is_a_token_and_round_trips() {
        // R3: a chart this node does not hold the registration of (synced ahead, or outside
        // sync scope) has no trust state to show. "unknown" says so honestly rather than
        // inventing "confirmed".
        assert_eq!(TrustState::Unknown.as_str(), "unknown");
        assert_eq!(
            serde_json::to_string(&TrustState::Unknown).unwrap(),
            "\"unknown\""
        );
        assert_eq!(
            serde_json::from_str::<TrustState>("\"unknown\"").unwrap(),
            TrustState::Unknown
        );
    }

    #[test]
    fn trust_states_render_the_tokens_the_chart_contract_uses() {
        // §5.7's projection-side contract. A picker must be able to show a John Doe
        // chart AS identity-pending — that chart is exactly the one a clerk needs when
        // the family arrives with a name.
        assert_eq!(TrustState::Confirmed.as_str(), "confirmed");
        assert_eq!(TrustState::Unconfirmed.as_str(), "unconfirmed");
        assert_eq!(TrustState::UnderReview.as_str(), "under-review");
        // And the SERDE spelling agrees with `as_str` on the one variant where kebab-case
        // and the default variant name actually differ — the whole point of the
        // `rename_all` on the enum.
        assert_eq!(
            serde_json::to_string(&TrustState::UnderReview).unwrap(),
            "\"under-review\"",
            "wire spelling and as_str must be the same token"
        );
        // And the READ side, pinned separately: the picker window and the native API will
        // both deserialize this vocabulary, and a change that broke only the read direction
        // would sail past a write-only assertion.
        assert_eq!(
            serde_json::from_str::<TrustState>("\"under-review\"").unwrap(),
            TrustState::UnderReview,
            "the wire token must read back as the variant that wrote it"
        );
    }

    #[test]
    fn a_candidate_survives_a_serde_round_trip_and_carries_a_photo_reference_never_bytes() {
        // Converted from a tautology (final review, minor): the old version read back the
        // `photo_ref` it had assigned two lines earlier. The round trip tests something that
        // can actually break — every §5.8 item-1 field must survive the wire, and
        // `photo_ref` must stay a REFERENCE (ADR-0013: fetching the image is byte-tier work
        // and must never sit on the search latency path), which is a property of the
        // SERIALIZED form, not of a struct literal.
        let c = Candidate {
            patient_id: Uuid::from_u128(1),
            display_name: "Smith, John".into(),
            age: Some(Age {
                years: 46,
                basis: "dob".into(),
            }),
            trust: TrustState::Confirmed,
            last_activity: Some("2026-08-01".into()),
            locale: Some("Bamaga QLD".into()),
            photo_ref: Some("b3:deadbeef".into()),
        };
        let json = serde_json::to_string(&c).unwrap();
        let round: Candidate = serde_json::from_str(&json).unwrap();
        assert_eq!(round, c, "every displayed field must survive the wire");
        // The digest travels; bytes never do. A future `photo: Vec<u8>` would have to add a
        // field here, and this assertion is what would notice.
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["photo_ref"], "b3:deadbeef");
        // The WIRE spelling of the trust vocabulary is the §5.7 token, same as `as_str` —
        // not serde's default variant name ("Confirmed"). One closed vocabulary, one
        // spelling, on every surface that carries it.
        assert_eq!(v["trust"], "confirmed");
        assert_eq!(
            v.as_object().unwrap().len(),
            7,
            "a new field on the search-latency path is a deliberate decision, not a drive-by \
             — if you added one, check it is not image bytes (ADR-0013): {json}"
        );
    }
}
