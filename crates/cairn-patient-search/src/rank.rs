//! The ORDER of a candidate list — which five the step-3 prompt shows.
//!
//! # Why ranking matters, and why it is only ever an order
//!
//! `db/046` is a disjunction (any name token OR the exact DOB OR an identifier), so a
//! registration search returns ~100 candidates and the prompt shows five (ADR-0075). The
//! prompt is a best-effort nudge, not a completeness claim, so the one lever that helps
//! without asking the person at the desk for anything is putting the likeliest duplicate
//! first. This module only REORDERS: it never adds or drops a candidate, so the search's
//! set — and the drift invariant *sweep-paired ⊆ search-found* — is untouched.
//!
//! # The keys, strongest first
//!
//! 1. `passes` — how many of db/046's passes matched (identifier / DOB / name).
//! 2. `tokens_matched` — how many DISTINCT query name tokens the chart's retained names
//!    contain. db/046's name pass counts ONCE however many tokens matched, so without this
//!    key "John Brown" ties "John Smith" for a "John Smith" query.
//! 3. `dob_near_miss` — the chart's DOB is the query's with a typical slip (day/month
//!    swapped, year ±1, last two year digits transposed). This is what lifts a duplicate
//!    typed with a WRONG date of birth, which matches the name pass alone.
//! 4. `id` ascending — UUIDv7, so chart age: a stable, deterministic final tie-break.
//!
//! # Stated limit
//!
//! Keys 2 and 3 are computed here, in Rust, over names Postgres has normalised. They may
//! drift from db/046's own SQL expression. That can only worsen the ORDER, never lose a
//! candidate — which is why it is stated rather than pinned by a cross-language twin.
use crate::candidate::parse_ymd;
use crate::query::name_tokens;
use std::collections::HashSet;
use uuid::Uuid;

/// Everything the order is decided on, for one candidate. Built by the caller (the node's
/// `search_patients`) from its reads; nothing here touches a database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RankKey {
    pub id: Uuid,
    /// Distinct db/046 passes this chart matched (1..=3).
    pub passes: u32,
    /// See [`tokens_matched`].
    pub tokens_matched: usize,
    /// See [`is_dob_near_miss`].
    pub dob_near_miss: bool,
}

/// How many DISTINCT `query_tokens` appear among the tokens of ANY of `stored_names`.
///
/// `stored_names` are the chart's retained names, already lowercased and NFC-normalised by
/// Postgres; each is tokenised by [`name_tokens`] — the SAME rule the query was — so a
/// punctuated compound matches by its whole form and by its parts alike.
pub fn tokens_matched(query_tokens: &[String], stored_names: &[String]) -> usize {
    let stored: HashSet<String> = stored_names.iter().flat_map(|n| name_tokens(n)).collect();
    let distinct_query: HashSet<&String> = query_tokens.iter().collect();
    distinct_query
        .into_iter()
        .filter(|t| stored.contains(*t))
        .count()
}

/// True when `candidate` is `query` with one of the commonest date-of-birth slips.
///
/// Both must be full, real ISO dates ([`parse_ymd`]); a partial-precision date is an honest
/// "only the year is known" (principle 4), not a slip, and never counts. An EXACT match is
/// not a near-miss — db/046's DOB pass already rewards it through `passes`.
pub fn is_dob_near_miss(query: &str, candidate: &str) -> bool {
    let (Some(q), Some(c)) = (parse_ymd(query), parse_ymd(candidate)) else {
        return false;
    };
    if q == c {
        return false;
    }
    let (qy, qm, qd) = q;
    let (cy, cm, cd) = c;
    let same_day_and_month = cm == qm && cd == qd;
    let day_month_swapped = cy == qy && cm == qd && cd == qm;
    let year_off_by_one = same_day_and_month && (cy - qy).abs() == 1;
    let year_digits_transposed = same_day_and_month && last_two_digits_transposed(qy, cy);
    day_month_swapped || year_off_by_one || year_digits_transposed
}

/// 1967 ↔ 1976: same century, the last two digits swapped. Different years only (a
/// palindromic pair like 1977 ↔ 1977 is excluded by the `a != b` check).
fn last_two_digits_transposed(a: i32, b: i32) -> bool {
    a != b && a / 100 == b / 100 && (a % 100) / 10 == b % 10 && a % 10 == (b % 100) / 10
}

/// Order candidates strongest-first by the four keys in the module doc; return their ids.
pub fn rank_candidates(mut keys: Vec<RankKey>) -> Vec<Uuid> {
    keys.sort_by(|a, b| {
        b.passes
            .cmp(&a.passes)
            .then(b.tokens_matched.cmp(&a.tokens_matched))
            // `bool` orders false < true, so comparing b to a puts a near-miss first.
            .then(b.dob_near_miss.cmp(&a.dob_near_miss))
            .then(a.id.cmp(&b.id))
    });
    keys.into_iter().map(|k| k.id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }
    fn key(n: u128, passes: u32, tokens_matched: usize, dob_near_miss: bool) -> RankKey {
        RankKey {
            id: Uuid::from_u128(n),
            passes,
            tokens_matched,
            dob_near_miss,
        }
    }

    #[test]
    fn a_two_token_match_counts_two() {
        assert_eq!(
            tokens_matched(&s(&["john", "smith"]), &s(&["john smith"])),
            2
        );
        assert_eq!(
            tokens_matched(&s(&["john", "smith"]), &s(&["john brown"])),
            1
        );
    }

    #[test]
    fn tokens_are_matched_across_every_retained_name() {
        // A chart known as "Jane Doe" and later "Jane Smith": both names are evidence.
        assert_eq!(
            tokens_matched(&s(&["jane", "smith"]), &s(&["jane doe", "jane smith"])),
            2
        );
    }

    #[test]
    fn repeated_query_tokens_count_once() {
        assert_eq!(
            tokens_matched(&s(&["john", "john"]), &s(&["john smith"])),
            1
        );
    }

    #[test]
    fn a_candidate_with_no_stored_names_matches_no_tokens() {
        assert_eq!(tokens_matched(&s(&["john"]), &[]), 0);
    }

    #[test]
    fn stored_names_are_tokenised_by_the_query_rule() {
        // "o'brien-smith" stored whole must match the query's whole token AND its parts.
        let q = crate::query::name_tokens("O'Brien-Smith");
        assert_eq!(tokens_matched(&q, &s(&["o'brien-smith ann"])), q.len());
    }

    #[test]
    fn day_and_month_swapped_is_a_near_miss() {
        assert!(is_dob_near_miss("1980-03-07", "1980-07-03"));
    }

    #[test]
    fn a_year_off_by_one_is_a_near_miss_either_way() {
        assert!(is_dob_near_miss("1980-03-07", "1981-03-07"));
        assert!(is_dob_near_miss("1980-03-07", "1979-03-07"));
    }

    #[test]
    fn transposed_last_two_year_digits_is_a_near_miss() {
        assert!(is_dob_near_miss("1967-05-20", "1976-05-20"));
        assert!(!is_dob_near_miss("1967-05-20", "1977-05-20"));
    }

    #[test]
    fn an_exact_dob_is_not_a_near_miss() {
        // The exact match is already rewarded by db/046's DOB pass (`passes`).
        assert!(!is_dob_near_miss("1980-03-07", "1980-03-07"));
    }

    #[test]
    fn a_partial_date_is_never_a_near_miss() {
        assert!(!is_dob_near_miss("1980", "1981"));
        assert!(!is_dob_near_miss("1980-03", "1980-03-07"));
    }

    #[test]
    fn an_impossible_date_is_never_a_near_miss() {
        assert!(!is_dob_near_miss("1980-02-30", "1980-30-02"));
        assert!(!is_dob_near_miss("1980-13-01", "1980-01-13"));
    }

    #[test]
    fn an_unrelated_date_is_not_a_near_miss() {
        assert!(!is_dob_near_miss("1980-03-07", "1955-11-21"));
        assert!(
            !is_dob_near_miss("1980-03-07", "1982-03-07"),
            "two years is not a slip"
        );
    }

    #[test]
    fn more_passes_rank_first() {
        let r = rank_candidates(vec![key(1, 1, 2, true), key(2, 2, 0, false)]);
        assert_eq!(r, vec![Uuid::from_u128(2), Uuid::from_u128(1)]);
    }

    #[test]
    fn within_equal_passes_more_tokens_rank_first() {
        let r = rank_candidates(vec![key(1, 1, 1, false), key(2, 1, 2, false)]);
        assert_eq!(r, vec![Uuid::from_u128(2), Uuid::from_u128(1)]);
    }

    #[test]
    fn within_equal_tokens_a_dob_near_miss_ranks_first() {
        let r = rank_candidates(vec![key(1, 1, 2, false), key(2, 1, 2, true)]);
        assert_eq!(r, vec![Uuid::from_u128(2), Uuid::from_u128(1)]);
    }

    #[test]
    fn full_ties_keep_chart_age_order() {
        let r = rank_candidates(vec![key(20, 1, 1, false), key(10, 1, 1, false)]);
        assert_eq!(r, vec![Uuid::from_u128(10), Uuid::from_u128(20)]);
    }

    #[test]
    fn ranking_never_adds_or_drops_a_candidate() {
        let keys = vec![key(3, 1, 0, false), key(1, 2, 1, true), key(2, 1, 2, false)];
        let mut r = rank_candidates(keys);
        r.sort();
        assert_eq!(
            r,
            vec![Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3)]
        );
    }
}
