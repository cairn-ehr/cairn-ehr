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
}
