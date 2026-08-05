//! What the clerk typed, normalised into the keys the blocking passes use.
use serde::{Deserialize, Serialize};

/// A normalised search query. Culture-neutral by construction: no phonetics, no nickname
/// expansion, no name-order assumption. Locale-specific comparison is the advisory
/// matcher's job (ADR-0014), and baking one culture's name model in here would be exactly
/// the cultural capture that ADR forbids.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchQuery {
    /// Lower-cased name tokens from the typed name (see `new`'s doc for exactly which
    /// tokens land here). May be empty.
    pub name_tokens: Vec<String>,
    /// The birth date as typed, trimmed of surrounding whitespace, or `None` when the clerk
    /// gave none. Deliberately NOT day-precision-only: a reduced-precision ISO date
    /// (`YYYY` or `YYYY-MM`) is a first-class value here, because a registrar is frequently
    /// told only a year (principle 4 — never force a commitment nobody can vouch for), and
    /// `cairn_node::patient::register::dob_precision` derives the honest precision label
    /// from this same shape. db/046 pass 2 is an exact string compare, so a reduced-precision
    /// query matches a reduced-precision stored value and nothing else — narrower recall
    /// than a range search would give, never a wrong match.
    ///
    /// No calendar validation happens here or at the floor (both are deliberately parse-free
    /// and culture-neutral), so a shaped-but-impossible date such as `1980-13-45` survives
    /// into a signed attestation — tracked, not accepted.
    pub birth_date: Option<String>,
    /// `(system, value)` pairs.
    pub identifiers: Vec<(String, String)>,
}

impl SearchQuery {
    /// Normalise raw operator input into the tokens db/046 pass 3 blocks on.
    ///
    /// TWO KINDS OF TOKEN PER WHITESPACE-DELIMITED WORD, not a single split — this is the
    /// fix for a review-round Critical (#344): db/046 pass 3 tokenises the STORED name only
    /// on whitespace (`regexp_split_to_table(..., '\s+')`, copied verbatim from the
    /// matcher), so a hyphenated or apostrophe'd compound like "O'Brien-Smith" stays ONE
    /// stored token — deliberately, because the same rule is what keeps a dash-joined §5.4
    /// callsign ("unknown-ed-site1-...") from fragmenting into pieces that would match
    /// every John Doe ever registered. If this side split on every non-alphanumeric
    /// character (the old behaviour), a clerk typing a punctuated name — including typing
    /// it back EXACTLY as printed — would never produce a token equal to that intact stored
    /// one, and would silently fail to find the chart. So each word contributes:
    ///   1. the WHOLE word, only its leading/trailing punctuation trimmed (so it can match
    ///      an intact stored token like "o'brien-smith" or a callsign), and
    ///   2. its alphanumeric PARTS (so "O'Brien-Smith, John" still finds a chart stored as
    ///      separate words, and a clerk typing just "Brien" still gets a hit on a name
    ///      stored as plain, unpunctuated words).
    ///
    /// Pass 3 is a disjunction (UNION), so the extra part-tokens can only ADD advisory
    /// candidates, never remove one — a missed candidate is the dangerous direction here,
    /// an extra one is merely something a clerk dismisses. Sorted + deduplicated because
    /// this list is carried, verbatim, into a permanent signed registration attestation
    /// (db/045) — no duplicate tokens belong in that record.
    pub fn new(raw_name: &str, birth_date: Option<&str>, identifiers: &[(String, String)]) -> Self {
        let mut name_tokens: Vec<String> = raw_name
            .split_whitespace()
            .flat_map(|word| {
                // The whole whitespace-delimited word, edge punctuation trimmed only —
                // this IS the token pass 3 stores for a punctuated compound or a callsign.
                let whole = word
                    .trim_matches(|c: char| !c.is_alphanumeric())
                    .to_lowercase();
                // Its alphanumeric parts, so a clerk typing a bare surname fragment still
                // finds a chart stored as separate plain words. Single characters ("o" out
                // of "O'Brien") are dropped as noise: they cannot narrow a search and would
                // only inflate the advisory candidate set.
                let parts: Vec<String> = word
                    .split(|c: char| !c.is_alphanumeric())
                    .filter(|p| p.chars().count() > 1)
                    .map(str::to_lowercase)
                    .collect();
                std::iter::once(whole).chain(parts)
            })
            .filter(|t| !t.is_empty())
            .collect();
        name_tokens.sort();
        name_tokens.dedup();
        Self {
            name_tokens,
            // Trimmed once and the TRIMMED string is what's stored (not merely used to
            // decide non-emptiness): db/046 pass 2 is an exact string compare against the
            // projected value, so a clerk's stray leading/trailing space (e.g. pasted from
            // a form field) must not silently defeat it.
            birth_date: birth_date
                .map(str::trim)
                .filter(|d| !d.is_empty())
                .map(str::to_string),
            identifiers: identifiers.to_vec(),
        }
    }

    /// True when there is nothing to search on. The db/045 floor refuses a registration
    /// whose attested query is empty: "I searched for nothing and found nothing" is not a
    /// search, and must not be allowed to satisfy the funnel.
    pub fn is_empty(&self) -> bool {
        self.name_tokens.is_empty() && self.birth_date.is_none() && self.identifiers.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_becomes_lowercase_tokens_sorted_and_deduplicated() {
        let q = SearchQuery::new("O'Brien-Smith,  John", None, &[]);
        // "O'Brien-Smith," contributes its WHOLE trimmed form ("o'brien-smith") plus its
        // parts ("brien", "smith" — "o" is dropped as single-character noise); "John"
        // contributes "john" as both its whole form and its (only) part, collapsed by dedup.
        assert_eq!(
            q.name_tokens,
            vec!["brien", "john", "o'brien-smith", "smith"]
        );
    }

    #[test]
    fn a_punctuated_compound_word_is_kept_whole_as_well_as_split_into_parts() {
        // The Critical fix (#344): a clerk typing a name back EXACTLY as printed — the
        // "standard narrowing gesture" of searching by surname alone — must produce a
        // token equal to the INTACT stored token db/046 pass 3 tokenises on whitespace
        // only. The old whole-string split fragmented "O'Brien-Smith" into pieces that
        // never equalled the stored "o'brien-smith" token, so this exact search failed.
        let q = SearchQuery::new("O'Brien-Smith", None, &[]);
        assert_eq!(q.name_tokens, vec!["brien", "o'brien-smith", "smith"]);
    }

    #[test]
    fn a_dash_joined_callsign_with_no_whitespace_becomes_one_whole_token() {
        // A §5.4 John Doe callsign ("unknown-ed-site1-2026-07-03-00ab") is one
        // whitespace-delimited word, so its WHOLE form is the single token that matches
        // the stored callsign row exactly. Its dash-separated PARTS are also emitted
        // (advisory-only extra recall — pass 3 is a disjunction, so this can only add
        // candidates), but the whole-token match is what makes the callsign findable at
        // all, not an accident of the parts happening to line up.
        let q = SearchQuery::new("Unknown-ED-site1-2026-07-03-00ab", None, &[]);
        assert!(
            q.name_tokens
                .contains(&"unknown-ed-site1-2026-07-03-00ab".to_string()),
            "the whole callsign must survive as one token: {:?}",
            q.name_tokens
        );
    }

    #[test]
    fn a_birth_date_with_surrounding_whitespace_is_trimmed_not_merely_checked() {
        // db/046 pass 2 is an exact string compare against the projected value — the old
        // code trimmed only to decide "is this blank?" and then stored the UNTRIMMED
        // string, so a stray leading/trailing space (e.g. pasted from a form field) would
        // silently defeat the exact match.
        let q = SearchQuery::new("", Some("  1980-06-15  "), &[]);
        assert_eq!(q.birth_date.as_deref(), Some("1980-06-15"));
    }

    #[test]
    fn a_query_with_only_an_identifier_is_not_empty() {
        let q = SearchQuery::new("", None, &[("MRN".into(), "12345".into())]);
        assert!(q.name_tokens.is_empty());
        assert!(!q.is_empty(), "an identifier alone is a real search");
    }

    #[test]
    fn a_query_with_nothing_in_it_is_empty() {
        assert!(SearchQuery::new("   ", None, &[]).is_empty());
    }

    #[test]
    fn a_dob_alone_is_a_real_search() {
        assert!(!SearchQuery::new("", Some("1980-01-01"), &[]).is_empty());
    }
}
