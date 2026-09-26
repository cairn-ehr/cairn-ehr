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
//! 2. `identifier_matched` — the identifier pass is among them. An identifier is the one
//!    near-unique key a clerk types; a chart found ONLY by it (a nickname plus a married
//!    surname) shares no name token, so without this key it sank below every one-token
//!    namesake and was cut from the prompt (review of #678).
//! 3. `tokens_matched` — how many DISTINCT query name tokens match the chart's retained names,
//!    exactly or as a typed prefix, as db/046 matches them. db/046's name pass counts ONCE
//!    however many tokens matched, so without this key "John Brown" ties "John Smith" for a
//!    "John Smith" query.
//! 4. `dob_near_miss` — the chart's DOB is the query's with a typical slip (day/month
//!    swapped, year ±1, last two year digits transposed). Measured, the name-token key does
//!    most of the lifting for a duplicate typed with a wrong DOB; this key decides it once a
//!    name token is lost too (a surname typo AND a DOB slip).
//! 5. `tokens_exact` — how many of key 3's tokens matched EXACTLY rather than as a prefix. A
//!    tie-break only: without it every prefix-only candidate ("Annabel" for a typed "Ann") tied
//!    a duplicate found by its exact given name, and chart age decided (measured, PR #678).
//! 6. `id` ascending — UUIDv7, so chart age: a stable, deterministic final tie-break.
//!
//! # Stated limit
//!
//! Keys 3 and 4 are computed here, in Rust, over names Postgres has normalised. Key 3 mirrors
//! db/046's name pass — exact OR a prefix of at least 3 bytes, callsigns never split — but
//! tokenises in Rust, so it may drift from db/046's own SQL expression. That can only worsen the ORDER, never lose a
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
    /// True when db/046's IDENTIFIER pass is among the passes this chart matched.
    pub identifier_matched: bool,
    /// See [`tokens_matched`].
    pub tokens_matched: usize,
    /// See [`is_dob_near_miss`].
    pub dob_near_miss: bool,
    /// See [`tokens_exactly_matched`]: how many of `tokens_matched` were EXACT, not a prefix.
    pub tokens_exact: usize,
}

/// How many DISTINCT plain `query_tokens` match a token of ANY of `stored_names`.
///
/// `stored_names` are the chart's retained names, already lowercased and NFC-normalised by
/// Postgres, with §5.4 callsigns already left out by the caller (see "Mirrors db/046" below);
/// each is tokenised by [`name_tokens`] — the SAME rule the query was.
///
/// **A query token matches a stored token the two ways db/046's name pass does:** it EQUALS
/// it, or — when the query token is at least [`MIN_PREFIX_BYTES`] bytes long — the stored
/// token STARTS WITH it. The prefix arm is #636's "a clerk types a fragment": "Alex" finds
/// "Alexander", so it must also count toward how strongly "Alexander" matched, or the
/// duplicate a clerk found by typing a short first name ties every namesake (review of #678).
///
/// **Each query WORD counts once, by its parts where they stand for it.** [`name_tokens`] emits a
/// punctuated word three ways — "mary-jane" plus "mary" and "jane" — so counting every token
/// let a hyphenated given name score three against a surname's one, and "Mary-Jane Brown"
/// tied the real "Mary Jane Smith" duplicate (final review, #671). So a punctuated whole form
/// is skipped WHEN ONE OF ITS PARTS IS ALSO A QUERY TOKEN: the parts carry the match, and both
/// sides emit them. See `is_represented_by_its_parts` for why the test is "a part is in the
/// query" and not "every character is alphanumeric".
///
/// # Mirrors db/046, callsigns included
///
/// db/046 never splits a callsign ("unknown-ed-site1-…") into parts, and refuses it the prefix
/// arm, so a clerk typing "Ed" does not match every John Doe on the node. A callsign is always
/// dash-joined, so even typed in full its whole form is represented by its parts here, and the
/// parts are exactly what db/046 refuses: it contributes nothing to this count. The caller
/// therefore leaves callsigns out of `stored_names` rather than this function carrying a `use`
/// flag it would only ever use to skip them.
pub fn tokens_matched(query_tokens: &[String], stored_names: &[String]) -> usize {
    count_matched(query_tokens, stored_names, token_matches)
}

/// Like [`tokens_matched`], but counting EXACT matches only — the tie-break under it.
///
/// Why a tie-break, not a replacement: a prefix match is still evidence (it is how db/046 found
/// the chart), but "Ann" typed is a closer match to a stored "Ann" than to "Annabel". Counting
/// prefixes alone lifted every prefix-only candidate into the tier of a duplicate found by its
/// exact given name, and chart age then decided the tie (measured, PR #678).
pub fn tokens_exactly_matched(query_tokens: &[String], stored_names: &[String]) -> usize {
    count_matched(query_tokens, stored_names, |q, s| q == s)
}

/// The DISTINCT plain query tokens for which `matches` holds against some stored token. The one
/// body behind [`tokens_matched`] and [`tokens_exactly_matched`], so they cannot disagree about
/// what a plain token is or how a stored name is tokenised.
fn count_matched(
    query_tokens: &[String],
    stored_names: &[String],
    matches: impl Fn(&str, &str) -> bool,
) -> usize {
    let stored: HashSet<String> = stored_names.iter().flat_map(|n| name_tokens(n)).collect();
    let query: HashSet<&str> = query_tokens.iter().map(String::as_str).collect();
    query
        .iter()
        .filter(|t| !is_represented_by_its_parts(t, &query))
        .filter(|t| stored.iter().any(|s| matches(t, s)))
        .count()
}

/// True when `token` is a punctuated whole form ("mary-jane") one of whose alphanumeric parts is
/// ALSO in `query` — so the parts are counted and the whole must not be counted again.
///
/// Why not simply "count only all-alphanumeric tokens" (the #671 rule this replaces): a word can
/// be non-alphanumeric without its parts standing for it, and that rule scored such a word
/// ZERO. Rust lowercases Turkish "İ" to "i" + U+0307, a combining mark, and `name_tokens` split
/// the word BEFORE lowercasing, so "i̇nce" has no part in the query at all; a Thai tone mark or
/// a Devanagari virama, and initials like "J-P", split a word into single characters, which
/// `name_tokens` drops. Asking "is a part in the query?" needs no table of Unicode categories and
/// cannot drift from `name_tokens`, which made the parts in the first place (review of #678).
fn is_represented_by_its_parts(token: &str, query: &HashSet<&str>) -> bool {
    token
        .split(|c: char| !c.is_alphanumeric())
        .any(|part| part != token && query.contains(part))
}

/// db/046's prefix gate, in BYTES (its `octet_length(q.qt) >= 3`, #638): UTF-8 spends 3 bytes
/// on an ideograph and 1 on a Latin letter, so "李小" (6) is admitted and "al" (2) is not.
pub const MIN_PREFIX_BYTES: usize = 3;

/// One query token against one stored token: equal, or a long-enough prefix (db/046 pass 3).
fn token_matches(query_token: &str, stored_token: &str) -> bool {
    query_token == stored_token
        || (query_token.len() >= MIN_PREFIX_BYTES && stored_token.starts_with(query_token))
}

/// True when `candidate` is `query` with one of the commonest date-of-birth slips.
///
/// Both must be full, real ISO dates (`candidate::parse_ymd`, after trimming); a
/// partial-precision date is an honest "only the year is known" (principle 4), not a slip, and
/// never counts.
///
/// An IDENTICAL STRING is not a near-miss — db/046's DOB pass already rewards it through
/// `passes`. But that pass is an exact STRING compare and nothing on the write path pins the
/// format, so the SAME date written differently ("1980-3-7", a stray space) is missed by it.
/// That is the strongest slip of all, so it counts here (review of #678).
pub fn is_dob_near_miss(query: &str, candidate: &str) -> bool {
    if query == candidate {
        return false;
    }
    let (Some(q), Some(c)) = (parse_ymd(query.trim()), parse_ymd(candidate.trim())) else {
        return false;
    };
    if q == c {
        return true;
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
            // `bool` orders false < true, so comparing b to a puts an identifier match first.
            .then(b.identifier_matched.cmp(&a.identifier_matched))
            .then(b.tokens_matched.cmp(&a.tokens_matched))
            // `bool` orders false < true, so comparing b to a puts a near-miss first.
            .then(b.dob_near_miss.cmp(&a.dob_near_miss))
            .then(b.tokens_exact.cmp(&a.tokens_exact))
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
            identifier_matched: false,
            tokens_matched,
            dob_near_miss,
            tokens_exact: tokens_matched,
        }
    }
    fn identifier_key(n: u128, passes: u32, tokens_matched: usize) -> RankKey {
        RankKey {
            identifier_matched: true,
            ..key(n, passes, tokens_matched, false)
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
        // A punctuated compound stored whole still matches by its parts: "brien" and "smith".
        let q = crate::query::name_tokens("O'Brien-Smith");
        assert_eq!(tokens_matched(&q, &s(&["o'brien-smith ann"])), 2);
    }

    /// Final review #2: a punctuated query word yields its whole form AND its parts, so counting
    /// every token let a hyphenated GIVEN name outweigh a matched surname — "Mary-Jane Brown"
    /// tied the real "Mary Jane Smith" duplicate. Only plain (all-alphanumeric) tokens count;
    /// both sides emit a punctuated word's parts, so nothing a whole form matched is lost.
    #[test]
    fn a_hyphenated_given_name_does_not_outweigh_a_matched_surname() {
        let q = crate::query::name_tokens("Mary-Jane Smith");
        let duplicate = tokens_matched(&q, &s(&["mary jane smith"]));
        let other = tokens_matched(&q, &s(&["mary-jane brown"]));
        assert!(duplicate > other, "duplicate {duplicate} must beat {other}");
    }

    /// Review of #678: db/046's name pass also matches a typed PREFIX of a stored token (#636 —
    /// "a clerk types a fragment"). Counting only exact tokens scored the "Alex" of an
    /// "Alexander Nguyen" duplicate zero, tying it with every other Nguyen.
    #[test]
    fn a_typed_prefix_of_a_stored_token_counts() {
        assert_eq!(
            tokens_matched(&s(&["alex", "nguyen"]), &s(&["alexander nguyen"])),
            2
        );
    }

    /// The prefix counts under db/046's own gate — at least 3 BYTES, not characters (#638) — so
    /// a Latin two-letter fragment does not count, and a two-character Han prefix does.
    #[test]
    fn a_prefix_counts_only_from_three_bytes() {
        assert_eq!(
            tokens_matched(&s(&["al", "nguyen"]), &s(&["alexander nguyen"])),
            1
        );
        assert_eq!(tokens_matched(&s(&["李小"]), &s(&["李小明"])), 1);
    }

    #[test]
    fn exact_tokens_do_not_count_a_prefix() {
        assert_eq!(
            tokens_exactly_matched(&s(&["alex", "nguyen"]), &s(&["alexander nguyen"])),
            1
        );
    }

    /// Measured on the PR #678 fix: counting prefixes lifted every PREFIX-only candidate (typed
    /// "Ann", stored "Annabel") into the tier of a duplicate found by its exact given name, and
    /// chart age then broke the tie — a surname-typo-plus-wrong-DOB duplicate fell 204 -> 183 of
    /// 500. Within equal tokens and near-miss, exactly-matched tokens break the tie.
    #[test]
    fn within_equal_tokens_an_exact_match_outranks_a_prefix_match() {
        let prefix_only = RankKey {
            tokens_exact: 0,
            ..key(1, 1, 1, false)
        };
        let r = rank_candidates(vec![prefix_only, key(2, 1, 1, false)]);
        assert_eq!(r, vec![Uuid::from_u128(2), Uuid::from_u128(1)]);
    }

    #[test]
    fn a_dob_near_miss_outweighs_exact_over_prefix() {
        let prefix_with_near_miss = RankKey {
            tokens_exact: 1,
            ..key(1, 1, 2, true)
        };
        let r = rank_candidates(vec![key(2, 1, 2, false), prefix_with_near_miss]);
        assert_eq!(r, vec![Uuid::from_u128(1), Uuid::from_u128(2)]);
    }

    #[test]
    fn a_short_token_still_counts_by_exact_match() {
        // The 3-byte gate is on the PREFIX arm only, exactly as in db/046: "Wu" stays countable.
        assert_eq!(tokens_matched(&s(&["wu", "li"]), &s(&["wu li"])), 2);
    }

    /// Review of #678: "only plain tokens count" silently scored ZERO for a word whose parts all
    /// fail to be plain tokens themselves. Rust lowercases Turkish "İ" to "i" + U+0307 (a
    /// combining mark, not alphanumeric); a Thai tone mark or a Devanagari virama splits a word
    /// into single characters, which `name_tokens` drops; so do initials like "J-P". Each word
    /// must still count once.
    #[test]
    fn a_word_whose_parts_do_not_stand_for_it_counts_whole() {
        let q = crate::query::name_tokens("İnce Yılmaz");
        assert_eq!(tokens_matched(&q, &s(&["i\u{307}nce yılmaz"])), 2, "{q:?}");
        let q = crate::query::name_tokens("ก่อ สมชาย");
        assert_eq!(tokens_matched(&q, &s(&["ก่อ สมชาย"])), 2, "{q:?}");
        let q = crate::query::name_tokens("J-P Smith");
        assert_eq!(tokens_matched(&q, &s(&["j-p smith"])), 2, "{q:?}");
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

    /// Review of #678: db/046's DOB pass is an EXACT STRING compare, and nothing on the write path
    /// pins the format, so a chart stored as "1980-3-7" is missed by it when "1980-03-07" is typed.
    /// Skipping every "parses equal" pair as "already rewarded" then gave that duplicate no DOB
    /// credit at all. Only an identical STRING is already rewarded.
    #[test]
    fn the_same_date_written_differently_is_a_near_miss() {
        assert!(is_dob_near_miss("1980-03-07", "1980-3-7"));
        assert!(is_dob_near_miss("1980-03-07", "1980-03-07 "));
    }

    #[test]
    fn near_miss_edges_across_centuries_and_double_slips() {
        assert!(
            is_dob_near_miss("1999-05-20", "2000-05-20"),
            "year +1 across a century"
        );
        assert!(
            is_dob_near_miss("2001-05-20", "2010-05-20"),
            "transposed in the 2000s"
        );
        assert!(
            !is_dob_near_miss("1980-03-07", "1981-07-03"),
            "a swap AND a year slip is two slips, not one"
        );
        assert!(
            !is_dob_near_miss("1980-03-03", "1980-03-03"),
            "day == month: the swap is the same string"
        );
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

    /// Review of #678: an identifier is the one near-unique key a clerk types. A chart found
    /// ONLY by it ("Peggy Jones" for a "Margaret Smith" query: nickname and married surname)
    /// shares no name token, so ranking by tokens alone put it below every one-token namesake
    /// and cut it from the prompt. Within equal passes, the identifier match comes first.
    #[test]
    fn within_equal_passes_an_identifier_match_ranks_first() {
        let r = rank_candidates(vec![key(1, 1, 2, true), identifier_key(2, 1, 0)]);
        assert_eq!(r, vec![Uuid::from_u128(2), Uuid::from_u128(1)]);
    }

    #[test]
    fn more_passes_still_outrank_an_identifier_match() {
        let r = rank_candidates(vec![identifier_key(1, 1, 0), key(2, 2, 2, false)]);
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
