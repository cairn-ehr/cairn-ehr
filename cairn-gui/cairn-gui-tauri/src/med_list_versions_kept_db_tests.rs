//! DB-gated walks for the words a kept version says (ruling R13), beside
//! `med_list_versions_db_tests.rs` (whose helpers these share) to keep each file under 500 lines:
//! a version beside a charted line stopped since says its line is no longer current, never that
//! it is another drug; and a line held by another terminal is never offered as a companion here,
//! since this terminal's Sign could never sign beside it. Run locally on `CAIRN_TEST_PG` (#905).
use super::db_tests::{arg, bea_list, done, left, pick, read, save, save_as, sign};
use super::versions_db_tests::{shown, statements, METOPROLOL};
use super::*;
use crate::commands::cease_impl;
use crate::notes::review::db_tests::credential;

/// A version beside a charted line that was stopped before the Sign stays in the list, saying its
/// line is no longer current.
#[tokio::test]
async fn a_version_beside_a_stopped_line_is_kept_saying_so() {
    let Some((_guard, owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let a = save(
        &state,
        chart,
        ("metoprolol", "50mg bd"),
        pick("metoprolol", METOPROLOL),
        true,
    )
    .await;
    done(
        sign(&state, chart, &[a.to_string()], credential("bea"))
            .await
            .unwrap(),
    );
    let med = statements(&owner, chart).await.remove(0).0;
    let mut v = arg(
        ("metoprolol", "25mg bd"),
        pick("metoprolol", METOPROLOL),
        false,
    );
    let v_id = v.line_id.clone();
    v.set_aside_of = Some(format!("med:{med}"));
    assert_eq!(save_as(&state, chart, v, Some("bea")).await, "saved");
    let patient = chart.to_string();
    cease_impl(
        &state,
        &patient,
        vec![patient.clone()],
        &med,
        "trial over",
        credential("bea"),
    )
    .await
    .unwrap();
    let met = save(&state, chart, ("metformin", "500mg bd"), vec![], true).await;
    let (outcome, charted, _, not_charted, _) = done(
        sign(&state, chart, &[met.to_string()], credential("bea"))
            .await
            .unwrap(),
    );
    assert_eq!(charted, 1, "{outcome}");
    assert!(
        outcome.contains("metoprolol 25mg bd — version kept: its line is no longer current"),
        "{outcome}"
    );
    assert!(not_charted.contains(&(
        v_id.clone(),
        "version kept: its line is no longer current".to_string()
    )));
    assert!(left(&owner, chart).await.contains(&v_id.parse().unwrap()));
}

/// A list held by another terminal offers no companions here: its lines are shown "elsewhere",
/// and this terminal's Sign plans versions over its own lines only.
#[tokio::test]
async fn a_line_on_another_terminal_is_never_offered_as_a_companion() {
    let Some((_guard, owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let parts = serde_json::to_string(&pick("metoprolol", METOPROLOL)).unwrap();
    for _ in 0..2 {
        let a: String = owner
            .query_one(
                "SELECT cairn_med_draft_line_save($1::text::uuid, $2::text::uuid, 'pc-elsewhere', 'bea', \
                        'pre-existing', 'metoprolol 50mg bd', $3::text::jsonb, NULL, false, '[]'::jsonb, \
                        true, 'metoprolol', '50mg bd', false, NULL, NULL, NULL, NULL)",
                &[&Uuid::now_v7().to_string(), &chart.to_string(), &parts],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(a, "saved");
    }
    let view = read(&state, chart).await;
    assert_eq!(view.elsewhere.as_deref(), Some("pc-elsewhere"));
    assert_eq!(view.lines.len(), 2);
    for l in &view.lines {
        assert!(
            shown(&view, &l.line_id).companions.is_empty(),
            "{:?}",
            l.companions
        );
    }
}

/// Each charted line's set-aside versions, as the chart pane draws them under it.
async fn versions_shown(state: &AppState, chart: Uuid) -> Vec<(String, Vec<String>)> {
    let pane = crate::commands::med_list_impl(state, &chart.to_string(), None)
        .await
        .unwrap();
    pane.list
        .rows
        .iter()
        .map(|r| {
            let lines = r.versions.iter().map(|v| v.line.clone()).collect();
            (r.status_label.clone(), lines)
        })
        .collect()
}

/// A version signed beside a charted line is shown collapsed under that line, worded in Rust,
/// and stays readable under it once the line is stopped (design §3.3; Task 5 carry).
#[tokio::test]
async fn a_charted_line_shows_its_versions_even_once_stopped() {
    let Some((_guard, owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let a = save(
        &state,
        chart,
        ("metoprolol", "50mg bd"),
        pick("metoprolol", METOPROLOL),
        true,
    )
    .await;
    done(
        sign(&state, chart, &[a.to_string()], credential("bea"))
            .await
            .unwrap(),
    );
    let med = statements(&owner, chart).await.remove(0).0;
    let mut v = arg(
        ("metoprolol", "25mg bd"),
        pick("metoprolol", METOPROLOL),
        false,
    );
    v.source_detail = Some("GP letter".into());
    v.why = Some("packet 25 mg".into());
    v.set_aside_of = Some(format!("med:{med}"));
    let v_id = v.line_id.clone();
    assert_eq!(save_as(&state, chart, v, Some("bea")).await, "saved");
    let list = read(&state, chart).await;
    assert_eq!(
        list.versions_recorded, 1,
        "beside a current charted line: Sign records it"
    );
    assert_eq!(
        shown(&list, &v_id).companion_words,
        ["charted: metoprolol 50mg bd"],
        "the pick names the charted line by its drug and directions"
    );
    let (outcome, ..) = done(sign(&state, chart, &[], credential("bea")).await.unwrap());
    assert!(outcome.contains("1 version(s) set aside"), "{outcome}");
    let one = |shown: &[(String, Vec<String>)], status: &str| {
        assert_eq!(
            shown.len(),
            1,
            "the version is no line of its own: {shown:?}"
        );
        assert_eq!(shown[0].0, status);
        let [line] = shown[0].1.as_slice() else {
            panic!("one version under the line: {shown:?}")
        };
        assert!(line.starts_with("GP letter: metoprolol"), "{line}");
        assert!(
            line.ends_with(" — set aside, not current — packet 25 mg"),
            "{line}"
        );
    };
    one(&versions_shown(&state, chart).await, "current");
    let patient = chart.to_string();
    cease_impl(
        &state,
        &patient,
        vec![patient.clone()],
        &med,
        "trial over",
        credential("bea"),
    )
    .await
    .unwrap();
    one(&versions_shown(&state, chart).await, "ceased");
}

/// Ruling R16: the count near Sign names only the versions Sign would record — beside a TICKED
/// list line — never one whose companion is unticked (Sign keeps that one in the list).
#[tokio::test]
async fn the_sign_line_counts_only_versions_sign_will_record() {
    let Some((_guard, _owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let mut a = arg(
        ("metoprolol", "50mg bd"),
        pick("metoprolol", METOPROLOL),
        true,
    );
    let a_id = a.line_id.clone();
    assert_eq!(
        save_as(&state, chart, a.clone(), Some("bea")).await,
        "saved"
    );
    let mut v = arg(
        ("metoprolol", "25mg bd"),
        pick("metoprolol", METOPROLOL),
        false,
    );
    v.set_aside_of = Some(format!("line:{a_id}"));
    assert_eq!(save_as(&state, chart, v, Some("bea")).await, "saved");
    let view = read(&state, chart).await;
    assert!(shown(&view, &a_id).ticked);
    assert_eq!(view.versions_recorded, 1, "its line is ticked");
    a.hand_ticked = false;
    a.held = true;
    assert_eq!(save_as(&state, chart, a, Some("bea")).await, "saved");
    let view = read(&state, chart).await;
    assert!(!shown(&view, &a_id).ticked);
    assert_eq!(
        view.versions_recorded, 0,
        "its line is unticked: Sign keeps it"
    );
}
