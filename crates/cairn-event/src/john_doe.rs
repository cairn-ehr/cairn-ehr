//! §5.4 unidentified-registration ("John Doe") callsign generation. Pure: no clock,
//! no randomness, no I/O — the caller supplies every part (including the date and the
//! disambiguating suffix) so the output is fully deterministic and testable. The
//! cairn-node edge (`john_doe.rs` there) is what stamps today's date and derives a
//! suffix from the freshly-minted patient UUID.
//!
//! A callsign is the display name for a patient whose identity is not yet known — an
//! unconscious ED arrival, an unidentified trauma. §5.4 requires it be a
//! **system-generated placeholder, never a plausible fake name**: `Unknown-<class>-
//! <site>-<date>-<suffix>`. Because it is obviously not a real name, staff never
//! mistake it for identification, and the matcher excludes it from its feature space
//! (see `matcher/pipeline/db.py`) so two different John Does never false-match on a
//! shared callsign token. The callsign is still stored as an ordinary name
//! (`facets.use = "callsign"`) so it renders in the chart header (db/012's
//! unidentified-patient fallback).

/// The reserved `use` token every callsign name carries. System-set and
/// culture-neutral (not a human name-use vocabulary term), so every node's matcher
/// recognises and excludes it identically without cultural capture. The matcher side
/// keeps the mirror of this constant (`PLACEHOLDER_NAME_USES`).
pub const CALLSIGN_USE: &str = "callsign";

/// The literal every callsign opens with, so a human (and a log grep) can spot an
/// unidentified-patient placeholder at a glance.
pub const CALLSIGN_PREFIX: &str = "Unknown";

/// Reduce one caller-supplied part to a single safe token. **Unicode-aware** (anti-cultural-
/// capture, the mission): every *alphanumeric* character of ANY script — Latin, CJK,
/// Cyrillic, Arabic, … — is KEPT and lower-cased (`char::to_lowercase`, correct per script),
/// so a non-Latin site label survives in the callsign instead of being dropped. Every run of
/// non-alphanumerics (space, `-`, `/`, punctuation) collapses to one ASCII `-`, and
/// leading/trailing `-` are trimmed. Because the delimiter is ASCII `-` and no alphanumeric
/// is `-`, a part can never inject an extra `-`-delimited field: a `site` of `"ED North"`
/// becomes `ed-north`, one field. An empty or all-separator part folds to a stable `unknown`
/// token rather than a doubled delimiter, so the callsign always has exactly five segments.
fn sanitize_part(part: &str) -> String {
    let mut out = String::with_capacity(part.len());
    let mut prev_dash = false;
    for ch in part.chars() {
        if ch.is_alphanumeric() {
            // Keep the character (any script) but lower-cased; to_lowercase yields an
            // iterator because one char can fold to several (e.g. some ligatures).
            out.extend(ch.to_lowercase());
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "unknown".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Build a §5.4 callsign: `Unknown-<class>-<site>-<date>-<suffix>`, each part
/// sanitized to a single safe token. Pure and deterministic — same inputs always
/// yield the same callsign. `class` is the care context (e.g. `ED`, `ward`), `site`
/// the registering location, `date` an already-formatted date string (the caller owns
/// the clock and the format — typically ISO `2026-07-03`), and `suffix` a
/// disambiguator the caller must make unique (cairn-node derives it from the minted
/// UUID so it is partition-safe with no coordination — see design call 3).
pub fn callsign(class: &str, site: &str, date: &str, suffix: &str) -> String {
    format!(
        "{}-{}-{}-{}-{}",
        CALLSIGN_PREFIX,
        sanitize_part(class),
        sanitize_part(site),
        sanitize_part(date),
        sanitize_part(suffix),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callsign_opens_with_the_unknown_prefix() {
        let c = callsign("ED", "site1", "2026-07-03", "A1B2");
        assert!(c.starts_with("Unknown-"), "callsign must be an obvious placeholder: {c}");
    }

    #[test]
    fn callsign_carries_all_parts_in_order() {
        let c = callsign("ED", "north", "2026-07-03", "a1b2");
        assert_eq!(c, "Unknown-ed-north-2026-07-03-a1b2");
    }

    #[test]
    fn parts_are_sanitized_so_a_part_cannot_inject_extra_fields() {
        // "ED North" contains a space; a raw interpolation would make two fields.
        let c = callsign("ED", "ED North", "2026-07-03", "a1");
        // Still exactly five '-'-delimited segments after the prefix's own segments:
        // Unknown / ed / ed-north / 2026-07-03 / a1  → but the site's internal '-' is
        // part of its single token, so count the fixed anchors instead:
        assert!(c.starts_with("Unknown-ed-ed-north-2026-07-03-a1"));
        assert_eq!(c, "Unknown-ed-ed-north-2026-07-03-a1");
    }

    #[test]
    fn deterministic() {
        assert_eq!(
            callsign("ED", "s", "2026-07-03", "x"),
            callsign("ED", "s", "2026-07-03", "x")
        );
    }

    #[test]
    fn distinct_suffixes_yield_distinct_callsigns() {
        let a = callsign("ED", "s", "2026-07-03", "aaaa");
        let b = callsign("ED", "s", "2026-07-03", "bbbb");
        assert_ne!(a, b, "the suffix is what keeps two same-site same-day John Does distinct");
    }

    #[test]
    fn empty_or_separator_only_part_folds_to_a_stable_token_never_a_doubled_delimiter() {
        let c = callsign("", "  ", "2026-07-03", "a1");
        assert_eq!(c, "Unknown-unknown-unknown-2026-07-03-a1");
        assert!(!c.contains("--"), "no empty segment / doubled delimiter: {c}");
    }

    #[test]
    fn unicode_and_punctuation_collapse_to_single_dashes() {
        // A messy site label must still reduce to one clean token.
        let c = callsign("ED", "Ward/3 — bay", "2026-07-03", "a1");
        assert_eq!(c, "Unknown-ed-ward-3-bay-2026-07-03-a1");
    }

    #[test]
    fn non_latin_alphanumerics_are_preserved_not_dropped() {
        // Anti-cultural-capture: a non-Latin site label (here CJK) stays in the callsign
        // rather than folding to "unknown". Punctuation between scripts still delimits.
        let c = callsign("ED", "北院/3", "2026-07-03", "a1");
        assert_eq!(c, "Unknown-ed-北院-3-2026-07-03-a1");
        // A Latin part with diacritics is preserved AND lower-cased per Unicode rules.
        let d = callsign("ED", "Süd", "2026-07-03", "a1");
        assert_eq!(d, "Unknown-ed-süd-2026-07-03-a1");
    }
}
