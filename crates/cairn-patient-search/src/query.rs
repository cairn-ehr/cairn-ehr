//! What the clerk typed, normalised into the keys the blocking passes use.
use serde::{Deserialize, Serialize};

/// A normalised search query. Culture-neutral by construction: it tokenises on
/// non-alphanumerics and lower-cases, and does NOTHING else — no phonetics, no nickname
/// expansion, no name-order assumption. Locale-specific comparison is the advisory
/// matcher's job (ADR-0014), and baking one culture's name model in here would be exactly
/// the cultural capture that ADR forbids.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchQuery {
    /// Lower-cased alphanumeric tokens from the typed name. May be empty.
    pub name_tokens: Vec<String>,
    /// ISO `YYYY-MM-DD` as typed, or `None`.
    pub birth_date: Option<String>,
    /// `(system, value)` pairs.
    pub identifiers: Vec<(String, String)>,
}

impl SearchQuery {
    /// Normalise raw operator input. `raw_name` is split on any non-alphanumeric run, so
    /// "O'Brien-Smith, John" yields four tokens; each blocking pass then matches on tokens
    /// rather than on a whole string, which is what lets a name typed in a different order
    /// still find the chart.
    pub fn new(raw_name: &str, birth_date: Option<&str>, identifiers: &[(String, String)]) -> Self {
        let name_tokens = raw_name
            .split(|c: char| !c.is_alphanumeric())
            .filter(|t| !t.is_empty())
            .map(|t| t.to_lowercase())
            .collect();
        Self {
            name_tokens,
            birth_date: birth_date
                .map(str::to_string)
                .filter(|d| !d.trim().is_empty()),
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
    fn a_name_becomes_lowercase_tokens_with_punctuation_dropped() {
        let q = SearchQuery::new("O'Brien-Smith,  John", None, &[]);
        assert_eq!(q.name_tokens, vec!["o", "brien", "smith", "john"]);
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
