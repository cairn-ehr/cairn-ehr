//! DB-gated walks for `allergy_overlays.rs` and the Correct path of `allergies.rs`, under the
//! window's own login (on `CAIRN_TEST_PG`, run locally: #905, CI never runs them) — a correction
//! filed under its target's chart, a de-label and two withdrawals read back, a de-labelled line
//! struck in one act, a stale target and a doubted chart's target refused before the key, the
//! de-label's own refusals, and each act timed with its own kind. The pure tests are in
//! `allergy_overlays_pure_tests.rs`.
use super::*;
use crate::allergies::{allergy_list_impl, allergy_none_impl, allergy_record_impl, EntryInput};
use crate::funnel::view::{Recorded, Retry};
use crate::notes::review::db_tests::{credential, supervised_note};

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

/// A John Doe chart linked to `chart`, holding one penicillin entry Bea signed on it (her key is
/// left held, so the caller's acts sign with no credential). Answers (that chart, its entry).
async fn linked_entry(
    state: &AppState,
    owner: &mut tokio_postgres::Client,
    chart: Uuid,
) -> (Uuid, Uuid) {
    let here = chart.to_string();
    // Bea signs one entry here, which leaves her key held for the setup below.
    allergy_record_impl(
        state,
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
        owner,
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
        owner, node_sk, &node_kid, &origin, other, &read, &author,
    )
    .await
    .unwrap();
    let reviewer = cairn_node::chart_link::Reviewer {
        human_sk: &bea_sk,
        human_kid: &bea_kid,
    };
    cairn_node::chart_link::link_charts(owner, chart, other, &reviewer, &origin)
        .await
        .unwrap();

    (other, theirs)
}

/// D2: a correction of a linked chart's entry, made from the chart on screen, is filed under
/// THAT entry's chart — the floor admits a correction only of an entry on its own chart.
#[tokio::test]
async fn correct_files_under_the_targets_chart() {
    let Some((_guard, mut owner, state, chart, _)) = supervised_note().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let here = chart.to_string();
    let (other, theirs) = linked_entry(&state, &mut owner, chart).await;
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

/// Review C1 against a database: once the link to the other chart is in doubt (a veto flag), its
/// line is still listed — marked, with no act — and every act aimed at it from this chart is
/// refused before the key with a re-read: Correct, De-label and Entered in error. Nothing is
/// signed.
#[tokio::test]
async fn an_act_on_a_doubted_charts_line_is_refused_before_the_key() {
    let Some((_guard, mut owner, state, chart, _)) = supervised_note().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let here = chart.to_string();
    let (other, theirs) = linked_entry(&state, &mut owner, chart).await;
    let (lo, hi) = (chart.min(other), chart.max(other));
    owner
        .execute(
            "INSERT INTO link_veto_flag (low, high, content_address) \
             SELECT low, high, content_address FROM patient_link \
             WHERE low = $1::text::uuid AND high = $2::text::uuid",
            &[&lo.to_string(), &hi.to_string()],
        )
        .await
        .unwrap();
    let pane = allergy_list_impl(&state, &here).await.unwrap();
    let line = pane
        .drug_allergies
        .iter()
        .find(|l| l.event_id == theirs.to_string())
        .expect("a doubted chart's entry is still shown (over-warning)");
    assert!(line.doubted_chart);
    let target = theirs.to_string();
    let errs = [
        allergy_record_impl(
            &state,
            &here,
            pane.charts.clone(),
            correction("penicillin V", &target),
            None,
        )
        .await
        .unwrap_err(),
        allergy_challenge_impl(
            &state,
            &here,
            pane.charts.clone(),
            &target,
            "amoxicillin",
            None,
            None,
            None,
        )
        .await
        .unwrap_err(),
        allergy_retract_impl(&state, &here, pane.charts.clone(), &target, None, None)
            .await
            .unwrap_err(),
    ];
    for err in errs {
        assert_eq!(err.text, ON_A_DOUBTED_CHART);
        assert_eq!(err.retry, Retry::AfterReread(Recorded::No));
    }
    let signed: i64 = owner
        .query_one(
            "SELECT count(*) FROM event_log WHERE patient_id = ANY($1::text[]::uuid[]) \
             AND event_type LIKE 'clinical.allergy%'",
            &[&vec![here.clone(), other.to_string()]],
        )
        .await
        .unwrap()
        .get(0);
    // Only the two entries the setup signed (aspirin here, penicillin there).
    assert_eq!(signed, 2, "nothing was signed");
}

/// TBD-2 against a database: a de-labelled line is struck as entered in error in ONE act, aimed
/// at its own entry; the struck line still says the de-label it carried, and the reason.
#[tokio::test]
async fn a_delabelled_line_is_struck_in_one_act() {
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
    let pane = allergy_challenge_impl(
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
    .unwrap()
    .pane
    .unwrap();
    assert_eq!(pane.historical[0].event_id, target);
    let pane = allergy_retract_impl(&state, &here, charts, &target, Some("wrong patient"), None)
        .await
        .unwrap()
        .pane
        .unwrap();
    assert!(pane.historical.is_empty() && pane.drug_allergies.is_empty());
    assert_eq!(
        pane.entered_in_error.len(),
        1,
        "{:?}",
        pane.entered_in_error
    );
    let text = &pane.entered_in_error[0].text;
    assert!(
        text.starts_with("Entered in error: Allergy: penicillin"),
        "{text}"
    );
    assert!(text.contains(") — wrong patient — de-labelled "), "{text}");
    assert!(text.ends_with(": tolerated amoxicillin"), "{text}");
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
