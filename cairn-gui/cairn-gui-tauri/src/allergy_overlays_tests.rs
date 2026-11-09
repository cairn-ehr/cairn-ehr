//! Tests for `allergy_overlays.rs` and the Correct path of `allergies.rs`: the outcome of a sign
//! that landed or was interrupted (pure), and DB-gated walks under the window's own login (on
//! `CAIRN_TEST_PG`, run locally: #905, CI never runs them) — a correction filed under its
//! target's chart, a de-label and two withdrawals read back, a stale target refused before the
//! key, the de-label's own refusals, and each act timed with its own kind.
use super::*;
use crate::allergies::{
    allergy_list_impl, allergy_none_impl, allergy_record_impl, sign_failure, written, Act,
    EntryInput,
};
use crate::allergy_pane::{AllergyPane, EnteredInErrorLine};
use crate::funnel::view::{Recorded, Retry};
use crate::notes::review::db_tests::{credential, supervised_note};
use cairn_event::allergy::{AllergyEntry, NoneScope};
use cairn_event::allergy_overlay::{AllergyChallenge, AllergyRetraction};

/// A complete entry, as the form sends it.
fn entry(term: &str) -> EntryInput {
    EntryInput {
        allergen_kind: Some("drug".into()),
        term: term.into(),
        kind: Some("allergy".into()),
        category: Some("anaphylaxis".into()),
        reaction_text: None,
        onset: Some("2014".into()),
        source: Some("seen-documented".into()),
        corrects: None,
    }
}

/// The same entry, sent as a correction of `target`.
fn correction(term: &str, target: &str) -> EntryInput {
    EntryInput {
        corrects: Some(target.into()),
        ..entry(term)
    }
}

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

/// How many timing samples of `kind` the table holds (it outlives the fixture's truncation).
async fn timed(owner: &tokio_postgres::Client, kind: &str) -> i64 {
    owner
        .query_one(
            "SELECT coalesce(sum(n), 0)::bigint FROM ui_gesture_timing WHERE gesture_kind = $1",
            &[&kind],
        )
        .await
        .unwrap()
        .get(0)
}

const GONE: &str = "That allergy line is no longer on this chart — read it again.";

/// D2: a correction of a linked chart's entry, made from the chart on screen, is filed under
/// THAT entry's chart — the floor admits a correction only of an entry on its own chart.
#[tokio::test]
async fn correct_files_under_the_targets_chart() {
    let Some((_guard, mut owner, state, chart, _)) = supervised_note().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let here = chart.to_string();
    // Bea signs one entry here, which leaves her key held for the setup below.
    allergy_record_impl(
        &state,
        &here,
        vec![here.clone()],
        entry("aspirin"),
        credential("bea"),
    )
    .await
    .unwrap();
    let (bea_sk, bea_kid) = state
        .signer_for_act(None, crate::state::Now::read())
        .await
        .unwrap();
    let node_sk = state.node_sk.as_ref().unwrap();
    let node_kid = hex::encode(node_sk.verifying_key().to_bytes());
    let origin = state.node_origin.clone();
    let (other, _, _) = cairn_node::john_doe::register_john_doe(
        &mut owner,
        node_sk,
        &node_kid,
        &origin,
        "ED",
        "test-site",
        "2026-11-07",
        "found wandering",
    )
    .await
    .unwrap();
    let author = cairn_node::medication::AuthorParams {
        human_sk: &bea_sk,
        human_kid: &bea_kid,
    };
    let typed = entry("penicillin");
    let read =
        crate::allergies::entry_before_the_key(&typed, crate::allergies::local_today()).unwrap();
    let theirs = cairn_node::allergy::sign::sign_allergy(
        &mut owner, node_sk, &node_kid, &origin, other, &read, &author,
    )
    .await
    .unwrap();
    let reviewer = cairn_node::chart_link::Reviewer {
        human_sk: &bea_sk,
        human_kid: &bea_kid,
    };
    cairn_node::chart_link::link_charts(&mut owner, chart, other, &reviewer, &origin)
        .await
        .unwrap();

    let pane = allergy_list_impl(&state, &here).await.unwrap();
    assert_eq!(pane.charts.len(), 2, "{:?}", pane.charts);
    let line = pane
        .drug_allergies
        .iter()
        .find(|l| l.event_id == theirs.to_string())
        .expect("the linked chart's entry is listed");
    assert_eq!(line.chart, other.to_string());
    let words = line.words.as_ref().expect("a live line carries its words");
    assert_eq!(words.term, "penicillin");
    assert_eq!(words.onset_value.as_deref(), Some("2014"));

    let done = allergy_record_impl(
        &state,
        &here,
        pane.charts.clone(),
        correction("penicillin V", &theirs.to_string()),
        None,
    )
    .await
    .unwrap();
    assert_eq!(done.outcome, "Recorded.");
    // The one correction on the node, read as the owner: filed under the target's chart.
    let filed: Vec<String> = owner
        .query(
            "SELECT patient_id::text FROM clinical_allergy WHERE refs @> jsonb_build_array(\
             jsonb_build_object('rel', 'correction-of', 'event', $1::text))",
            &[&theirs.to_string()],
        )
        .await
        .unwrap()
        .iter()
        .map(|r| r.get(0))
        .collect();
    assert_eq!(filed, vec![other.to_string()]);
    let pane = done.pane.expect("the list is read again");
    let head = pane
        .drug_allergies
        .iter()
        .find(|l| l.text.contains("penicillin V"))
        .expect("the correction is the head");
    assert_eq!(head.chart, other.to_string());
    assert_eq!(head.earlier.len(), 1, "{:?}", head.earlier);
    assert!(
        head.earlier[0].starts_with("Earlier: "),
        "{:?}",
        head.earlier
    );
    // The corrected version is no longer a line: correcting it again names a line not shown.
    let err = allergy_record_impl(
        &state,
        &here,
        pane.charts.clone(),
        correction("penicillin G", &theirs.to_string()),
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(err.text, GONE);
}

/// A de-label moves the line to "historical"; withdrawing the de-label brings it back; withdrawing
/// the entry, and a "none known" statement, lists each as "Entered in error". Each answer is the
/// list read again.
#[tokio::test]
async fn a_challenge_and_a_retraction_land_and_reread() {
    let Some((_guard, _owner, state, chart, _)) = supervised_note().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let here = chart.to_string();
    let charts = vec![here.clone()];
    let pane = allergy_record_impl(
        &state,
        &here,
        charts.clone(),
        entry("penicillin"),
        credential("bea"),
    )
    .await
    .unwrap()
    .pane
    .unwrap();
    let target = pane.drug_allergies[0].event_id.clone();

    let done = allergy_challenge_impl(
        &state,
        &here,
        charts.clone(),
        &target,
        " amoxicillin ",
        Some("03/2025"),
        Some("  "),
        None,
    )
    .await
    .unwrap();
    assert_eq!(done.outcome, "Recorded.");
    let pane = done.pane.unwrap();
    assert!(pane.drug_allergies.is_empty(), "{:?}", pane.drug_allergies);
    assert_eq!(pane.historical.len(), 1);
    let historical = &pane.historical[0];
    assert!(
        historical.text.contains("tolerated amoxicillin"),
        "{}",
        historical.text
    );
    assert!(
        historical.words.is_none(),
        "a historical line carries no words to correct"
    );
    let challenge = historical
        .challenge
        .clone()
        .expect("the de-label it rests on");
    // A de-labelled line is no head: neither corrected nor de-labelled again.
    let err = allergy_challenge_impl(
        &state,
        &here,
        charts.clone(),
        &target,
        "amoxicillin",
        None,
        None,
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(err.text, GONE);
    assert_eq!(err.retry, Retry::AfterReread(Recorded::No));

    // Withdraw the de-label: the line is live again.
    let pane = allergy_retract_impl(
        &state,
        &here,
        charts.clone(),
        &challenge,
        Some("wrong patient"),
        None,
    )
    .await
    .unwrap()
    .pane
    .unwrap();
    assert_eq!(pane.drug_allergies.len(), 1);
    assert!(pane.historical.is_empty());
    assert_eq!(
        pane.entered_in_error.len(),
        1,
        "{:?}",
        pane.entered_in_error
    );
    // Withdraw the entry itself.
    let pane = allergy_retract_impl(&state, &here, charts.clone(), &target, None, None)
        .await
        .unwrap()
        .pane
        .unwrap();
    assert!(pane.drug_allergies.is_empty());
    assert_eq!(
        pane.entered_in_error.len(),
        2,
        "{:?}",
        pane.entered_in_error
    );
    assert!(pane
        .entered_in_error
        .iter()
        .any(|l| l.text.starts_with("Entered in error: ") && l.text.contains("penicillin")));

    // A "none known" statement is withdrawn by its line's event id.
    let pane = allergy_none_impl(&state, &here, charts.clone(), "all", None)
        .await
        .unwrap()
        .pane
        .unwrap();
    let none = pane.standing_none[0].event_id.clone();
    assert!(pane.standing_none[0]
        .text
        .starts_with("No known allergies — asked "));
    let pane = allergy_retract_impl(&state, &here, charts, &none, None, None)
        .await
        .unwrap()
        .pane
        .unwrap();
    assert!(pane.standing_none.is_empty());
    assert_eq!(pane.strip, "Allergies not yet asked");
    assert_eq!(
        pane.entered_in_error.len(),
        3,
        "{:?}",
        pane.entered_in_error
    );
}

/// A target the fresh read does not show — here one that never existed — is refused before the
/// key: no key is held and none is typed, so a check after the key would say the key's sentence.
#[tokio::test]
async fn a_target_not_in_the_fresh_read_is_refused_before_the_key() {
    let Some((_guard, _owner, state, chart, part)) = supervised_note().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let here = chart.to_string();
    let charts = vec![here.clone()];
    // A real event on this chart that is no allergy line (the fixture's note), and a stranger.
    for target in [
        part.to_string(),
        Uuid::now_v7().to_string(),
        "not-a-uuid".into(),
    ] {
        let errs = [
            allergy_record_impl(
                &state,
                &here,
                charts.clone(),
                correction("penicillin", &target),
                None,
            )
            .await
            .unwrap_err(),
            allergy_challenge_impl(
                &state,
                &here,
                charts.clone(),
                &target,
                "amoxicillin",
                None,
                None,
                None,
            )
            .await
            .unwrap_err(),
            allergy_retract_impl(&state, &here, charts.clone(), &target, None, None)
                .await
                .unwrap_err(),
        ];
        for err in errs {
            assert_eq!(err.text, GONE, "{target}");
            assert_eq!(err.retry, Retry::AfterReread(Recorded::No));
        }
    }
    // The wrong set is said before the target.
    let other = Uuid::from_u128(424_242).to_string();
    let err = allergy_retract_impl(&state, &here, vec![other], &part.to_string(), None, None)
        .await
        .unwrap_err();
    assert_ne!(err.text, GONE);
    assert_eq!(err.retry, Retry::AfterReread(Recorded::No));
}

/// The de-label's own refusals, said before the target is looked up and before the key: nothing
/// tolerated, a date that cannot be read, a date after today.
#[tokio::test]
async fn a_challenge_needs_what_was_tolerated() {
    let Some((_guard, _owner, state, chart, _)) = supervised_note().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let here = chart.to_string();
    let charts = vec![here.clone()];
    let stranger = Uuid::now_v7().to_string();
    let say = async |tolerated: &str, date: Option<&str>| {
        allergy_challenge_impl(
            &state,
            &here,
            charts.clone(),
            &stranger,
            tolerated,
            date,
            None,
            None,
        )
        .await
        .unwrap_err()
        .text
    };
    assert_eq!(say("  ", Some("2019")).await, "Name what was tolerated.");
    assert!(say("amoxicillin", Some("spring 2019"))
        .await
        .starts_with("\"spring 2019\" is not a date"),);
    assert_eq!(
        say("amoxicillin", Some("2999")).await,
        "\"2999\" is after today — when it happened cannot be later than today."
    );
    // A well-formed de-label of a line not shown is the target's refusal, not the form's.
    assert_eq!(say("amoxicillin", None).await, GONE);
}

/// db/044's three slice-2 kinds: a correction is timed as one (never as a record), a de-label and
/// a withdrawal as theirs.
#[tokio::test]
async fn each_act_is_timed_with_its_own_kind() {
    let Some((_guard, owner, state, chart, _)) = supervised_note().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let here = chart.to_string();
    let charts = vec![here.clone()];
    let pane = allergy_record_impl(
        &state,
        &here,
        charts.clone(),
        entry("penicillin"),
        credential("bea"),
    )
    .await
    .unwrap()
    .pane
    .unwrap();
    let kinds = [
        "allergy-record",
        "allergy-correct",
        "allergy-challenge",
        "allergy-retract",
    ];
    let mut before = vec![];
    for k in kinds {
        before.push(timed(&owner, k).await);
    }
    let first = pane.drug_allergies[0].event_id.clone();
    let pane = allergy_record_impl(
        &state,
        &here,
        charts.clone(),
        correction("penicillin V", &first),
        None,
    )
    .await
    .unwrap()
    .pane
    .unwrap();
    let head = pane.drug_allergies[0].event_id.clone();
    let pane = allergy_challenge_impl(
        &state,
        &here,
        charts.clone(),
        &head,
        "amoxicillin",
        None,
        None,
        None,
    )
    .await
    .unwrap()
    .pane
    .unwrap();
    let challenge = pane.historical[0].challenge.clone().unwrap();
    allergy_retract_impl(&state, &here, charts, &challenge, None, None)
        .await
        .unwrap();
    for (k, was) in kinds.iter().zip(before) {
        let expected = if *k == "allergy-record" { was } else { was + 1 };
        assert_eq!(timed(&owner, k).await, expected, "{k}");
    }
}

/// The target rule at its pure seam (runs in CI, unlike the DB walks): Correct and De-label name a
/// live line only; Entered in error also names a "none known" line or the de-label a historical
/// line rests on — never the de-labelled entry itself. Each is filed under its own row's chart.
#[test]
fn a_target_names_only_what_its_act_may_name() {
    use cairn_node::allergy::read::{AllergyRow, ChallengeRow, KeyState, NoneRow};
    let at = chrono::DateTime::from_timestamp(1_762_600_000, 0).unwrap();
    let (here, linked) = (Uuid::from_u128(1), Uuid::from_u128(2));
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
        doubted_charts: vec![],
        unreadable: 0,
    };
    let v = build_view(&a, chrono::FixedOffset::east_opt(0).unwrap());
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
        assert_eq!(on(11, names).unwrap_err().text, GONE, "a de-labelled entry");
        assert_eq!(on(99, names).unwrap_err().text, GONE, "a stranger");
        let garbage = target_on(&a, &v, "not-a-uuid", names).unwrap_err();
        assert_eq!(garbage.retry, Retry::AfterReread(Recorded::No));
    }
    assert_eq!(on(20, Names::Entry).unwrap_err().text, GONE);
    assert_eq!(on(30, Names::Entry).unwrap_err().text, GONE);
    assert_eq!(on(20, Names::Withdrawable), filed(20, here), "a none line");
    assert_eq!(
        on(30, Names::Withdrawable),
        filed(30, linked),
        "the de-label"
    );
}
