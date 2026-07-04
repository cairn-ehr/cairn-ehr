//! §5.4 clinician-observed identity evidence for unidentified ("John Doe") patients.
//!
//! A clinician who registers an unknown patient cannot know a DOB — they have an
//! *estimated age with basis* ("apparent age ~40, dentition/greying"). This module turns
//! that honest, imprecise observation into a demographic assertion the existing db/011
//! `dob` field already accepts, and an observed-sex assertion on `administrative-sex`.
//!
//! Two principle-4 rules shape the representation:
//!   1. Store the derived **birth-year window**, never the raw age — birth year is
//!      time-invariant; age drifts, so storing "40" would silently age the record.
//!   2. Store an **explicit range**, never a single midpoint — a midpoint year is a
//!      *precise untruth* the matcher would wrongly DISAGREE against a nearby true DOB.
//!
//! Pure functions only (no DB, no clock): the caller (the node layer) supplies the
//! observation year so every function here is deterministic and unit-testable.

use crate::demographics::{demographic_field_body, dob_assertion_body};
use serde_json::{json, Value};

/// The `facets.precision` term marking a dob value as an inclusive birth-year interval
/// (`"<min>/<max>"`). Distinct from the point precisions ("year"/"month"/"day"); the
/// matcher keys its range parsing on exactly this string.
pub const YEAR_RANGE_PRECISION: &str = "year-range";

/// The §4.1 provenance ladder term for evidence a clinician directly observed. Ranks 30
/// in db/011 — below patient-stated (50) and document-verified (60), so a real document
/// correctly displaces the estimate the moment identity is established.
pub const CLINICIAN_OBSERVED_PROVENANCE: &str = "clinician-observed";

/// Convert an estimated age (with a ± tolerance) observed in a given year into an
/// inclusive birth-year range. `birth_year = observed_year - age`; the tolerance widens
/// it symmetrically. Example: age 40 ± 5 observed in 2026 -> (1981, 1991). Returns
/// (min_year, max_year) with min_year <= max_year for any non-negative inputs.
///
/// PRECONDITION: `age_years`/`tolerance_years` are plausible human values (a lifespan's
/// worth, not billions). The caller (the CLI arm) bounds them; a value near `u32::MAX`
/// would reinterpret negative on the `as i32` cast and overflow the subtraction. This is
/// a pure helper, so it does not re-validate — keep the bound at the human input boundary.
pub fn birth_year_range_from_age(age_years: u32, tolerance_years: u32, observed_year: i32) -> (i32, i32) {
    let mid = observed_year - age_years as i32;
    let tol = tolerance_years as i32;
    (mid - tol, mid + tol)
}

/// Render an inclusive birth-year range as the canonical dob value string `"<min>/<max>"`
/// (ISO-8601-interval-style `/` separator). The `/` never collides with the ISO `-` date
/// splitter, so a range value safely fails point-date parsing on any older node.
pub fn format_year_range(min_year: i32, max_year: i32) -> String {
    format!("{min_year}/{max_year}")
}

/// Build a §4.2 estimated-age `dob` assertion payload: value = `"<min>/<max>"`, precision
/// = `year-range`, basis = the clinician's stated basis (required — §5.4 is "estimated age
/// WITH basis"; principle 4). Reuses `dob_assertion_body`, so the db/011 dob floor
/// (precision required, basis optional-non-empty) accepts it unchanged.
pub fn estimated_dob_body(min_year: i32, max_year: i32, basis: &str, provenance: &str) -> Value {
    let value = format_year_range(min_year, max_year);
    dob_assertion_body(&value, YEAR_RANGE_PRECISION, Some(basis), provenance)
}

/// Build a §4.2 observed-sex assertion payload on the `administrative-sex` field (the
/// apparent/phenotypic marker a clinician can honestly observe — NOT the `sex-at-birth`
/// fact, which they cannot know for a stranger). `basis` (how it was observed) is optional
/// and omitted entirely when None. Value is an OPEN string (principle 4).
pub fn observed_sex_body(value: &str, basis: Option<&str>, provenance: &str) -> Value {
    let facets = basis.map(|b| json!({ "basis": b }));
    demographic_field_body("administrative-sex", value, facets, provenance)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demographics::render_dob_twin;

    #[test]
    fn birth_year_range_is_observed_year_minus_age_widened_by_tolerance() {
        // "apparent age ~40 ± 5", observed in 2026 -> born 1981..=1991.
        assert_eq!(birth_year_range_from_age(40, 5, 2026), (1981, 1991));
    }

    #[test]
    fn zero_tolerance_is_a_single_year_still_expressed_as_a_range() {
        assert_eq!(birth_year_range_from_age(30, 0, 2020), (1990, 1990));
    }

    #[test]
    fn format_year_range_uses_the_slash_separator() {
        assert_eq!(format_year_range(1981, 1991), "1981/1991");
    }

    #[test]
    fn estimated_dob_body_is_a_year_range_dob_with_basis_and_provenance() {
        let v = estimated_dob_body(1981, 1991, "apparent age ~40±5: dentition, greying",
                                   CLINICIAN_OBSERVED_PROVENANCE);
        assert_eq!(v["field"], "dob");
        assert_eq!(v["value"], "1981/1991");
        assert_eq!(v["facets"]["precision"], "year-range");
        assert_eq!(v["facets"]["basis"], "apparent age ~40±5: dentition, greying");
        assert_eq!(v["provenance"], "clinician-observed");
    }

    #[test]
    fn estimated_dob_twin_is_legible_without_a_profile() {
        // The reused render_dob_twin must produce a non-empty, human-readable twin.
        let twin = render_dob_twin("1981/1991", YEAR_RANGE_PRECISION, CLINICIAN_OBSERVED_PROVENANCE);
        assert!(twin.contains("1981/1991"));
        assert!(twin.contains("year-range"));
        assert!(!twin.trim().is_empty());
    }

    #[test]
    fn observed_sex_body_is_administrative_sex_with_optional_basis() {
        let with = observed_sex_body("male", Some("external genitalia"), CLINICIAN_OBSERVED_PROVENANCE);
        assert_eq!(with["field"], "administrative-sex");
        assert_eq!(with["value"], "male");
        assert_eq!(with["facets"]["basis"], "external genitalia");
        assert_eq!(with["provenance"], "clinician-observed");

        let without = observed_sex_body("female", None, CLINICIAN_OBSERVED_PROVENANCE);
        assert_eq!(without["field"], "administrative-sex");
        assert!(without.get("facets").is_none(), "absent basis must omit facets entirely, never null");
    }
}
