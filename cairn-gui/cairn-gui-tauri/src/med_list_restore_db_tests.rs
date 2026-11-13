//! DB-gated walks for batch charting slice 2b's "Use this version instead" (`med_list_restore.rs`):
//! the press adds a pre-filled line carrying `restores`, and the list's one Sign stops what stands
//! for the companion and charts the line in one transaction. On `med_list_db_tests.rs`'s fixture
//! (Dr Bea named, a John Doe chart), whose helpers these share. Run locally on `CAIRN_TEST_PG`
//! (#905: CI never runs these). The check's walks are `med_version_check_db_tests.rs`.
use super::db_tests::{arg, bea_list, done, left, pick, read, save, save_as, sign, ticked};
use super::versions_db_tests::{shown, statements, METOPROLOL};
use super::*;
use crate::allergies::allergy_none_impl;
use crate::med_list_restore::{
    med_list_restore_open_impl, RestoreOpened, LIST_ELSEWHERE, RESTORE_VERSION_GONE,
    SAME_LINE_RESTORED, VERSION_GONE,
};
use crate::notes::review::db_tests::credential;
use tokio_postgres::Client;

#[path = "med_version_check_db_tests.rs"]
mod check_db_tests;

/// Bea's "None known" for every allergy kind, so a complete coded line can take the default tick.
pub(super) async fn allergies_known(state: &AppState, chart: Uuid) {
    let patient = chart.to_string();
    allergy_none_impl(
        state,
        &patient,
        vec![patient.clone()],
        "all",
        credential("bea"),
    )
    .await
    .unwrap();
}

/// Chart metoprolol 50mg bd on `chart` beside a set-aside version per dose in `doses` (each as a
/// GP letter reported it): one Sign. Answers the companion's thread and the versions' ids, in
/// `doses`' order.
pub(super) async fn charted_with_versions(
    state: &AppState,
    owner: &Client,
    chart: Uuid,
    doses: &[&str],
) -> (Uuid, Vec<Uuid>) {
    let a = save(
        state,
        chart,
        ("metoprolol", "50mg bd"),
        pick("metoprolol", METOPROLOL),
        true,
    )
    .await;
    for dose in doses {
        let mut v = arg(("metoprolol", dose), pick("metoprolol", METOPROLOL), false);
        v.charted_as = "pre-existing".into();
        v.info_source = Some("external-record".into());
        v.source_detail = Some("GP letter".into());
        v.set_aside_of = Some(format!("line:{a}"));
        assert_eq!(save_as(state, chart, v, Some("bea")).await, "saved");
    }
    let (outcome, charted, _, _, _) = done(
        sign(state, chart, &[a.to_string()], credential("bea"))
            .await
            .unwrap(),
    );
    assert_eq!(charted, 1, "{outcome}");
    let companion: Uuid = statements(owner, chart).await[0].0.parse().unwrap();
    let mut versions = vec![];
    for dose in doses {
        let amount = dose
            .split_whitespace()
            .next()
            .unwrap()
            .trim_end_matches("mg");
        let row = owner
            .query_one(
                "SELECT version_id::text FROM medication_set_aside \
                  WHERE patient_id = $1::text::uuid AND dose_amount = $2",
                &[&chart.to_string(), &amount],
            )
            .await
            .unwrap();
        versions.push(row.get::<_, String>(0).parse().unwrap());
    }
    (companion, versions)
}

/// Press "Use this version instead" on `version`, as the webview sends it.
pub(super) async fn restore(
    state: &AppState,
    chart: Uuid,
    charts: &[Uuid],
    version: Uuid,
) -> Result<RestoreOpened, ErrorView> {
    let charts = charts.iter().map(Uuid::to_string).collect();
    med_list_restore_open_impl(state, &chart.to_string(), charts, &version.to_string()).await
}

/// The cessation reason of `thread` (`None`: not stopped).
async fn stop_reason(owner: &Client, thread: Uuid) -> Option<Option<String>> {
    owner
        .query_opt(
            "SELECT reason FROM medication_cessation WHERE medication_id = $1::text::uuid",
            &[&thread.to_string()],
        )
        .await
        .unwrap()
        .map(|r| r.get(0))
}

/// `l` saved again ticked by hand, having seen what it was drawn with.
fn hand_ticked(l: &ListLineView) -> LineArg {
    LineArg {
        line_id: l.line_id.clone(),
        charted_as: l.charted_as.clone(),
        line_text: l.line_text.clone(),
        parts: l.parts.clone(),
        typed_drug: l.typed_drug.clone(),
        directions: l.directions.clone(),
        reason: l.reason.clone(),
        hand_ticked: true,
        finished: l.finished,
        seen: l.against.clone(),
        held: false,
        info_source: l.info_source.clone(),
        source_detail: l.source_detail.clone(),
        why: l.why.clone(),
        set_aside_of: l.set_aside_of.clone(),
    }
}

/// The press adds one finished line, pre-filled from the version, inheriting the companion's
/// press, marked with the version it restores; a list held on another terminal adds nothing.
#[tokio::test]
async fn restore_open_adds_a_prefilled_line_with_its_mark() {
    let Some((_guard, owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let (_, versions) = charted_with_versions(&state, &owner, chart, &["25mg tab bd"]).await;
    let opened = restore(&state, chart, &[chart], versions[0]).await.unwrap();
    let line = shown(&opened.list, &opened.line_id);
    assert_eq!(line.typed_drug, "metoprolol");
    assert_eq!(line.directions, "25mg tab bd");
    assert_eq!(line.line_text, "metoprolol 25mg tab bd");
    assert_eq!(line.charted_as, "started-here", "the companion's press");
    assert!(line.finished && line.parts.len() == 1);
    assert_eq!(
        line.restores_words.as_deref(),
        Some("restores the GP letter version")
    );
    assert!(line.info_source.is_none(), "\"From:\" never stamps it");
    assert_eq!(
        left(&owner, chart).await,
        [opened.line_id.parse::<Uuid>().unwrap()]
    );
    owner
        .execute(
            "UPDATE medication_draft_line SET terminal_id = 'pc-other'",
            &[],
        )
        .await
        .unwrap();
    let held = restore(&state, chart, &[chart], versions[0])
        .await
        .unwrap_err();
    assert_eq!(held.text, format!("{LIST_ELSEWHERE}pc-other"));
    assert_eq!(left(&owner, chart).await.len(), 1, "nothing was added");
}

/// Spec §2: a linked chart's version is never restored from this chart: refused, nothing added.
#[tokio::test]
async fn restore_open_refuses_a_linked_charts_version() {
    let Some((_guard, mut owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let node_sk = state.node_sk.as_ref().unwrap();
    let node_kid = hex::encode(node_sk.verifying_key().to_bytes());
    let (other, _, _) = cairn_node::john_doe::register_john_doe(
        &mut owner,
        node_sk,
        &node_kid,
        &state.node_origin,
        "ED",
        "test-site",
        "2026-11-14",
        "found wandering",
    )
    .await
    .unwrap();
    *state.chart.lock().await = Some(crate::funnel::window::OpenChart::by_id(other));
    let (_, theirs) = charted_with_versions(&state, &owner, other, &["25mg tab bd"]).await;
    *state.chart.lock().await = Some(crate::funnel::window::OpenChart::by_id(chart));
    let (bea_sk, bea_kid) = state
        .signer_for_act(None, crate::state::Now::read())
        .await
        .unwrap();
    let reviewer = cairn_node::chart_link::Reviewer {
        human_sk: &bea_sk,
        human_kid: &bea_kid,
    };
    cairn_node::chart_link::link_charts(&mut owner, chart, other, &reviewer, &state.node_origin)
        .await
        .unwrap();
    let refused = restore(&state, chart, &[chart, other], theirs[0])
        .await
        .unwrap_err();
    assert_eq!(refused.text, VERSION_GONE);
    assert!(left(&owner, chart).await.is_empty(), "nothing was added");
}

/// The morning act: the restore line takes the default tick, and ONE Sign stops the companion
/// with the plan's reason and charts the line, signing `unknown` (no source of its own).
#[tokio::test]
async fn one_sign_stops_the_companion_and_charts_the_restore() {
    let Some((_guard, owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    allergies_known(&state, chart).await;
    let (companion, versions) =
        charted_with_versions(&state, &owner, chart, &["25mg tab bd"]).await;
    let opened = restore(&state, chart, &[chart], versions[0]).await.unwrap();
    assert_eq!(ticked(&opened.list), std::slice::from_ref(&opened.line_id));
    let (outcome, charted, of, not_charted, _) = done(
        sign(&state, chart, &ticked(&opened.list), credential("bea"))
            .await
            .unwrap(),
    );
    assert_eq!((charted, of), (1, 1), "{outcome}");
    assert!(not_charted.is_empty());
    assert_eq!(
        stop_reason(&owner, companion).await,
        Some(Some("replaced by the GP letter version".into()))
    );
    let st = statements(&owner, chart).await;
    let new = st.iter().find(|s| s.0 != companion.to_string()).unwrap();
    assert_eq!((new.1.as_deref(), new.2.as_str()), (Some("25"), "unknown"));
    let restored: i64 = owner
        .query_one(
            "SELECT count(*) FROM medication_restore WHERE version_id = $1::text::uuid \
               AND medication_id = $2::text::uuid",
            &[&versions[0].to_string(), &new.0],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(restored, 1);
    assert!(
        left(&owner, chart).await.is_empty(),
        "the line left the list"
    );
}

/// Review Focus 2: the restore line is not a duplicate of the line it will stop, so it keeps
/// the default tick; it IS one of another current line of the same drug.
#[tokio::test]
async fn a_restore_line_is_not_a_duplicate_of_its_companion_but_is_of_another() {
    let Some((_guard, owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    allergies_known(&state, chart).await;
    let (_, versions) = charted_with_versions(&state, &owner, chart, &["25mg tab bd"]).await;
    let opened = restore(&state, chart, &[chart], versions[0]).await.unwrap();
    let line = shown(&opened.list, &opened.line_id);
    assert!(
        !line.duplicate && line.default_ticked,
        "{:?}",
        line.duplicate_of
    );
    // Another metoprolol line charted beside it (ticked by hand: it duplicates the companion).
    let other = save(
        &state,
        chart,
        ("metoprolol", "100mg tab mane"),
        pick("metoprolol", METOPROLOL),
        true,
    )
    .await;
    let (outcome, charted, _, _, _) = done(
        sign(&state, chart, &[other.to_string()], credential("bea"))
            .await
            .unwrap(),
    );
    assert_eq!(charted, 1, "{outcome}");
    let view = read(&state, chart).await;
    let line = shown(&view, &opened.line_id);
    assert!(line.duplicate && !line.default_ticked);
    assert!(line.duplicate_of.is_some());
}

/// Review Focus 3: two restore lines of one companion duplicate each other, so neither is
/// default-ticked (one alone is).
#[tokio::test]
async fn two_restores_of_one_line_duplicate_each_other() {
    let Some((_guard, owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    allergies_known(&state, chart).await;
    let (_, versions) =
        charted_with_versions(&state, &owner, chart, &["25mg tab bd", "75mg tab bd"]).await;
    let first = restore(&state, chart, &[chart], versions[0]).await.unwrap();
    assert!(
        shown(&first.list, &first.line_id).default_ticked,
        "one alone"
    );
    let second = restore(&state, chart, &[chart], versions[1]).await.unwrap();
    for id in [&first.line_id, &second.line_id] {
        let line = shown(&second.list, id);
        assert!(
            line.duplicate && !line.default_ticked && !line.ticked,
            "{id}"
        );
    }
    assert!(ticked(&second.list).is_empty());
}

/// Review Focus 4: the version is gone by Sign: the restore line is refused alone with the
/// plan's words, stays in the list, and nothing is stopped.
#[tokio::test]
async fn a_restore_whose_version_is_gone_is_kept() {
    let Some((_guard, owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let (companion, versions) =
        charted_with_versions(&state, &owner, chart, &["25mg tab bd"]).await;
    let opened = restore(&state, chart, &[chart], versions[0]).await.unwrap();
    let line = hand_ticked(shown(&opened.list, &opened.line_id));
    assert_eq!(save_as(&state, chart, line, Some("bea")).await, "saved");
    // The version is no longer readable on this chart (as a shred leaves it).
    owner
        .execute(
            "DELETE FROM medication_set_aside WHERE version_id = $1::text::uuid",
            &[&versions[0].to_string()],
        )
        .await
        .unwrap();
    let gone = read(&state, chart).await;
    assert_eq!(
        shown(&gone, &opened.line_id).restores_words.as_deref(),
        Some(RESTORE_VERSION_GONE),
        "the line says so before Sign"
    );
    let (outcome, charted, of, not_charted, _) = done(
        sign(
            &state,
            chart,
            std::slice::from_ref(&opened.line_id),
            credential("bea"),
        )
        .await
        .unwrap(),
    );
    assert_eq!((charted, of), (0, 1), "{outcome}");
    assert_eq!(
        not_charted,
        [(opened.line_id.clone(), RESTORE_VERSION_GONE.to_string())]
    );
    assert_eq!(
        stop_reason(&owner, companion).await,
        None,
        "nothing stopped"
    );
    assert_eq!(
        left(&owner, chart).await,
        [opened.line_id.parse::<Uuid>().unwrap()]
    );
}

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
