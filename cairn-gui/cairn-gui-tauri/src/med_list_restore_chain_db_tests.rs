//! DB-gated walks for rulings R9 and R10 (`med_list_restore::restore_holds`, the judge's
//! `replaces_a_restore`): two restores of one lineage in one Sign, the second refused before the
//! key in words; a restore replacing a charted restore never default-ticked. Nested under
//! `med_list_restore_db_tests.rs`, whose fixture and helpers these share (moved verbatim out of
//! it for its 500-line cap). Run locally on `CAIRN_TEST_PG` (#905).
use super::*;

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
        let line = hand_ticked(shown(&second.list, id));
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
        sign(&state, chart, &ticked(&first.list), credential("bea"))
            .await
            .unwrap(),
    );
    assert_eq!(charted, 1, "{outcome}");
    let second = restore(&state, chart, &[chart], versions[1]).await.unwrap();
    let line = shown(&second.list, &second.line_id);
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
    let line = hand_ticked(shown(&first.list, &first.line_id));
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
        let line = hand_ticked(shown(&chain.list, id));
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
