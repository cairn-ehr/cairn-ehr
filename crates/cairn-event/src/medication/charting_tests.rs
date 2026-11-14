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
        source_note: None,
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
    static AGAINST: [OverrideRef<'static>; 1] = [OverrideRef {
        source: ALLERGY_SOURCE,
        reference: "E1",
    }];
    let mut a = lipitor(Some(Charting {
        charted_as: Some(ChartedAs::StartedHere),
        route: Some("po"),
        strength: Some("875/125 mg"),
        components: &COMPONENTS,
        warning_override: Some(WarningOverride {
            reason: "benefit outweighs risk",
            against: &AGAINST,
        }),
        ..Charting::default()
    }));
    a.coding = None; // components never travel beside a single coding
    let v = medication_assertion_body(&a);
    let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(|k| k.as_str()).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "charted_as",
            "dose",
            "info_source",
            "medication_id",
            "route",
            "sig",
            "started",
            "substance",
            "warning_override"
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
        v["warning_override"],
        json!({"reason": "benefit outweighs risk",
               "against": [{"source": "allergy", "ref": "E1"}]})
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
    static AGAINST: [OverrideRef<'static>; 1] = [OverrideRef {
        source: ALLERGY_SOURCE,
        reference: "E1",
    }];
    MedicationAssertion {
        medication_id: "33333333-3333-7333-8333-333333333333",
        term: "Augmentin",
        coding: None,
        formulation: Some("tablet"),
        dose_amount: None,
        dose_unit: None,
        sig: Some("1 bd"),
        source_note: None,
        info_source: "clinician-observed",
        started: None,
        started_precision: None,
        safety: None,
        charting: Some(Charting {
            charted_as: Some(charted_as),
            route: Some("po"),
            strength: Some("875/125 mg"),
            components,
            warning_override: Some(WarningOverride {
                reason: "no alternative",
                against: &AGAINST,
            }),
            ..Charting::default()
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
         · charted despite a warning (allergy): no alternative"
    );
    assert_eq!(
        render_medication_twin(&augmentin(&BOTH_CODED, ChartedAs::PreExisting)),
        "Augmentin tablet 875/125 mg po — 1 bd (clinician-observed) \
         [amoxicillin + clavulanic acid] · already taking \
         · charted despite a warning (allergy): no alternative"
    );
    assert_eq!(
        render_medication_twin(&augmentin(&BOTH_CODED, ChartedAs::Unknown)),
        "Augmentin tablet 875/125 mg po — 1 bd (clinician-observed) \
         [amoxicillin + clavulanic acid] \
         · charted despite a warning (allergy): no alternative"
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
    assert_eq!(
        render_medication_twin(&augmentin(&ONE_UNCODED, ChartedAs::Unknown)),
        "Augmentin tablet 875/125 mg po — 1 bd (clinician-observed) [amoxicillin + clav] \
         · charted despite a warning (allergy): no alternative"
    );
}

/// The tail names each warning SOURCE once, in first-seen order. Only `allergy` exists
/// today, so the two-source case is built by hand to pin the joining rule before a second
/// source (drug interactions, from drugref) reaches the wire.
#[test]
fn the_tail_names_each_source_once_in_first_seen_order() {
    static AGAINST: [OverrideRef<'static>; 3] = [
        OverrideRef {
            source: ALLERGY_SOURCE,
            reference: "E1",
        },
        OverrideRef {
            source: "interaction",
            reference: "X1",
        },
        OverrideRef {
            source: ALLERGY_SOURCE,
            reference: "E2",
        },
    ];
    let c = Charting {
        warning_override: Some(WarningOverride {
            reason: "benefit outweighs risk",
            against: &AGAINST,
        }),
        ..Charting::default()
    };
    assert_eq!(
        charting_twin_parts(&c).tail,
        " · charted despite a warning (allergy, interaction): benefit outweighs risk"
    );
}

/// A blank source names no kind of warning, so it is skipped; when no source is left the
/// parenthetical is omitted rather than rendered empty. Reachable: the remote door admits
/// an empty `against`, and a raw caller of the builder can pass one.
#[test]
fn a_blank_or_missing_source_is_never_rendered_as_empty_parentheses() {
    static NONE: [OverrideRef<'static>; 0] = [];
    static BLANK: [OverrideRef<'static>; 1] = [OverrideRef {
        source: " ",
        reference: "X1",
    }];
    static BLANK_THEN_ALLERGY: [OverrideRef<'static>; 2] = [
        OverrideRef {
            source: "",
            reference: "X1",
        },
        OverrideRef {
            source: ALLERGY_SOURCE,
            reference: "E1",
        },
    ];
    let tail = |against: &'static [OverrideRef<'static>]| {
        charting_twin_parts(&Charting {
            warning_override: Some(WarningOverride {
                reason: "no alternative",
                against,
            }),
            ..Charting::default()
        })
        .tail
    };
    assert_eq!(tail(&NONE), " · charted despite a warning: no alternative");
    assert_eq!(tail(&BLANK), " · charted despite a warning: no alternative");
    assert_eq!(
        tail(&BLANK_THEN_ALLERGY),
        " · charted despite a warning (allergy): no alternative"
    );
}

const VERSION: &str = "22222222-2222-7222-8222-222222222222";

#[test]
fn an_assertion_without_restores_is_byte_identical() {
    // Slice 2b added `restores`; a line that does not use it must serialise exactly as before.
    let want = serde_json::to_vec(&todays_full_assertion_json()).unwrap();
    let got = medication_assertion_body(&lipitor(Some(Charting {
        restores: None,
        ..Charting::default()
    })));
    assert_eq!(serde_json::to_vec(&got).unwrap(), want);
}

#[test]
fn restores_is_a_top_level_key_and_named_in_the_twin() {
    let c = Charting {
        charted_as: Some(ChartedAs::StartedHere),
        restores: Some(VERSION),
        ..Charting::default()
    };
    let v = medication_assertion_body(&lipitor(Some(c)));
    assert_eq!(v["restores"], VERSION);
    let o = v.as_object().unwrap();
    assert!(!o["substance"].as_object().unwrap().contains_key("restores"));
    assert_eq!(
        charting_twin_parts(&c).tail,
        " · started here · restores a set-aside version"
    );
    // The restores tail comes BEFORE any source note.
    let mut a = lipitor(Some(c));
    a.source_note = Some(crate::medication::source_note::SourceNote {
        source_detail: Some("GP letter"),
        why: None,
    });
    assert!(
        render_medication_twin(&a).ends_with(" · restores a set-aside version — from GP letter")
    );
}

#[test]
fn an_assertion_without_stat_is_byte_identical() {
    // Charting slice 2 added `stat`; a line that is not a stat order must serialise exactly as
    // before (principle 11): `stat: false` writes nothing, and `false` never appears on the wire.
    let want = serde_json::to_vec(&todays_full_assertion_json()).unwrap();
    let got = medication_assertion_body(&lipitor(Some(Charting {
        stat: false,
        ..Charting::default()
    })));
    assert_eq!(serde_json::to_vec(&got).unwrap(), want);
    assert!(!got.as_object().unwrap().contains_key("stat"));
}

#[test]
fn stat_is_a_top_level_true_and_absent_when_false() {
    let v = medication_assertion_body(&lipitor(Some(Charting {
        stat: true,
        ..Charting::default()
    })));
    assert_eq!(v["stat"], json!(true));
    // Top level, never inside `substance`; and the twin does not mention it (its sig says "stat").
    assert!(!v["substance"].as_object().unwrap().contains_key("stat"));
    let c = Charting {
        stat: true,
        ..Charting::default()
    };
    assert_eq!(
        charting_twin_parts(&c),
        charting_twin_parts(&Charting::default())
    );
}
