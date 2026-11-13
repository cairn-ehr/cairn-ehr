use super::*;

fn u(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

/// A draft line; `parts` is the stored jsonb shape `[{term, coding}]`.
fn line(id: u128, parts: serde_json::Value, set_aside_of: Option<String>) -> DraftLine {
    DraftLine {
        line_id: u(id),
        terminal_id: "pc-1".into(),
        seq: id as i32,
        charted_as: "started-here".into(),
        line_text: format!("line {id}"),
        parts,
        reason: None,
        hand_ticked: false,
        ticked_against: vec![],
        finished: true,
        saved_at: "2026-11-13 10:00:00+00".into(),
        typed_drug: String::new(),
        directions: String::new(),
        held: false,
        info_source: None,
        source_detail: None,
        why: None,
        set_aside_of,
    }
}

/// One part coded under `system`.
fn coded(system: &str, code: &str) -> serde_json::Value {
    serde_json::json!([{"term": "x", "coding": {"system": system, "code": code, "display": "x"}}])
}

fn oxy(id: u128, set_aside_of: Option<String>) -> DraftLine {
    line(id, coded(MOIETY_SYSTEM, "oxycodone"), set_aside_of)
}

#[test]
fn uncoded_and_combinations_have_no_candidates() {
    let uncoded = line(1, serde_json::json!([{"term": "x", "coding": null}]), None);
    let combo = line(
        2,
        serde_json::json!([
            {"term": "a", "coding": {"system": MOIETY_SYSTEM, "code": "oxycodone", "display": "a"}},
            {"term": "b", "coding": {"system": MOIETY_SYSTEM, "code": "naloxone", "display": "b"}}
        ]),
        None,
    );
    let other_system = line(3, coded("some-other-system", "oxycodone"), None);
    let twin = oxy(4, None);
    let current = [(u(90), "oxycodone")];
    for l in [&uncoded, &combo, &other_system] {
        assert!(companion_candidates(l, std::slice::from_ref(&twin), &current).is_empty());
    }
}

#[test]
fn two_oxycodone_lines_are_two_candidates() {
    let (a, b, c) = (oxy(1, None), oxy(2, None), oxy(3, None));
    let got = companion_candidates(&a, &[a.clone(), b, c], &[]);
    assert_eq!(got, vec![Companion::Draft(u(2)), Companion::Draft(u(3))]);
}

#[test]
fn a_charted_current_line_is_a_candidate() {
    let a = oxy(1, None);
    let got = companion_candidates(
        &a,
        std::slice::from_ref(&a),
        &[(u(90), "oxycodone"), (u(91), "codeine")],
    );
    assert_eq!(got, vec![Companion::Charted(u(90))]);
}

#[test]
fn a_set_aside_line_is_not_a_candidate_for_another() {
    let a = oxy(1, None);
    let b = oxy(2, Some(format!("med:{}", u(90))));
    assert!(companion_candidates(&a, &[a.clone(), b], &[]).is_empty());
}

#[test]
fn a_version_holds_while_its_companion_is_still_a_candidate() {
    let a = oxy(1, Some(format!("line:{}", u(2))));
    let b = oxy(2, None);
    let v = version_holds(&a, &[a.clone(), b], &[]);
    assert_eq!(v, Ok(Companion::Draft(u(2))));
    let a = oxy(1, Some(format!("med:{}", u(90))));
    assert_eq!(
        version_holds(&a, std::slice::from_ref(&a), &[(u(90), "oxycodone")]),
        Ok(Companion::Charted(u(90)))
    );
}

/// Review Focus 2: the companion was edited to another drug after the mark was made.
#[test]
fn an_edited_companion_of_another_drug_no_longer_holds() {
    let a = oxy(1, Some(format!("line:{}", u(2))));
    let b = line(2, coded(MOIETY_SYSTEM, "morphine"), None);
    assert_eq!(
        version_holds(&a, &[a.clone(), b], &[]),
        Err("the line it was set aside beside is no longer the same drug")
    );
    // A charted line recoded to another drug reads the same.
    let a = oxy(1, Some(format!("med:{}", u(90))));
    assert_eq!(
        version_holds(&a, std::slice::from_ref(&a), &[(u(90), "morphine")]),
        Err("the line it was set aside beside is no longer the same drug")
    );
    // A line not set aside, or a mark that is not an id, never holds either.
    assert!(version_holds(&oxy(1, None), &[], &[]).is_err());
    assert!(version_holds(&oxy(1, Some("line:nonsense".into())), &[], &[]).is_err());
}

/// Ruling R13: a companion that is GONE is never worded as another drug. A charted line stopped
/// (no longer in `current`) is no longer current; a list line removed is no longer on the list.
#[test]
fn a_gone_companion_says_it_is_gone_not_another_drug() {
    let a = oxy(1, Some(format!("med:{}", u(90))));
    assert_eq!(
        version_holds(&a, std::slice::from_ref(&a), &[]),
        Err("its line is no longer current")
    );
    let a = oxy(1, Some(format!("line:{}", u(2))));
    assert_eq!(
        version_holds(&a, std::slice::from_ref(&a), &[]),
        Err("its line is no longer on the list")
    );
}

#[test]
fn each_preset_maps_to_its_category() {
    for (preset, want) in [
        ("Patient", "patient-reported"),
        ("Carer / family", "patient-reported"),
        ("GP letter", "external-record"),
        ("Pharmacy", "external-record"),
        ("Packets / Webster pack seen", "clinician-observed"),
    ] {
        assert_eq!(info_source_of(Some(preset)), Some(want), "{preset}");
    }
}

#[test]
fn other_is_unknown() {
    assert_eq!(info_source_of(Some("Other")), Some("unknown"));
    assert_eq!(info_source_of(None), None);
    assert_eq!(info_source_of(Some("Hearsay")), None);
}

/// The labels db/078's `medication_draft_from_preset_check` accepts; keep in step with the SQL.
#[test]
fn every_stored_preset_label_maps_to_a_category() {
    for label in [
        "Patient",
        "Carer / family",
        "GP letter",
        "Pharmacy",
        "Packets / Webster pack seen",
        "Other",
    ] {
        assert!(info_source_of(Some(label)).is_some(), "{label}");
    }
}
