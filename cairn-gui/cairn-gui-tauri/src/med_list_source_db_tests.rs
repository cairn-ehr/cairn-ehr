//! DB-gated walks for a list line's source and why (batch charting slice 2a, design §3.1):
//! "From:" stamps the lines saved after it, a line may name its own source, and both keys are
//! signed into the line's event; a save that sends the line back as drawn keeps them and its
//! tick. Beside `med_list_versions_db_tests.rs` (whose helpers these share) to keep each file
//! under 500 lines. Run locally on `CAIRN_TEST_PG` (#905: CI never runs these).
use super::db_tests::{arg, bea_list, done, here, read, save, save_as, sign};
use super::versions_db_tests::{shown, statements};
use super::*;
use crate::med_list_versions::med_list_from_save_impl;
use crate::notes::review::db_tests::credential;
use tokio_postgres::Client;

/// `(source_detail, why)` projected for the charted line `medication_id`.
async fn charting_note(owner: &Client, medication_id: &str) -> (Option<String>, Option<String>) {
    let r = owner
        .query_one(
            "SELECT source_detail, why FROM medication_charting WHERE medication_id = $1::text::uuid",
            &[&medication_id],
        )
        .await
        .unwrap();
    (r.get(0), r.get(1))
}

/// "From:" set once stamps each line saved after it (its category, and the document named); a
/// line given its own source keeps it; both are signed into their events.
#[tokio::test]
async fn from_sets_each_new_lines_source_and_a_line_can_change_its_own() {
    let Some((_guard, owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let patient = chart.to_string();
    let view = med_list_from_save_impl(
        &state,
        &patient,
        std::slice::from_ref(&patient),
        Some("GP letter".into()),
        Some("dated Aug 2025".into()),
        Some("bea"),
    )
    .await
    .unwrap();
    assert_eq!(
        view.from,
        Some(FromView {
            preset: Some("GP letter".into()),
            detail: Some("dated Aug 2025".into()),
        })
    );
    let first = save(&state, chart, ("metformin", "500mg bd"), vec![], true).await;
    let mut own = arg(("aspirin", "100mg daily"), vec![], true);
    own.charted_as = "pre-existing".into();
    own.info_source = Some("patient-reported".into());
    own.source_detail = Some("daughter".into());
    let own_id = own.line_id.clone();
    assert_eq!(save_as(&state, chart, own, Some("bea")).await, "saved");
    let view = read(&state, chart).await;
    let f = shown(&view, &first.to_string());
    assert_eq!(
        (f.info_source.as_deref(), f.source_detail.as_deref()),
        (Some("external-record"), Some("dated Aug 2025"))
    );
    let o = shown(&view, &own_id);
    assert_eq!(
        (o.info_source.as_deref(), o.source_detail.as_deref()),
        (Some("patient-reported"), Some("daughter"))
    );
    let (outcome, charted, _, _, _) = done(
        sign(
            &state,
            chart,
            &[first.to_string(), own_id],
            credential("bea"),
        )
        .await
        .unwrap(),
    );
    assert_eq!(charted, 2, "{outcome}");
    let st = statements(&owner, chart).await;
    // ORDER BY dose_amount: aspirin 100 before metformin 500.
    assert_eq!(st[0].2, "patient-reported");
    assert_eq!(st[1].2, "external-record");
    assert_eq!(
        charting_note(&owner, &st[0].0).await,
        (Some("daughter".into()), None)
    );
    assert_eq!(
        charting_note(&owner, &st[1].0).await,
        (Some("dated Aug 2025".into()), None)
    );
}

/// With "From:" never set, each line signs the press's default and no source detail.
#[tokio::test]
async fn unset_from_keeps_the_press_default() {
    let Some((_guard, owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let start = save(&state, chart, ("aspirin", "100mg daily"), vec![], true).await;
    let mut taking = arg(("metformin", "500mg bd"), vec![], true);
    taking.charted_as = "pre-existing".into();
    let taking_id = taking.line_id.clone();
    assert_eq!(save_as(&state, chart, taking, Some("bea")).await, "saved");
    let view = read(&state, chart).await;
    assert_eq!(view.from, None);
    assert_eq!(shown(&view, &start.to_string()).info_source, None);
    done(
        sign(
            &state,
            chart,
            &[start.to_string(), taking_id],
            credential("bea"),
        )
        .await
        .unwrap(),
    );
    let st = statements(&owner, chart).await;
    assert_eq!(st[0].2, "clinician-observed");
    assert_eq!(st[1].2, "patient-reported");
    assert_eq!(charting_note(&owner, &st[0].0).await, (None, None));
}

/// The line's why is signed into its event (and a blank one is no why at all).
#[tokio::test]
async fn a_why_is_signed_into_the_event() {
    let Some((_guard, owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let mut line = arg(("aspirin", "100mg daily"), vec![], true);
    line.why = Some("packet 25 mg, daughter confirms".into());
    let mut blank = arg(("metformin", "500mg bd"), vec![], true);
    blank.why = Some("   ".into());
    let ids = [line.line_id.clone(), blank.line_id.clone()];
    assert_eq!(save_as(&state, chart, line, Some("bea")).await, "saved");
    assert_eq!(save_as(&state, chart, blank, Some("bea")).await, "saved");
    let (outcome, charted, _, _, _) =
        done(sign(&state, chart, &ids, credential("bea")).await.unwrap());
    assert_eq!(charted, 2, "{outcome}");
    let st = statements(&owner, chart).await;
    assert_eq!(
        charting_note(&owner, &st[0].0).await,
        (None, Some("packet 25 mg, daughter confirms".into()))
    );
    assert_eq!(charting_note(&owner, &st[1].0).await, (None, None));
}

/// The save door reads an omitted source as cleared (Task 4's carry), so the window always sends
/// what the line holds: a save that does not touch the source keeps it, and keeps the hand tick.
#[tokio::test]
async fn a_save_that_does_not_touch_the_source_keeps_it_and_the_tick() {
    let Some((_guard, _owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let mut line = arg(("aspirin", "100mg daily"), vec![], true);
    line.info_source = Some("external-record".into());
    line.source_detail = Some("GP letter".into());
    line.why = Some("as the letter says".into());
    let id = line.line_id.clone();
    assert_eq!(save_as(&state, chart, line, Some("bea")).await, "saved");
    let v = read(&state, chart).await;
    let drawn = shown(&v, &id);
    // The webview sends back the line as drawn: the same words, the same four values.
    let again = LineArg {
        line_id: drawn.line_id.clone(),
        charted_as: drawn.charted_as.clone(),
        line_text: drawn.line_text.clone(),
        parts: drawn.parts.clone(),
        typed_drug: drawn.typed_drug.clone(),
        directions: drawn.directions.clone(),
        reason: drawn.reason.clone(),
        hand_ticked: drawn.hand_ticked,
        finished: drawn.finished,
        seen: drawn.against.clone(),
        held: drawn.held,
        info_source: drawn.info_source.clone(),
        source_detail: drawn.source_detail.clone(),
        why: drawn.why.clone(),
        set_aside_of: drawn.set_aside_of.clone(),
    };
    assert_eq!(save_as(&state, chart, again, Some("bea")).await, "saved");
    let v = read(&state, chart).await;
    let after = shown(&v, &id);
    assert!(after.hand_ticked && after.ticked, "the tick stands");
    assert_eq!(after.source_detail.as_deref(), Some("GP letter"));
    assert_eq!(after.why.as_deref(), Some("as the letter says"));
}

/// Ruling R2: "From:" picked on Bea's list, landing after who's here switched to Sue, files under
/// Bea, whose list was read; Sue's list gets no source she never chose.
#[tokio::test]
async fn a_from_save_after_a_switch_files_under_the_read_writer() {
    let Some((_guard, owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let writer = read(&state, chart).await.writer;
    assert_eq!(writer.as_deref(), Some("bea"));
    here(&state, "sue", "Dr Sue").await;
    let patient = chart.to_string();
    med_list_from_save_impl(
        &state,
        &patient,
        std::slice::from_ref(&patient),
        Some("GP letter".into()),
        None,
        writer.as_deref(),
    )
    .await
    .unwrap();
    let filed: Vec<(Option<String>, Option<String>)> = owner
        .query(
            "SELECT writer, preset FROM medication_draft_from WHERE patient_id = $1::text::uuid",
            &[&patient],
        )
        .await
        .unwrap()
        .iter()
        .map(|r| (r.get(0), r.get(1)))
        .collect();
    assert_eq!(filed, [(Some("bea".into()), Some("GP letter".into()))]);
    assert_eq!(read(&state, chart).await.from, None, "never Sue's");
}

/// Ruling R12: "From:" stamps only a line typed after it. A line typed before it, saved again
/// (as drawn, then with a typo fixed), keeps its own empty source, and the unchanged save keeps
/// its hand tick: it never signs a provenance nobody gave.
#[tokio::test]
async fn editing_a_line_typed_before_from_keeps_its_own_source_and_tick() {
    let Some((_guard, _owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let mut line = arg(("metfromin", "500mg bd"), vec![], true);
    let id = line.line_id.clone();
    assert_eq!(
        save_as(&state, chart, line.clone(), Some("bea")).await,
        "saved"
    );
    let patient = chart.to_string();
    med_list_from_save_impl(
        &state,
        &patient,
        std::slice::from_ref(&patient),
        Some("GP letter".into()),
        None,
        Some("bea"),
    )
    .await
    .unwrap();
    assert_eq!(
        save_as(&state, chart, line.clone(), Some("bea")).await,
        "saved"
    );
    let v = read(&state, chart).await;
    let drawn = shown(&v, &id);
    assert_eq!(
        (drawn.info_source.as_deref(), drawn.source_detail.as_deref()),
        (None, None)
    );
    assert!(drawn.hand_ticked && drawn.ticked, "the tick stands");
    line.typed_drug = "metformin".into();
    line.line_text = "metformin 500mg bd".into();
    assert_eq!(save_as(&state, chart, line, Some("bea")).await, "saved");
    let v = read(&state, chart).await;
    assert_eq!(
        shown(&v, &id).source_detail,
        None,
        "a typo fix stamps nothing"
    );
}

/// Ruling R23: the save that stamps "From:" on a new line says so in its answer, so the window can
/// carry the stamp on the line's next save even before a re-read lands (else that save would send
/// no source, and the door would clear the stamp). A save that stamped nothing answers no source.
#[tokio::test]
async fn the_save_that_stamps_from_answers_the_stamp() {
    let Some((_guard, _owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let patient = chart.to_string();
    let charts = std::slice::from_ref(&patient);
    med_list_from_save_impl(
        &state,
        &patient,
        charts,
        Some("GP letter".into()),
        None,
        Some("bea"),
    )
    .await
    .unwrap();
    let line = arg(("metformin", "500mg bd"), vec![], false);
    let again = line.clone();
    let first = med_list_save_impl(&state, &patient, charts, line, Some("bea"))
        .await
        .unwrap();
    assert_eq!(
        (
            first.answer.as_str(),
            first.info_source.as_deref(),
            first.source_detail.as_deref()
        ),
        ("saved", Some("external-record"), Some("GP letter"))
    );
    // The same line again, still sent with no source (the read has not landed): stored, so no stamp.
    let second = med_list_save_impl(&state, &patient, charts, again, Some("bea"))
        .await
        .unwrap();
    assert_eq!((second.info_source, second.source_detail), (None, None));
}
