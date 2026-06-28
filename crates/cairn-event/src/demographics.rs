//! Demographic assertion builders (spec §4). Slice 1: the §4.4 patient
//! **identifier** assertion. Pure: explicit inputs, no I/O, no DB. The
//! safety-critical structural floor lives in the database (db/010); these
//! functions only shape and render the event a node will sign.

use serde_json::{json, Value};

/// One §4.4 identifier assertion. `normalized` present without a `profile` is
/// rejected by the in-DB floor (db/010), so a caller materialising a normalized
/// form must also name the profile that produced it (the §4.4 materialised-key rule).
pub struct IdentifierAssertion<'a> {
    pub value: &'a str,      // §4.4 mandatory — as-entered, never rewritten
    pub system: &'a str,     // §4.4 mandatory — stable namespace (or the literal "unknown")
    pub provenance: &'a str, // §4.1 provenance ladder — required-present, value-open
    pub normalized: Option<&'a str>, // §4.4 — materialised matching key when a profile is present
    pub profile: Option<&'a str>,    // §4.4 — namespace@hash validator-bundle reference
    pub use_: Option<&'a str>,       // §4.4 — recommended-but-open use/type vocabulary
}

/// Build the §4.4 identifier-assertion payload (the value of `EventBody.payload`).
/// Optional facets are omitted entirely when absent — never serialized as null —
/// so the in-DB floor's key-presence checks see exactly what the author asserted.
pub fn identifier_assertion_body(a: &IdentifierAssertion) -> Value {
    let mut p = json!({
        "field": "identifier",
        "provenance": a.provenance,
        "value": a.value,
        "system": a.system,
    });
    let obj = p.as_object_mut().expect("json! built an object");
    if let Some(n) = a.normalized { obj.insert("normalized".into(), json!(n)); }
    if let Some(pr) = a.profile   { obj.insert("profile".into(),    json!(pr)); }
    if let Some(u) = a.use_       { obj.insert("use".into(),        json!(u)); }
    p
}

/// Render the §4.5 materialised legibility twin: profile-independent plaintext,
/// `"<system>, <provenance>: <value>"`. The namespace is always legible without a
/// registry; a human-friendly system label is a UI-layer refinement, not floor data.
pub fn render_identifier_twin(a: &IdentifierAssertion) -> String {
    format!("{}, {}: {}", a.system, a.provenance, a.value)
}

/// Build a generic §4.2 demographic-field assertion payload (the value of
/// `EventBody.payload`). `field` is the discriminator a node's projection keys on;
/// `facets` is an optional per-field bag (DOB's precision/basis), omitted entirely
/// when absent so the in-DB floor's key-presence checks see exactly what was asserted.
pub fn demographic_field_body(
    field: &str, value: &str, facets: Option<Value>, provenance: &str,
) -> Value {
    let mut p = json!({ "field": field, "provenance": provenance, "value": value });
    let obj = p.as_object_mut().expect("json! built an object");
    if let Some(f) = facets { obj.insert("facets".into(), f); }
    p
}

/// One §4.2 date-of-birth assertion. `precision` is mandatory (principle 4 — a date
/// must declare how precise it is; the in-DB floor rejects a dob with no precision).
/// `basis` (how the date was derived) is optional and omitted when `None`.
pub fn dob_assertion_body(
    value: &str, precision: &str, basis: Option<&str>, provenance: &str,
) -> Value {
    let mut facets = json!({ "precision": precision });
    if let Some(b) = basis {
        facets.as_object_mut().expect("json! built an object").insert("basis".into(), json!(b));
    }
    demographic_field_body("dob", value, Some(facets), provenance)
}

/// One §4.2 sex-at-birth assertion. `value` is an OPEN string — intersex /
/// indeterminate / unknown must be recordable (principle 4); never a closed enum.
pub fn sex_at_birth_assertion_body(value: &str, provenance: &str) -> Value {
    demographic_field_body("sex-at-birth", value, None, provenance)
}

/// One §4.2 name assertion. `value` is the authored display string, carried
/// verbatim ("田中 太郎", a mononym, a patronymic — culture-neutral as-authored;
/// the core never parses or normalises it). `use_` is the recommended-but-open
/// category (legal/maiden/alias/transliteration/…), placed in the `facets.use`
/// bag and omitted entirely when None so the in-DB floor sees exactly what was
/// asserted. Structured parts (given/family + a locale profile) are a later slice.
pub fn name_assertion_body(value: &str, use_: Option<&str>, provenance: &str) -> Value {
    let facets = use_.map(|u| json!({ "use": u }));
    demographic_field_body("name", value, facets, provenance)
}

/// Render the §4.5 legibility twin for a name. Matches the spec example
/// "Name (legal): 田中 太郎": the `use` sits in the parens when present; when it is
/// absent the parens fall back to the provenance ("Name (patient-stated): Mary")
/// so the parenthetical is never empty and the fact stays legible without a profile.
pub fn render_name_twin(value: &str, use_: Option<&str>, provenance: &str) -> String {
    let context = use_.unwrap_or(provenance);
    format!("Name ({context}): {value}")
}

/// Render the §4.5 materialised legibility twin for a date of birth:
/// `"Date of birth (<provenance>): <value> (<precision>)"`. Profile-independent —
/// readable on a node that has never seen the dob field's schema.
pub fn render_dob_twin(value: &str, precision: &str, provenance: &str) -> String {
    format!("Date of birth ({provenance}): {value} ({precision})")
}

/// Render the §4.5 legibility twin for sex-at-birth:
/// `"Sex at birth (<provenance>): <value>"`.
pub fn render_sex_at_birth_twin(value: &str, provenance: &str) -> String {
    format!("Sex at birth ({provenance}): {value}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> IdentifierAssertion<'static> {
        IdentifierAssertion {
            value: "943 476 5919", system: "nhs-number",
            provenance: "document-verified",
            normalized: Some("9434765919"), profile: Some("nhs-number@b3-abc"),
            use_: Some("national-id"),
        }
    }

    #[test]
    fn body_includes_all_facets_when_present() {
        let v = identifier_assertion_body(&sample());
        assert_eq!(v["field"], "identifier");
        assert_eq!(v["value"], "943 476 5919");
        assert_eq!(v["system"], "nhs-number");
        assert_eq!(v["provenance"], "document-verified");
        assert_eq!(v["normalized"], "9434765919");
        assert_eq!(v["profile"], "nhs-number@b3-abc");
        assert_eq!(v["use"], "national-id");
    }

    #[test]
    fn body_omits_absent_optional_facets_never_null() {
        let a = IdentifierAssertion {
            value: "X1", system: "unknown", provenance: "patient-stated",
            normalized: None, profile: None, use_: None,
        };
        let v = identifier_assertion_body(&a);
        let obj = v.as_object().unwrap();
        assert!(!obj.contains_key("normalized"), "absent facet must be omitted, not null");
        assert!(!obj.contains_key("profile"));
        assert!(!obj.contains_key("use"));
    }

    #[test]
    fn twin_renders_profile_independent_plaintext() {
        assert_eq!(
            render_identifier_twin(&sample()),
            "nhs-number, document-verified: 943 476 5919"
        );
    }

    #[test]
    fn dob_body_carries_field_value_provenance_and_facets() {
        let v = dob_assertion_body("1980-07-15", "day", Some("document"), "document-verified");
        assert_eq!(v["field"], "dob");
        assert_eq!(v["value"], "1980-07-15");
        assert_eq!(v["provenance"], "document-verified");
        assert_eq!(v["facets"]["precision"], "day");
        assert_eq!(v["facets"]["basis"], "document");
    }

    #[test]
    fn dob_body_omits_absent_basis_never_null() {
        let v = dob_assertion_body("1980", "year", None, "patient-stated");
        assert_eq!(v["facets"]["precision"], "year");
        let facets = v["facets"].as_object().unwrap();
        assert!(!facets.contains_key("basis"), "absent basis must be omitted, not null");
    }

    #[test]
    fn sex_at_birth_body_has_no_facets() {
        let v = sex_at_birth_assertion_body("female", "clinician-observed");
        assert_eq!(v["field"], "sex-at-birth");
        assert_eq!(v["value"], "female");
        assert_eq!(v["provenance"], "clinician-observed");
        let obj = v.as_object().unwrap();
        assert!(!obj.contains_key("facets"), "sex-at-birth carries no facets bag");
    }

    #[test]
    fn dob_and_sex_at_birth_twins_render_profile_independent_plaintext() {
        assert_eq!(
            render_dob_twin("1980", "year", "patient-stated"),
            "Date of birth (patient-stated): 1980 (year)"
        );
        assert_eq!(
            render_sex_at_birth_twin("female", "clinician-observed"),
            "Sex at birth (clinician-observed): female"
        );
    }

    #[test]
    fn name_body_carries_field_value_use_and_provenance() {
        let v = name_assertion_body("田中 太郎", Some("legal"), "document-verified");
        assert_eq!(v["field"], "name");
        assert_eq!(v["value"], "田中 太郎");
        assert_eq!(v["provenance"], "document-verified");
        assert_eq!(v["facets"]["use"], "legal");
    }

    #[test]
    fn name_body_omits_absent_use_never_null() {
        let v = name_assertion_body("Ronaldinho", None, "patient-stated");
        assert_eq!(v["field"], "name");
        assert_eq!(v["value"], "Ronaldinho");
        let obj = v.as_object().unwrap();
        assert!(!obj.contains_key("facets"), "absent use carries no facets bag, never null");
    }

    #[test]
    fn name_twin_uses_use_when_present_else_provenance() {
        assert_eq!(
            render_name_twin("田中 太郎", Some("legal"), "document-verified"),
            "Name (legal): 田中 太郎"
        );
        assert_eq!(
            render_name_twin("Mary", None, "patient-stated"),
            "Name (patient-stated): Mary"
        );
    }
}
