//! The allergy stream: `clinical.allergy.asserted` and `clinical.allergy-none.asserted`
//! (allergy slice 1). Pure: no clock, no randomness, no I/O.
//!
//! Each event is signed with a plaintext legibility twin (principle 11); the twin wording
//! lives here ONCE ([`entry_words`]) so the signed twin and the strip's line cannot drift.
use serde_json::{json, Value};

/// Event type of one allergy / adverse-effect entry.
pub const ALLERGY_EVENT_TYPE: &str = "clinical.allergy.asserted";
/// Payload schema tag; additive-only evolution (principle 11).
pub const ALLERGY_SCHEMA_VERSION: &str = "clinical.allergy/1";
/// Event type of the "no known allergies" assertion.
pub const ALLERGY_NONE_EVENT_TYPE: &str = "clinical.allergy-none.asserted";
/// Payload schema tag of the "none" assertion.
pub const ALLERGY_NONE_SCHEMA_VERSION: &str = "clinical.allergy-none/1";

/// Declares a closed wire vocabulary: each variant maps to one exact (case-sensitive) word.
macro_rules! wire_enum {
    ($(#[$m:meta])* $name:ident { $($variant:ident => $word:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $name { $($variant),+ }
        impl $name {
            /// The wire word.
            pub fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $word),+ }
            }
            /// Parse a wire word; `None` for anything else (case matters).
            pub fn parse(s: &str) -> Option<Self> {
                match s { $($word => Some(Self::$variant),)+ _ => None }
            }
        }
    };
}

wire_enum!(/// Whether the allergen is a drug or something else.
    AllergenKind { Drug => "drug", Other => "other" });
wire_enum!(/// A true allergy versus a previous adverse effect.
    EntryKind { Allergy => "allergy", AdverseEffect => "adverse-effect" });
wire_enum!(/// The reaction, coarsely; `NotKnown` is a first-class answer (principle 4).
ReactionCategory {
    Anaphylaxis => "anaphylaxis", Angioedema => "angioedema", Urticaria => "urticaria",
    Rash => "rash", GiUpset => "gi-upset", Other => "other", NotKnown => "not-known",
});
wire_enum!(/// Where the clinician learned of it.
    Source { PatientStated => "patient-stated", SeenDocumented => "seen-documented",
             AnotherRecord => "another-record" });
wire_enum!(/// What a "none" assertion covers.
    NoneScope { Drug => "drug", All => "all" });

/// When the reaction first happened; owns its (already validated, normalised ISO) value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Onset {
    Day(String),
    Month(String),
    Year(String),
    NotKnown,
}

/// One entry, ready to seal.
pub struct AllergyEntry<'a> {
    pub allergen_kind: AllergenKind,
    pub term: &'a str,
    pub kind: EntryKind,
    pub category: ReactionCategory,
    pub reaction_text: Option<&'a str>,
    pub onset: Onset,
    pub source: Source,
}

/// Build the `clinical.allergy/1` payload. Keys are frozen; `substance.coding` and `refs`
/// are reserved slots written empty so filling them later is additive.
pub fn allergy_payload(e: &AllergyEntry) -> Value {
    let (precision, value) = match &e.onset {
        Onset::Day(v) => ("day", Some(v)),
        Onset::Month(v) => ("month", Some(v)),
        Onset::Year(v) => ("year", Some(v)),
        Onset::NotKnown => ("not-known", None),
    };
    json!({
        "allergen_kind": e.allergen_kind.as_str(),
        "substance": {"term": e.term, "coding": null},
        "kind": e.kind.as_str(),
        "reaction": {"category": e.category.as_str(), "text": e.reaction_text},
        "onset": {"precision": precision, "value": value},
        "source": e.source.as_str(),
        "refs": [],
    })
}

/// Build the `clinical.allergy-none/1` payload.
pub fn allergy_none_payload(scope: NoneScope) -> Value {
    json!({"scope": scope.as_str()})
}

/// The reaction phrase. Takes wire words so a newer peer's unknown word reads as itself.
fn reaction_phrase(category: &str, text: Option<&str>) -> String {
    let with_text = |base: &str| match text {
        Some(t) => format!("{base}: {t}"),
        None => base.to_string(),
    };
    match category {
        "anaphylaxis" => with_text("anaphylaxis"),
        "angioedema" => with_text("angioedema"),
        "urticaria" => with_text("urticaria"),
        "rash" => with_text("rash"),
        "gi-upset" => with_text("GI upset"),
        "other" => text.unwrap_or("other reaction").to_string(),
        "not-known" => "effect not known".to_string(),
        unknown => with_text(unknown),
    }
}

/// The clinician wording of one entry, without dates (a later layer adds onset). Takes WIRE
/// WORDS, not enums, so the strip can call it with database strings.
pub fn entry_words(
    kind: &str,
    source: &str,
    term: &str,
    category: &str,
    reaction_text: Option<&str>,
) -> String {
    if kind == "adverse-effect" {
        let tail = match source {
            "patient-stated" => " (patient stated)",
            "another-record" => " (from another record)",
            _ => "",
        };
        return format!(
            "Previous adverse effect: {term} → {}{tail}",
            reaction_phrase(category, reaction_text)
        );
    }
    let reaction = reaction_phrase(category, reaction_text);
    match source {
        "patient-stated" if category == "not-known" => {
            format!("Patient stated allergy: {term}. Effect not known.")
        }
        "patient-stated" => format!("Patient stated allergy: {term} → {reaction}"),
        "another-record" => format!("Allergy (from another record): {term} → {reaction}"),
        _ => format!("Allergy: {term} → {reaction}"),
    }
}

/// The signed plaintext twin of an entry.
pub fn render_allergy_twin(e: &AllergyEntry) -> String {
    entry_words(
        e.kind.as_str(),
        e.source.as_str(),
        e.term,
        e.category.as_str(),
        e.reaction_text,
    )
}

/// The signed plaintext twin of a "none" assertion.
pub fn render_allergy_none_twin(scope: NoneScope) -> String {
    match scope {
        NoneScope::All => "No known allergies".to_string(),
        NoneScope::Drug => "No known drug allergies".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> AllergyEntry<'static> {
        AllergyEntry {
            allergen_kind: AllergenKind::Drug,
            term: "penicillin",
            kind: EntryKind::Allergy,
            category: ReactionCategory::Rash,
            reaction_text: None,
            onset: Onset::NotKnown,
            source: Source::SeenDocumented,
        }
    }

    #[test]
    fn payload_has_exactly_the_frozen_keys() {
        let p = allergy_payload(&entry());
        let mut keys: Vec<_> = p.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "allergen_kind",
                "kind",
                "onset",
                "reaction",
                "refs",
                "source",
                "substance"
            ]
        );
        assert_eq!(p["substance"]["coding"], serde_json::Value::Null);
        assert_eq!(p["substance"]["term"], "penicillin");
        assert_eq!(p["refs"], serde_json::json!([]));
        assert_eq!(p["reaction"]["category"], "rash");
        assert_eq!(p["reaction"]["text"], serde_json::Value::Null);
        assert_eq!(
            allergy_none_payload(NoneScope::Drug),
            serde_json::json!({"scope": "drug"})
        );
    }

    #[test]
    fn every_enum_round_trips_its_wire_word() {
        for x in [AllergenKind::Drug, AllergenKind::Other] {
            assert_eq!(AllergenKind::parse(x.as_str()), Some(x));
        }
        for x in [EntryKind::Allergy, EntryKind::AdverseEffect] {
            assert_eq!(EntryKind::parse(x.as_str()), Some(x));
        }
        for x in [
            ReactionCategory::Anaphylaxis,
            ReactionCategory::Angioedema,
            ReactionCategory::Urticaria,
            ReactionCategory::Rash,
            ReactionCategory::GiUpset,
            ReactionCategory::Other,
            ReactionCategory::NotKnown,
        ] {
            assert_eq!(ReactionCategory::parse(x.as_str()), Some(x));
        }
        for x in [
            Source::PatientStated,
            Source::SeenDocumented,
            Source::AnotherRecord,
        ] {
            assert_eq!(Source::parse(x.as_str()), Some(x));
        }
        for x in [NoneScope::Drug, NoneScope::All] {
            assert_eq!(NoneScope::parse(x.as_str()), Some(x));
        }
        assert_eq!(EntryKind::parse("Allergy"), None);
        assert_eq!(ReactionCategory::GiUpset.as_str(), "gi-upset");
    }

    #[test]
    fn not_known_onset_has_a_null_value() {
        let p = allergy_payload(&entry());
        assert_eq!(
            p["onset"],
            serde_json::json!({"precision": "not-known", "value": null})
        );
        let mut e = entry();
        e.onset = Onset::Month("2019-04".to_string());
        assert_eq!(
            allergy_payload(&e)["onset"],
            serde_json::json!({"precision": "month", "value": "2019-04"})
        );
    }

    #[test]
    fn twin_keeps_text_verbatim() {
        let mut e = entry();
        e.term = "<script>\"5 < 10\" 🙂";
        e.reaction_text = Some("a & b");
        let t = render_allergy_twin(&e);
        assert!(t.contains("<script>\"5 < 10\" 🙂"), "{t}");
        assert!(t.contains("a & b"), "{t}");
    }

    #[test]
    fn twin_says_patient_stated_and_effect_not_known() {
        let mut e = entry();
        e.term = "penicillin";
        e.source = Source::PatientStated;
        e.category = ReactionCategory::NotKnown;
        assert_eq!(
            render_allergy_twin(&e),
            "Patient stated allergy: penicillin. Effect not known."
        );
    }

    #[test]
    fn wording_of_every_shape() {
        let w = |k, s, c, t| entry_words(k, s, "X", c, t);
        assert_eq!(
            w("allergy", "seen-documented", "rash", None),
            "Allergy: X → rash"
        );
        assert_eq!(
            w("allergy", "seen-documented", "gi-upset", Some("vomiting")),
            "Allergy: X → GI upset: vomiting"
        );
        assert_eq!(
            w("allergy", "another-record", "anaphylaxis", None),
            "Allergy (from another record): X → anaphylaxis"
        );
        assert_eq!(
            w("allergy", "seen-documented", "other", Some("swelling")),
            "Allergy: X → swelling"
        );
        assert_eq!(
            w("allergy", "seen-documented", "other", None),
            "Allergy: X → other reaction"
        );
        assert_eq!(
            w("allergy", "seen-documented", "not-known", None),
            "Allergy: X → effect not known"
        );
        assert_eq!(
            w("adverse-effect", "seen-documented", "urticaria", None),
            "Previous adverse effect: X → urticaria"
        );
        assert_eq!(
            w("adverse-effect", "patient-stated", "rash", None),
            "Previous adverse effect: X → rash (patient stated)"
        );
        assert_eq!(
            w("adverse-effect", "another-record", "rash", None),
            "Previous adverse effect: X → rash (from another record)"
        );
        assert_eq!(
            w("allergy", "seen-documented", "weird", Some("t")),
            "Allergy: X → weird: t"
        );
        assert_eq!(
            w("allergy", "patient-stated", "angioedema", None),
            "Patient stated allergy: X → angioedema"
        );
    }

    #[test]
    fn none_twins() {
        assert_eq!(
            render_allergy_none_twin(NoneScope::All),
            "No known allergies"
        );
        assert_eq!(
            render_allergy_none_twin(NoneScope::Drug),
            "No known drug allergies"
        );
    }
}
