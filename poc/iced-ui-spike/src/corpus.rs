//! The multi-script name corpus for Spike 0004 claim **I** (complex text).
//!
//! Why this module exists: Cairn is culture-neutral — a patient's name in any
//! script must render and be editable (ADR-0014, §5.13). The spike must shape
//! names in scripts that exercise the hard cases: a right-to-left script
//! (Arabic), a complex-clustering Brahmic script (Devanagari), and an ideographic
//! script that needs IME input (Han). This module is the *single source of truth*
//! for those samples plus the expectations a shaper must satisfy, so both the
//! headless shaping check ([`crate::shaping`], `--features shaping`) and the GUI
//! name fields draw from one list.
//!
//! It has **zero dependencies** — the corpus and its structural expectations are
//! plain data, unit-tested without a font or a shaper — so claim **I**'s
//! *structure* is verified in CI even where no Noto fonts are installed.

/// Writing direction of a sample. Bidi is the canonical EHR failure mode:
/// an Arabic given name beside a Latin family name must lay out correctly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Left-to-right (Latin, Devanagari, Han).
    Ltr,
    /// Right-to-left (Arabic, Hebrew, …).
    Rtl,
}

/// One name sample plus what a correct shaper must produce for it.
#[derive(Debug, Clone)]
pub struct Sample {
    /// Human label for results/operator screenshots (e.g. "Arabic (RTL)").
    pub label: &'static str,
    /// ISO 15924-style script tag, for results tabulation.
    pub script: &'static str,
    /// The name text itself.
    pub text: &'static str,
    /// Expected writing direction.
    pub direction: Direction,
    /// Lower bound on the number of shaped, non-`.notdef` glyph clusters a
    /// correct shaping must yield. A conservative floor (not an exact count)
    /// because shaping legitimately merges/forms clusters — the point is
    /// "more than zero real glyphs, no tofu", not a brittle exact match.
    pub min_clusters: usize,
}

/// The corpus: one realistic name per hard-case script, plus a Latin baseline.
///
/// The texts are ordinary given/family names, not lorem ipsum — the spike is
/// about *names*, the thing Cairn must never mangle. Kept as a function (not a
/// const) so it is easy to extend without `const fn` friction.
pub fn corpus() -> Vec<Sample> {
    vec![
        Sample {
            label: "Latin (baseline)",
            script: "Latn",
            text: "Ngọc Anh Trần", // Latin + combining diacritics (Vietnamese)
            direction: Direction::Ltr,
            min_clusters: 8,
        },
        Sample {
            label: "Arabic (RTL)",
            script: "Arab",
            text: "محمد عبد الله", // "Muhammad Abdullah" — cursive joining + RTL
            direction: Direction::Rtl,
            min_clusters: 8,
        },
        Sample {
            label: "Devanagari (complex)",
            script: "Deva",
            text: "प्रिया शर्मा", // "Priya Sharma" — conjuncts + matras
            direction: Direction::Ltr,
            min_clusters: 6,
        },
        Sample {
            label: "Han (CJK + IME)",
            script: "Hani",
            text: "李明", // "Li Ming" — ideographic, requires IME to input
            direction: Direction::Ltr,
            min_clusters: 2,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn covers_the_three_hard_scripts_plus_baseline() {
        let scripts: Vec<&str> = corpus().iter().map(|s| s.script).collect();
        for needed in ["Latn", "Arab", "Deva", "Hani"] {
            assert!(scripts.contains(&needed), "corpus must include {needed}");
        }
    }

    #[test]
    fn arabic_is_the_rtl_sample() {
        let arab = corpus().into_iter().find(|s| s.script == "Arab").unwrap();
        assert_eq!(arab.direction, Direction::Rtl);
    }

    #[test]
    fn every_sample_is_nonempty_with_a_positive_cluster_floor() {
        for s in corpus() {
            assert!(!s.text.is_empty(), "{} text empty", s.label);
            assert!(s.min_clusters > 0, "{} needs a positive floor", s.label);
            // The expected-cluster floor must be physically possible: there can
            // never be more clusters than scalar values in the text.
            let scalars = s.text.chars().count();
            assert!(
                s.min_clusters <= scalars,
                "{}: floor {} exceeds {} scalars",
                s.label,
                s.min_clusters,
                scalars
            );
        }
    }

    #[test]
    fn only_arabic_is_rtl() {
        let rtl: Vec<&str> = corpus()
            .iter()
            .filter(|s| s.direction == Direction::Rtl)
            .map(|s| s.script)
            .collect();
        assert_eq!(rtl, vec!["Arab"]);
    }
}
