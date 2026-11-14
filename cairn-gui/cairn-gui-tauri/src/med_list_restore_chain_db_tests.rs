//! DB-gated walks for rulings R9, R10 and R12 (`med_list_restore::restore_holds`, the judge's
//! `replaces_a_restore` and `restores_a_stopped_line`): two restores of one lineage in one Sign,
//! the second refused before the key in words; a restore replacing a charted restore, or one
//! whose line was stopped since the press, never default-ticked. Nested under
//! `med_list_restore_db_tests.rs`, whose fixture and helpers these share (moved verbatim out of
//! it for its 500-line cap). Run locally on `CAIRN_TEST_PG` (#905).
use super::*;
use crate::med_list_sign::MedListSigned;

/// Ruling R9 (a): two hand-ticked restores of one line in one Sign: the first charts, the second
/// is refused before the key in words (never the floor's commit text) and stays in the list.
#[tokio::test]
async fn two_hand_ticked_restores_of_one_line_refuse_the_second_legibly() {
    let Some((_guard, owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let (companion, versions) =
        charted_with_versions(&state, &owner, chart, &["25mg tab bd", "75mg tab bd"]).await;
    let first = restore(&state, chart, &[chart], versions[0]).await.unwrap();
    let second = restore(&state, chart, &[chart], versions[1]).await.unwrap();
    for id in [&first.line_id, &second.line_id] {
        let line = hand_ticked(shown(
            second.list.as_ref().expect("the list read again"),
            id,
        ));
        assert_eq!(save_as(&state, chart, line, Some("bea")).await, "saved");
    }
    let ids = [first.line_id.clone(), second.line_id.clone()];
    let (outcome, charted, of, not_charted, _) =
        done(sign(&state, chart, &ids, credential("bea")).await.unwrap());
    assert_eq!((charted, of), (1, 2), "{outcome}");
    assert_eq!(
        not_charted,
        [(second.line_id.clone(), SAME_LINE_RESTORED.to_string())]
    );
    assert_eq!(
        stop_reason(&owner, companion).await,
        Some(Some("replaced by the GP letter version".into()))
    );
    assert_eq!(
        left(&owner, chart).await,
        [second.line_id.parse::<Uuid>().unwrap()]
    );
}

/// Ruling R9 (b): once a restore is charted, a second restore of the same line (which would stop
/// it) is never default-ticked, though it duplicates nothing; ticked by hand, it stops the first.
#[tokio::test]
async fn a_restore_replacing_a_charted_restore_is_never_default_ticked() {
    let Some((_guard, owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    allergies_known(&state, chart).await;
    let (_, versions) =
        charted_with_versions(&state, &owner, chart, &["25mg tab bd", "75mg tab bd"]).await;
    let first = restore(&state, chart, &[chart], versions[0]).await.unwrap();
    let (outcome, charted, _, _, _) = done(
        sign(
            &state,
            chart,
            &ticked(first.list.as_ref().expect("the list read again")),
            credential("bea"),
        )
        .await
        .unwrap(),
    );
    assert_eq!(charted, 1, "{outcome}");
    let second = restore(&state, chart, &[chart], versions[1]).await.unwrap();
    let line = shown(
        second.list.as_ref().expect("the list read again"),
        &second.line_id,
    );
    assert!(!line.duplicate, "{:?}", line.duplicate_of);
    assert!(!line.default_ticked && !line.ticked);
    let line = hand_ticked(line);
    assert_eq!(save_as(&state, chart, line, Some("bea")).await, "saved");
    let (outcome, charted, _, _, _) = done(
        sign(
            &state,
            chart,
            std::slice::from_ref(&second.line_id),
            credential("bea"),
        )
        .await
        .unwrap(),
    );
    assert_eq!(charted, 1, "{outcome}");
    let first_thread: String = owner
        .query_one(
            "SELECT medication_id::text FROM medication_restore WHERE version_id = $1::text::uuid",
            &[&versions[0].to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        stop_reason(&owner, first_thread.parse().unwrap()).await,
        Some(Some("replaced by the GP letter version".into()))
    );
}

/// Ruling R10, in the window: L1 restored v1 (beside X); v3 is set aside beside L1 while v2 is
/// still beside X. Restores of v2 (companion X) and v3 (companion L1) would both stop L1: in one
/// Sign the first charts and the second is refused before the key, legibly, and stays.
#[tokio::test]
async fn a_chain_restore_and_a_sibling_restore_refuse_the_second_legibly() {
    let Some((_guard, owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let (companion, versions) =
        charted_with_versions(&state, &owner, chart, &["25mg tab bd", "75mg tab bd"]).await;
    let first = restore(&state, chart, &[chart], versions[0]).await.unwrap();
    let line = hand_ticked(shown(
        first.list.as_ref().expect("the list read again"),
        &first.line_id,
    ));
    assert_eq!(save_as(&state, chart, line, Some("bea")).await, "saved");
    let ids = std::slice::from_ref(&first.line_id);
    let (outcome, charted, _, _, _) =
        done(sign(&state, chart, ids, credential("bea")).await.unwrap());
    assert_eq!(charted, 1, "{outcome}");
    let l1: String = owner
        .query_one(
            "SELECT medication_id::text FROM medication_restore WHERE version_id = $1::text::uuid",
            &[&versions[0].to_string()],
        )
        .await
        .unwrap()
        .get(0);
    // v3, set aside beside the restore line L1 (signed alone beside a charted line).
    let mut v3 = arg(
        ("metoprolol", "50mg tab bd"),
        pick("metoprolol", METOPROLOL),
        false,
    );
    v3.charted_as = "pre-existing".into();
    v3.info_source = Some("external-record".into());
    v3.set_aside_of = Some(format!("med:{l1}"));
    assert_eq!(save_as(&state, chart, v3, Some("bea")).await, "saved");
    let (outcome, _, _, _, _) = done(sign(&state, chart, &[], credential("bea")).await.unwrap());
    let v3: Uuid = owner
        .query_one(
            "SELECT version_id::text FROM medication_set_aside WHERE alternative_to = $1::text::uuid",
            &[&l1],
        )
        .await
        .unwrap_or_else(|e| panic!("{outcome}: {e}"))
        .get::<_, String>(0)
        .parse()
        .unwrap();
    let sibling = restore(&state, chart, &[chart], versions[1]).await.unwrap();
    let chain = restore(&state, chart, &[chart], v3).await.unwrap();
    for id in [&sibling.line_id, &chain.line_id] {
        let line = hand_ticked(shown(chain.list.as_ref().expect("the list read again"), id));
        assert_eq!(save_as(&state, chart, line, Some("bea")).await, "saved");
    }
    let ids = [sibling.line_id.clone(), chain.line_id.clone()];
    let (outcome, charted, of, not_charted, _) =
        done(sign(&state, chart, &ids, credential("bea")).await.unwrap());
    assert_eq!((charted, of), (1, 2), "{outcome}");
    assert_eq!(
        not_charted,
        [(chain.line_id.clone(), SAME_LINE_RESTORED.to_string())]
    );
    assert_eq!(
        left(&owner, chart).await,
        [chain.line_id.parse::<Uuid>().unwrap()]
    );
    assert!(stop_reason(&owner, companion).await.is_some());
    assert!(
        stop_reason(&owner, l1.parse().unwrap()).await.is_some(),
        "the sibling stopped L1"
    );
}

/// Ruling R12 (the chart-held rule): the companion is stopped by a separate cessation after the
/// press (myalgia, CK 5000). On the re-read the restore line is never default-ticked and its mark
/// says the line was stopped; ticked by hand, Sign charts it as a deliberate restart, stopping
/// nothing, and the colleague's reason stands.
#[tokio::test]
async fn a_restore_whose_line_was_stopped_since_is_never_default_ticked() {
    let Some((_guard, owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    allergies_known(&state, chart).await;
    let (companion, versions) =
        charted_with_versions(&state, &owner, chart, &["25mg tab bd"]).await;
    let opened = restore(&state, chart, &[chart], versions[0]).await.unwrap();
    assert!(
        shown(
            opened.list.as_ref().expect("the list read again"),
            &opened.line_id
        )
        .default_ticked
    );
    let patient = chart.to_string();
    crate::commands::cease_impl(
        &state,
        &patient,
        vec![patient.clone()],
        &companion.to_string(),
        "myalgia, CK 5000",
        credential("bea"),
    )
    .await
    .unwrap();
    let view = read(&state, chart).await;
    let line = shown(&view, &opened.line_id);
    assert!(!line.default_ticked && !line.ticked, "{line:?}");
    assert_eq!(
        line.restores_words.as_deref(),
        Some("restores the GP letter version — its line has since been stopped")
    );
    let line = hand_ticked(line);
    assert_eq!(save_as(&state, chart, line, Some("bea")).await, "saved");
    let ids = std::slice::from_ref(&opened.line_id);
    let (outcome, charted, _, not_charted, _) =
        done(sign(&state, chart, ids, credential("bea")).await.unwrap());
    assert_eq!(charted, 1, "{outcome}");
    assert!(not_charted.is_empty());
    assert_eq!(
        stop_reason(&owner, companion).await,
        Some(Some("myalgia, CK 5000".into())),
        "the colleague's stop stands"
    );
    let stops: i64 = owner
        .query_one(
            "SELECT count(*) FROM medication_cessation WHERE patient_id = $1::text::uuid",
            &[&patient],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(stops, 1, "the restore stopped nothing");
    assert!(left(&owner, chart).await.is_empty(), "it charted");
}

/// The chart-held rule for a HAND tick (the maintainer's ruling at the final check): a restore
/// line ticked by hand BEFORE a colleague stops its line is handed back unticked at Sign, nothing
/// signed — the tick vouched for the line that stood. Ticked again with the mark drawn, it charts
/// as a deliberate restart.
#[tokio::test]
async fn a_hand_tick_given_before_the_line_was_stopped_does_not_sign() {
    let Some((_guard, owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    allergies_known(&state, chart).await;
    let (companion, versions) =
        charted_with_versions(&state, &owner, chart, &["25mg tab bd"]).await;
    let opened = restore(&state, chart, &[chart], versions[0]).await.unwrap();
    let drawn = shown(
        opened.list.as_ref().expect("the list read again"),
        &opened.line_id,
    );
    assert_eq!(
        save_as(&state, chart, hand_ticked(drawn), Some("bea")).await,
        "saved"
    );
    let patient = chart.to_string();
    crate::commands::cease_impl(
        &state,
        &patient,
        vec![patient.clone()],
        &companion.to_string(),
        "myalgia, CK 5000",
        credential("bea"),
    )
    .await
    .unwrap();
    let ids = std::slice::from_ref(&opened.line_id);
    match sign(&state, chart, ids, credential("bea")).await.unwrap() {
        MedListSigned::Changed { lines, .. } => assert_eq!(lines, ids),
        other => panic!("expected Changed, got {other:?}"),
    }
    assert_eq!(left(&owner, chart).await.len(), 1, "nothing was charted");
    let view = read(&state, chart).await;
    let line = shown(&view, &opened.line_id);
    assert!(!line.ticked, "{line:?}");
    assert_eq!(
        save_as(&state, chart, hand_ticked(line), Some("bea")).await,
        "saved"
    );
    let (outcome, charted, _, _, _) =
        done(sign(&state, chart, ids, credential("bea")).await.unwrap());
    assert_eq!(charted, 1, "{outcome}");
    assert_eq!(
        stop_reason(&owner, companion).await,
        Some(Some("myalgia, CK 5000".into())),
        "the colleague's stop stands"
    );
}
