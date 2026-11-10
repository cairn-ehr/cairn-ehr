//! Charting fields on a medication assertion (charting slice 1a, design §3.4–§3.5).
//!
//! Charting a medication adds five OPTIONAL keys to `clinical.medication.asserted` —
//! top-level `charted_as`, `route`, `allergy_override`, and `substance.strength`,
//! `substance.components`. The event type and schema version do not change: an absent key
//! is the old event, byte for byte (principle 11), so a `Charting` with nothing in it writes
//! nothing. Every key and every twin word here is wire, frozen once merged.
//!
//! `assert.rs` owns the payload and the twin; it calls [`insert_charting`] and
//! [`charting_twin_parts`] so the charting shape lives in one place. The floor that judges
//! these keys is `cairn_check_medication_charting` (db/077).
//!
//! Pure: no clock, no randomness, no I/O.
use serde_json::{json, Map, Value};

use super::assert::SubstanceCoding;

/// How the medication came to be on the chart. A closed set at the authoring door; a peer's
/// newer word is admitted at the remote door (ADR-0056), so a reader must not assume these
/// three are the only strings it will ever see on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChartedAs {
    /// Started by this clinician, now ("Start medication").
    StartedHere,
    /// The patient was already taking it ("Already taking").
    PreExisting,
    /// Nobody said which — an honest unknown, never a guess (principle 4).
    Unknown,
}

impl ChartedAs {
    /// The wire word.
    pub fn as_str(self) -> &'static str {
        match self {
            ChartedAs::StartedHere => "started-here",
            ChartedAs::PreExisting => "pre-existing",
            ChartedAs::Unknown => "unknown",
        }
    }

    /// The wire word back into the enum; `None` for any word outside the closed set.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "started-here" => Some(ChartedAs::StartedHere),
            "pre-existing" => Some(ChartedAs::PreExisting),
            "unknown" => Some(ChartedAs::Unknown),
            _ => None,
        }
    }
}

/// One component of a combination ("amox + clav"): the term as typed and, when picked, its
/// drugref moiety. `coding: None` is written as an explicit `null` — inside the array every
/// element has the same two keys, so a reader never has to tell "absent" from "uncoded".
#[derive(Debug, Clone, Copy)]
pub struct Component<'a> {
    pub term: &'a str,
    pub coding: Option<SubstanceCoding<'a>>,
}

/// Charting despite a listed allergy: the clinician's reason, and which allergy record(s) the
/// line was charted against (their uuids). The window requires the reason; the floor checks
/// only the shape when it is present (design §3.5, ADR-0021 soft policy).
#[derive(Debug, Clone, Copy)]
pub struct AllergyOverride<'a> {
    pub reason: &'a str,
    pub against: &'a [&'a str],
}

/// The charting fields of one assertion. Every field is optional; `Default` is "none of
/// them", which writes nothing to the payload and nothing to the twin.
#[derive(Debug, Clone, Copy, Default)]
pub struct Charting<'a> {
    pub charted_as: Option<ChartedAs>,
    /// Open lower-case token (`po`, `iv`, `im`, …); `None` = unknown.
    pub route: Option<&'a str>,
    /// Free strength text (`500 mg`, `250 mg/5 mL`); `None` = unknown.
    pub strength: Option<&'a str>,
    /// A combination's components (≥ 2 when present). Empty = not a combination; the
    /// assertion's single `coding` is then the identity, as before.
    pub components: &'a [Component<'a>],
    pub allergy_override: Option<AllergyOverride<'a>>,
}

/// Write the charting keys into an assertion's `payload` and its `substance` object.
/// Each key is inserted only when present, never as null (principle 11).
pub fn insert_charting(
    payload: &mut Map<String, Value>,
    substance: &mut Map<String, Value>,
    c: &Charting,
) {
    if let Some(k) = c.charted_as {
        payload.insert("charted_as".into(), json!(k.as_str()));
    }
    if let Some(r) = c.route {
        payload.insert("route".into(), json!(r));
    }
    if let Some(o) = c.allergy_override {
        payload.insert(
            "allergy_override".into(),
            json!({ "reason": o.reason, "against": o.against }),
        );
    }
    if let Some(s) = c.strength {
        substance.insert("strength".into(), json!(s));
    }
    if !c.components.is_empty() {
        let list: Vec<Value> = c
            .components
            .iter()
            .map(|p| {
                let coding = p.coding.map_or(
                    Value::Null,
                    |k| json!({ "system": k.system, "code": k.code, "display": k.display }),
                );
                json!({ "term": p.term, "coding": coding })
            })
            .collect();
        substance.insert("components".into(), Value::Array(list));
    }
}

/// The strings the assertion twin splices in for charting. Empty / `None` when absent, so an
/// assertion without charting renders exactly as before.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct TwinParts {
    /// `{ strength}{ route}`, rendered right after the formulation.
    pub after_formulation: String,
    /// The components joined `" + "` (a coded one by its display, an uncoded one by its
    /// term). When `Some`, it replaces the single-coding `[display]` bracket and is always shown.
    pub components: Option<String>,
    /// `{ · started here | · already taking}{ · charted despite a listed allergy: {reason}}`,
    /// rendered at the very end.
    pub tail: String,
}

/// Build the [`TwinParts`] for one `Charting`. The word order is frozen (twin = wire).
pub fn charting_twin_parts(c: &Charting) -> TwinParts {
    let mut after_formulation = String::new();
    for v in [c.strength, c.route].into_iter().flatten() {
        after_formulation.push(' ');
        after_formulation.push_str(v);
    }
    let components = (!c.components.is_empty()).then(|| {
        c.components
            .iter()
            .map(|p| p.coding.map_or(p.term, |k| k.display))
            .collect::<Vec<_>>()
            .join(" + ")
    });
    let mut tail = String::new();
    match c.charted_as {
        Some(ChartedAs::StartedHere) => tail.push_str(" · started here"),
        Some(ChartedAs::PreExisting) => tail.push_str(" · already taking"),
        Some(ChartedAs::Unknown) | None => {}
    }
    if let Some(o) = c.allergy_override {
        tail.push_str(" · charted despite a listed allergy: ");
        tail.push_str(o.reason);
    }
    TwinParts {
        after_formulation,
        components,
        tail,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::medication::assert::{
        medication_assertion_body, render_medication_twin, MedicationAssertion, SubstanceCoding,
    };
    use serde_json::json;

    const AMOX: SubstanceCoding<'static> = SubstanceCoding {
        system: "drugref-moiety",
        code: "0f8c4b1e-1b7a-5c2d-9a3e-2b6f7c8d9e01",
        display: "amoxicillin",
    };
    const CLAV: SubstanceCoding<'static> = SubstanceCoding {
        system: "drugref-moiety",
        code: "1a2b3c4d-1b7a-5c2d-9a3e-2b6f7c8d9e02",
        display: "clavulanic acid",
    };

    /// Today's `full_assertion()` (assert.rs tests) with `charting` as given.
    fn lipitor(charting: Option<Charting<'static>>) -> MedicationAssertion<'static> {
        MedicationAssertion {
            medication_id: "11111111-1111-7111-8111-111111111111",
            term: "Lipitor",
            coding: Some(SubstanceCoding {
                system: "drugref-moiety",
                code: "0f8c4b1e-1b7a-5c2d-9a3e-2b6f7c8d9e01",
                display: "atorvastatin",
            }),
            formulation: Some("tablet"),
            dose_amount: Some("40"),
            dose_unit: Some("mg"),
            sig: Some("one BD"),
            info_source: "patient-reported",
            started: Some("2024"),
            started_precision: Some("year"),
            safety: None,
            charting,
        }
    }

    /// The payload `full_assertion()` produced before charting existed, written out by hand
    /// so a change to the old shape cannot hide behind a shared builder.
    fn todays_full_assertion_json() -> Value {
        json!({
            "medication_id": "11111111-1111-7111-8111-111111111111",
            "substance": {
                "term": "Lipitor",
                "coding": {"system": "drugref-moiety",
                           "code": "0f8c4b1e-1b7a-5c2d-9a3e-2b6f7c8d9e01",
                           "display": "atorvastatin"},
                "formulation": "tablet"
            },
            "info_source": "patient-reported",
            "dose": {"amount": "40", "unit": "mg"},
            "sig": "one BD",
            "started": {"value": "2024", "precision": "year"}
        })
    }

    #[test]
    fn no_charting_leaves_the_payload_byte_identical() {
        let want = todays_full_assertion_json();
        assert_eq!(medication_assertion_body(&lipitor(None)), want);
        assert_eq!(
            medication_assertion_body(&lipitor(Some(Charting::default()))),
            want,
            "an all-absent Charting must add no key at all (principle 11)"
        );
        assert_eq!(
            render_medication_twin(&lipitor(Some(Charting::default()))),
            render_medication_twin(&lipitor(None)),
            "an all-absent Charting must not change the twin"
        );
    }

    #[test]
    fn charting_keys_are_exactly_the_frozen_ones() {
        static COMPONENTS: [Component<'static>; 2] = [
            Component {
                term: "amox",
                coding: Some(AMOX),
            },
            Component {
                term: "clav",
                coding: None,
            },
        ];
        static AGAINST: [&str; 1] = ["E1"];
        let mut a = lipitor(Some(Charting {
            charted_as: Some(ChartedAs::StartedHere),
            route: Some("po"),
            strength: Some("875/125 mg"),
            components: &COMPONENTS,
            allergy_override: Some(AllergyOverride {
                reason: "benefit outweighs risk",
                against: &AGAINST,
            }),
        }));
        a.coding = None; // components never travel beside a single coding
        let v = medication_assertion_body(&a);
        let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(|k| k.as_str()).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "allergy_override",
                "charted_as",
                "dose",
                "info_source",
                "medication_id",
                "route",
                "sig",
                "started",
                "substance"
            ]
        );
        let mut skeys: Vec<&str> = v["substance"]
            .as_object()
            .unwrap()
            .keys()
            .map(|k| k.as_str())
            .collect();
        skeys.sort_unstable();
        assert_eq!(skeys, ["components", "formulation", "strength", "term"]);
        assert_eq!(v["charted_as"], "started-here");
        assert_eq!(v["route"], "po");
        assert_eq!(v["substance"]["strength"], "875/125 mg");
        assert_eq!(
            v["allergy_override"],
            json!({"reason": "benefit outweighs risk", "against": ["E1"]})
        );
        assert_eq!(
            v["substance"]["components"][0],
            json!({"term": "amox", "coding": {"system": "drugref-moiety",
                    "code": "0f8c4b1e-1b7a-5c2d-9a3e-2b6f7c8d9e01", "display": "amoxicillin"}})
        );
        assert_eq!(
            v["substance"]["components"][1],
            json!({"term": "clav", "coding": null})
        );
    }

    #[test]
    fn charted_as_round_trips_its_three_words() {
        for (c, s) in [
            (ChartedAs::StartedHere, "started-here"),
            (ChartedAs::PreExisting, "pre-existing"),
            (ChartedAs::Unknown, "unknown"),
        ] {
            assert_eq!(c.as_str(), s);
            assert_eq!(ChartedAs::parse(s), Some(c));
        }
        assert_eq!(ChartedAs::parse("stat"), None, "the word set is closed");
    }

    /// The Augmentin line of the brief: a combination charted with every new field.
    fn augmentin(
        components: &'static [Component<'static>],
        charted_as: ChartedAs,
    ) -> MedicationAssertion<'static> {
        static AGAINST: [&str; 1] = ["E1"];
        MedicationAssertion {
            medication_id: "33333333-3333-7333-8333-333333333333",
            term: "Augmentin",
            coding: None,
            formulation: Some("tablet"),
            dose_amount: None,
            dose_unit: None,
            sig: Some("1 bd"),
            info_source: "clinician-observed",
            started: None,
            started_precision: None,
            safety: None,
            charting: Some(Charting {
                charted_as: Some(charted_as),
                route: Some("po"),
                strength: Some("875/125 mg"),
                components,
                allergy_override: Some(AllergyOverride {
                    reason: "no alternative",
                    against: &AGAINST,
                }),
            }),
        }
    }

    #[test]
    fn twin_words_are_exact() {
        static BOTH_CODED: [Component<'static>; 2] = [
            Component {
                term: "amox",
                coding: Some(AMOX),
            },
            Component {
                term: "clav",
                coding: Some(CLAV),
            },
        ];
        assert_eq!(
            render_medication_twin(&augmentin(&BOTH_CODED, ChartedAs::StartedHere)),
            "Augmentin tablet 875/125 mg po — 1 bd (clinician-observed) \
             [amoxicillin + clavulanic acid] · started here \
             · charted despite a listed allergy: no alternative"
        );
        assert_eq!(
            render_medication_twin(&augmentin(&BOTH_CODED, ChartedAs::PreExisting)),
            "Augmentin tablet 875/125 mg po — 1 bd (clinician-observed) \
             [amoxicillin + clavulanic acid] · already taking \
             · charted despite a listed allergy: no alternative"
        );
        assert_eq!(
            render_medication_twin(&augmentin(&BOTH_CODED, ChartedAs::Unknown)),
            "Augmentin tablet 875/125 mg po — 1 bd (clinician-observed) \
             [amoxicillin + clavulanic acid] \
             · charted despite a listed allergy: no alternative"
        );
        // An uncoded component reads as its term: the bracket never drops a component.
        static ONE_UNCODED: [Component<'static>; 2] = [
            Component {
                term: "amox",
                coding: Some(AMOX),
            },
            Component {
                term: "clav",
                coding: None,
            },
        ];
        assert!(
            render_medication_twin(&augmentin(&ONE_UNCODED, ChartedAs::Unknown))
                .contains(" [amoxicillin + clav] "),
        );
    }
}
