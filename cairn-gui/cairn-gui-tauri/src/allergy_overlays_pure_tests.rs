//! Pure tests for `allergy_overlays.rs` (they run in CI, unlike the DB walks in
//! `allergy_overlays_tests.rs`): the outcome of a sign that landed or was interrupted, and the
//! target rule (`target_on`) — what each act may name, and that nothing on a chart whose link is
//! in doubt may be named (review C1). Split from the DB walks to keep both files short.
use super::*;
use crate::allergies::{sign_failure, written, Act};
use crate::allergy_pane::{AllergyPane, EnteredInErrorLine};
use crate::funnel::view::{Recorded, Retry};
use cairn_event::allergy::{AllergyEntry, NoneScope};
use cairn_event::allergy_overlay::{AllergyChallenge, AllergyRetraction};

const GONE: &str = "That allergy line is no longer on this chart — read it again.";

/// A pane with something in every slice-2 list, so `written` is seen carrying it whole.
fn full_pane() -> AllergyPane {
    AllergyPane {
        strip: "Allergies not yet asked".into(),
        drug: "not-yet-asked",
        other: "not-yet-asked",
        drug_allergies: vec![],
        other_allergies: vec![],
        adverse_effects: vec![],
        previously_none: vec![],
        standing_none: vec![],
        unreadable: 0,
        historical: vec![],
        entered_in_error: vec![EnteredInErrorLine {
            text: "Entered in error: de-label — tolerated amoxicillin (withdrawn 9 Nov 2026)"
                .into(),
            earlier: vec![],
        }],
        ignored_overlays: vec!["Not counted: …".into()],
        charts: vec![],
    }
}

/// Ruling R11 for the overlays: a de-label or withdrawal that landed is an Ok whatever the re-read
/// did, because both commands end in the one `sign_then_reread`, which hands its re-read to
/// `written` (pinned at its call site by `allergies_webview_tests`).
#[test]
fn a_landed_overlay_is_an_ok_with_or_without_its_pane() {
    let ok = written(Ok(full_pane()));
    assert_eq!(ok.outcome, "Recorded.");
    let pane = ok.pane.expect("the list read again");
    assert_eq!(pane.entered_in_error.len(), 1);
    assert_eq!(pane.ignored_overlays.len(), 1);
    let failed = written(Err(ErrorView {
        text: "connection closed".into(),
        retry: Retry::Now,
    }));
    assert!(failed.pane.is_none());
    assert!(
        failed.outcome.starts_with("Signed — but"),
        "{}",
        failed.outcome
    );
    let src = include_str!("allergy_overlays.rs");
    for f in ["allergy_challenge_impl", "allergy_retract_impl"] {
        let body = crate::source_guard::rust_fn(src, f);
        assert!(body.contains("sign_then_reread("), "{f}: {body}");
    }
}

/// #814 for the overlays: an interrupted commit is NOT known, named as the act it was, and is
/// re-read, never offered again live.
#[test]
fn an_interrupted_overlay_is_not_known_and_never_retried() {
    let event = Uuid::from_u128(814);
    let interrupted = || {
        cairn_node::db_diagnosis::commit_outcome_unknown(
            event,
            anyhow::anyhow!("connection closed"),
        )
        .context("signing the overlay")
    };
    let target = Uuid::from_u128(1).to_string();
    let challenge = Act::Challenge(AllergyChallenge {
        target: &target,
        tolerated: "amoxicillin",
        date: cairn_event::allergy::Onset::NotKnown,
        text: None,
    });
    let retraction = Act::Retraction(AllergyRetraction {
        target: &target,
        text: None,
    });
    let corrected = Act::Entry(AllergyEntry {
        allergen_kind: cairn_event::allergy::AllergenKind::Drug,
        term: "penicillin",
        kind: cairn_event::allergy::EntryKind::Allergy,
        category: cairn_event::allergy::ReactionCategory::Anaphylaxis,
        reaction_text: None,
        onset: cairn_event::allergy::Onset::NotKnown,
        source: cairn_event::allergy::Source::SeenDocumented,
        corrects: Some(&target),
    });
    for (act, lead) in [
        (
            &challenge,
            "Whether this de-label was recorded is NOT known — ",
        ),
        (
            &retraction,
            "Whether this was marked entered in error is NOT known — ",
        ),
        (
            &corrected,
            "Whether this correction was recorded is NOT known — ",
        ),
        (
            &Act::None(NoneScope::All),
            "Whether this allergy was recorded is NOT known — ",
        ),
    ] {
        let view = sign_failure(&interrupted(), act);
        assert!(view.text.starts_with(lead), "{}", view.text);
        assert!(
            view.text.contains(&format!("(event {event}: ")),
            "{}",
            view.text
        );
        assert_eq!(view.retry, Retry::AfterReread(Recorded::Unknown));
    }
    let plain = anyhow::anyhow!("the floor refused it");
    assert_eq!(
        sign_failure(&plain, &retraction),
        crate::write_failure::node_failure(&plain)
    );
}

/// The target rule at its pure seam (runs in CI, unlike the DB walks): Correct and De-label name a
/// live line only; Entered in error also names a "none known" line, the de-label a historical
/// line rests on, or that historical line's own entry (striking a de-labelled line is one act,
/// TBD-2). Each is filed under its own row's chart.
#[test]
fn a_target_names_only_what_its_act_may_name() {
    let (a, v) = overlay_fixture(vec![]);
    let (here, linked) = (Uuid::from_u128(1), Uuid::from_u128(2));
    let on = |id: u128, names| target_on(&a, &v, &Uuid::from_u128(id).to_string(), names);
    let filed = |id: u128, chart| {
        Ok::<_, ErrorView>(Target {
            id: Uuid::from_u128(id),
            chart,
        })
    };
    for names in [Names::Entry, Names::Withdrawable] {
        assert_eq!(
            on(10, names),
            filed(10, linked),
            "a live line, on its own chart"
        );
        assert_eq!(on(99, names).unwrap_err().text, GONE, "a stranger");
        let garbage = target_on(&a, &v, "not-a-uuid", names).unwrap_err();
        assert_eq!(garbage.retry, Retry::AfterReread(Recorded::No));
    }
    assert_eq!(
        on(11, Names::Entry).unwrap_err().text,
        GONE,
        "a de-labelled entry"
    );
    assert_eq!(
        on(11, Names::Withdrawable),
        filed(11, linked),
        "a de-labelled line's own entry, struck in one act"
    );
    assert_eq!(on(20, Names::Entry).unwrap_err().text, GONE);
    assert_eq!(on(30, Names::Entry).unwrap_err().text, GONE);
    assert_eq!(on(20, Names::Withdrawable), filed(20, here), "a none line");
    assert_eq!(
        on(30, Names::Withdrawable),
        filed(30, linked),
        "the de-label"
    );
}

/// Rows for the pure target tests, and their view: on chart 2 (`linked`), a live penicillin
/// line (10) and a codeine line (11) de-labelled by challenge 30; on chart 1 (`here`), a "none
/// known" statement (20). `doubted_charts` is the read's.
fn overlay_fixture(doubted_charts: Vec<Uuid>) -> (PatientAllergies, AllergyView) {
    use cairn_node::allergy::read::{AllergyRow, ChallengeRow, KeyState, NoneRow};
    let (here, linked) = (Uuid::from_u128(1), Uuid::from_u128(2));
    let at = chrono::DateTime::from_timestamp(1_762_600_000, 0).unwrap();
    let row = |id: u128, chart: Uuid, term: &str| AllergyRow {
        event_id: Uuid::from_u128(id),
        chart,
        allergen_kind: "drug".into(),
        term: term.into(),
        kind: "allergy".into(),
        category: "rash".into(),
        reaction_text: None,
        onset_precision: "not-known".into(),
        onset_value: None,
        source: "patient-stated".into(),
        recorded_at: at,
        hlc: (at.timestamp_millis(), id as i32),
        signer: None,
        corrects: None,
        key: KeyState::Standing,
    };
    let a = PatientAllergies {
        entries: vec![row(10, linked, "penicillin"), row(11, linked, "codeine")],
        none: vec![NoneRow {
            event_id: Uuid::from_u128(20),
            chart: here,
            scope: "drug".into(),
            recorded_at: at,
            hlc: (at.timestamp_millis(), 20),
            signer: None,
            key: KeyState::Standing,
        }],
        challenges: vec![ChallengeRow {
            event_id: Uuid::from_u128(30),
            chart: linked,
            target: Uuid::from_u128(11),
            tolerated: "codeine".into(),
            date_precision: "not-known".into(),
            date_value: None,
            text: None,
            recorded_at: at,
            hlc: (at.timestamp_millis(), 30),
            signer: None,
            key: KeyState::Standing,
        }],
        retractions: vec![],
        doubted_charts,
        unreadable: 0,
    };
    let v = build_view(&a, chrono::FixedOffset::east_opt(0).unwrap());
    (a, v)
}

const DOUBTED: &str =
    "This line is on a chart whose link to this one is in doubt — open that chart, \
                       or settle the link, to change it.";

/// Review C1: on a chart set holding a doubted link, a line from the other chart may be a
/// stranger's. Over-warning is the safe error, so no act may take its warning away (or change it)
/// from here: every target on a doubted chart — a live line, a de-labelled line's entry or its
/// de-label, a "none known" line — is refused before the key, with a re-read. The opened chart's
/// own lines are unaffected.
#[test]
fn a_target_on_a_doubted_chart_is_refused_before_the_key() {
    let (here, linked) = (Uuid::from_u128(1), Uuid::from_u128(2));
    let (a, v) = overlay_fixture(vec![linked]);
    let on = |id: u128, names| target_on(&a, &v, &Uuid::from_u128(id).to_string(), names);
    for (id, names) in [
        (10, Names::Entry),
        (10, Names::Withdrawable),
        (11, Names::Withdrawable),
        (30, Names::Withdrawable),
    ] {
        let err = on(id, names).unwrap_err();
        assert_eq!(err.text, DOUBTED, "{id} {names:?}");
        assert_eq!(err.retry, Retry::AfterReread(Recorded::No));
    }
    assert_eq!(
        on(20, Names::Withdrawable),
        Ok(Target {
            id: Uuid::from_u128(20),
            chart: here
        }),
        "this chart's own none line"
    );
    // A none line on the doubted chart, too.
    let (mut a, _) = overlay_fixture(vec![linked]);
    a.none[0].chart = linked;
    let v = build_view(&a, chrono::FixedOffset::east_opt(0).unwrap());
    let err = target_on(
        &a,
        &v,
        &Uuid::from_u128(20).to_string(),
        Names::Withdrawable,
    )
    .unwrap_err();
    assert_eq!(err.text, DOUBTED);
}
