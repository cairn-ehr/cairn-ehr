//! DB-gated walks for batch charting slice 2a's Sign of a set-aside version
//! (`med_list_versions.rs`): signed with its companion in the list's one Sign, alone beside a
//! charted line, or kept when its companion is not charted. On `med_list_db_tests.rs`'s fixture
//! (Dr Bea named, a John Doe chart), whose helpers these share; the source and why walks are
//! `med_list_source_db_tests.rs`. Run locally on `CAIRN_TEST_PG` (#905: CI never runs these).
use super::db_tests::{
    amoxicillin_allergy, arg, bea_list, done, left, pick, read, save, save_as, sign,
};
use super::*;
use crate::med_chart::NEEDS_A_REASON;
use crate::med_list_sign::MedListSigned;
use crate::med_list_versions::VERSION_KEPT;
use crate::notes::review::db_tests::credential;
use tokio_postgres::Client;

/// A metoprolol-family moiety, and another drug (atenolol) for an edited companion.
pub(super) const METOPROLOL: u128 = 0x3E70_0001;
const ATENOLOL: u128 = 0xA7E0_0002;

/// A finished Already-taking line set aside beside `mark`, as a source reported it.
fn version(typed: (&str, &str), moiety: u128, mark: String) -> LineArg {
    let mut v = arg(typed, pick("metoprolol", moiety), false);
    v.charted_as = "pre-existing".into();
    v.info_source = Some("external-record".into());
    v.source_detail = Some("GP letter".into());
    v.why = Some("packet 25 mg, daughter confirms".into());
    v.set_aside_of = Some(mark);
    v
}

fn line_mark(id: &str) -> String {
    format!("line:{id}")
}

/// The line `id` as the pane drew it.
pub(super) fn shown<'a>(view: &'a MedListView, id: &str) -> &'a ListLineView {
    view.lines.iter().find(|l| l.line_id == id).unwrap()
}

/// The chart's statements as `(medication_id, dose_amount, info_source)` (read as the owner).
pub(super) async fn statements(
    owner: &Client,
    chart: Uuid,
) -> Vec<(String, Option<String>, String)> {
    owner
        .query(
            "SELECT medication_id::text, dose_amount, info_source FROM medication_statement \
              WHERE patient_id = $1::text::uuid ORDER BY dose_amount",
            &[&chart.to_string()],
        )
        .await
        .unwrap()
        .iter()
        .map(|r| (r.get(0), r.get(1), r.get(2)))
        .collect()
}

/// The chart's set-aside versions as `(alternative_to, info_source, source_detail, why)`.
async fn set_asides(
    owner: &Client,
    chart: Uuid,
) -> Vec<(String, String, Option<String>, Option<String>)> {
    owner
        .query(
            "SELECT alternative_to::text, info_source, source_detail, why FROM medication_set_aside \
              WHERE patient_id = $1::text::uuid",
            &[&chart.to_string()],
        )
        .await
        .unwrap()
        .iter()
        .map(|r| (r.get(0), r.get(1), r.get(2), r.get(3)))
        .collect()
}

/// A companion ticked by hand and a version set aside beside it: ONE Sign charts the line and
/// signs the version under its new thread, and both leave the list.
#[tokio::test]
async fn one_sign_charts_the_line_and_its_version_together() {
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
    let v = version(
        ("metoprolol", "25mg bd"),
        METOPROLOL,
        line_mark(&a.to_string()),
    );
    let v_id = v.line_id.clone();
    let unmarked = LineArg {
        set_aside_of: None,
        ..v.clone()
    };
    assert_eq!(save_as(&state, chart, unmarked, Some("bea")).await, "saved");
    let before = read(&state, chart).await;
    assert_eq!(
        shown(&before, &a.to_string()).companions,
        [line_mark(&v_id)],
        "each is the other's candidate before the press"
    );
    assert_eq!(save_as(&state, chart, v, Some("bea")).await, "saved");
    let view = read(&state, chart).await;
    let shown_v = shown(&view, &v_id);
    assert_eq!(shown_v.companions, [line_mark(&a.to_string())]);
    assert!(!shown_v.ticked, "a set-aside line is never ticked");
    assert!(
        shown(&view, &a.to_string()).companions.is_empty(),
        "a set-aside line is no one's companion"
    );
    let (outcome, charted, of, not_charted, _) = done(
        sign(&state, chart, &[a.to_string()], credential("bea"))
            .await
            .unwrap(),
    );
    assert_eq!((charted, of), (1, 1), "{outcome}");
    assert!(not_charted.is_empty());
    assert_eq!(outcome, "Charted 1 of 1; 1 version(s) set aside");
    let st = statements(&owner, chart).await;
    assert_eq!(st.len(), 1, "the version is never a statement");
    assert_eq!(
        set_asides(&owner, chart).await,
        [(
            st[0].0.clone(),
            "external-record".to_string(),
            Some("GP letter".to_string()),
            Some("packet 25 mg, daughter confirms".to_string()),
        )]
    );
    assert!(left(&owner, chart).await.is_empty(), "both left the list");
}

/// Review Focus 1: the companion is refused for its own line (a Start tick on a seen mark with no
/// reason): it is not charted, and its version is not signed but stays in the list, saying so.
#[tokio::test]
async fn a_refused_companion_keeps_its_versions_unsigned() {
    let Some((_guard, owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    amoxicillin_allergy(&state, chart, METOPROLOL).await;
    let mut a = arg(
        ("metoprolol", "50mg bd"),
        pick("metoprolol", METOPROLOL),
        false,
    );
    let a_id = a.line_id.clone();
    assert_eq!(
        save_as(&state, chart, a.clone(), Some("bea")).await,
        "saved"
    );
    a.hand_ticked = true;
    a.seen = shown(&read(&state, chart).await, &a_id).against.clone();
    assert_eq!(a.seen.len(), 1, "the mark was drawn");
    assert_eq!(save_as(&state, chart, a, Some("bea")).await, "saved");
    let v = version(("metoprolol", "25mg bd"), METOPROLOL, line_mark(&a_id));
    let v_id: Uuid = v.line_id.parse().unwrap();
    assert_eq!(save_as(&state, chart, v, Some("bea")).await, "saved");
    let met = save(&state, chart, ("metformin", "500mg bd"), vec![], true).await;
    let (outcome, charted, of, not_charted, _) = done(
        sign(
            &state,
            chart,
            &[a_id.clone(), met.to_string()],
            credential("bea"),
        )
        .await
        .unwrap(),
    );
    assert_eq!((charted, of), (1, 2), "{outcome}");
    assert!(not_charted.contains(&(a_id.clone(), NEEDS_A_REASON.to_string())));
    assert!(
        outcome.contains(&format!(
            "metoprolol 25mg bd — version kept: {VERSION_KEPT}"
        )),
        "{outcome}"
    );
    assert!(
        set_asides(&owner, chart).await.is_empty(),
        "no version points at nothing"
    );
    assert!(left(&owner, chart).await.contains(&v_id));
}

/// Review Focus 2: the companion is edited to another drug after "Set aside" (metoprolol →
/// atenolol) and ticked again: it is charted, and the version is refused for its own line.
#[tokio::test]
async fn an_edited_companion_refuses_its_version_per_line() {
    let Some((_guard, owner, state, chart)) = bea_list().await else {
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
    let v = version(("metoprolol", "25mg bd"), METOPROLOL, line_mark(&a_id));
    let v_id: Uuid = v.line_id.parse().unwrap();
    assert_eq!(save_as(&state, chart, v, Some("bea")).await, "saved");
    a.parts = pick("atenolol", ATENOLOL);
    a.typed_drug = "atenolol".into();
    a.line_text = "atenolol 50mg bd".into();
    // The edit clears the tick in the door; the clinician ticks the new words again.
    assert_eq!(
        save_as(&state, chart, a.clone(), Some("bea")).await,
        "saved"
    );
    assert_eq!(save_as(&state, chart, a, Some("bea")).await, "saved");
    let (outcome, charted, of, _, _) = done(
        sign(&state, chart, &[a_id], credential("bea"))
            .await
            .unwrap(),
    );
    assert_eq!((charted, of), (1, 1), "{outcome}");
    assert!(
        outcome.contains(
            "metoprolol 25mg bd — version kept: the line it was set aside beside is no longer the same drug"
        ),
        "{outcome}"
    );
    assert!(set_asides(&owner, chart).await.is_empty());
    assert_eq!(left(&owner, chart).await, [v_id]);
}

/// Ruling R9: the version's companion is the one its STORED mark names. Ticking the other line of
/// the same drug never takes the version with it (and a tick on the version refuses the Sign).
#[tokio::test]
async fn a_webview_naming_another_companion_cannot_move_a_version() {
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
    let b = save(
        &state,
        chart,
        ("metoprolol", "95mg daily"),
        pick("metoprolol", METOPROLOL),
        true,
    )
    .await;
    let v = version(
        ("metoprolol", "25mg bd"),
        METOPROLOL,
        line_mark(&a.to_string()),
    );
    let v_id = v.line_id.clone();
    assert_eq!(save_as(&state, chart, v, Some("bea")).await, "saved");
    let signed = sign(
        &state,
        chart,
        &[b.to_string(), v_id.clone()],
        credential("bea"),
    )
    .await
    .unwrap();
    assert!(
        matches!(signed, MedListSigned::Changed { .. }),
        "{signed:?}"
    );
    let (outcome, charted, _, _, _) = done(
        sign(&state, chart, &[b.to_string()], credential("bea"))
            .await
            .unwrap(),
    );
    assert_eq!(charted, 1, "{outcome}");
    assert!(outcome.contains("version kept"), "{outcome}");
    assert!(
        set_asides(&owner, chart).await.is_empty(),
        "never signed under B"
    );
    let (outcome, charted, _, _, _) = done(
        sign(&state, chart, &[a.to_string()], credential("bea"))
            .await
            .unwrap(),
    );
    assert_eq!(charted, 1, "{outcome}");
    let a_med = statements(&owner, chart)
        .await
        .into_iter()
        .find(|(_, dose, _)| dose.as_deref() == Some("50"))
        .unwrap()
        .0;
    let signed_beside: Vec<String> = set_asides(&owner, chart)
        .await
        .into_iter()
        .map(|s| s.0)
        .collect();
    assert_eq!(signed_beside, [a_med], "under the line its mark names");
}

/// A version beside a line ALREADY charted signs alone, in its own transaction, at the Sign.
#[tokio::test]
async fn a_version_beside_a_charted_line_signs_alone() {
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
    assert_eq!(
        save_as(&state, chart, v.clone(), Some("bea")).await,
        "saved"
    );
    let offered = shown(&read(&state, chart).await, &v_id).companions.clone();
    assert_eq!(
        offered,
        [format!("med:{med}")],
        "the charted line is offered"
    );
    v.set_aside_of = Some(offered[0].clone());
    assert_eq!(save_as(&state, chart, v, Some("bea")).await, "saved");
    let met = save(&state, chart, ("metformin", "500mg bd"), vec![], true).await;
    let (outcome, charted, of, _, _) = done(
        sign(&state, chart, &[met.to_string()], credential("bea"))
            .await
            .unwrap(),
    );
    assert_eq!((charted, of), (1, 1), "{outcome}");
    assert_eq!(outcome, "Charted 1 of 1; 1 version(s) set aside");
    let beside: Vec<String> = set_asides(&owner, chart)
        .await
        .into_iter()
        .map(|s| s.0)
        .collect();
    assert_eq!(beside, [med]);
    assert!(left(&owner, chart).await.is_empty());
}

/// The carry from Task 5: a companion's commit is lost, so its versions may have landed too;
/// they are reported NOT known with it, and the loop stops.
#[tokio::test]
async fn a_lost_commit_names_its_versions_not_known() {
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
    let v = version(
        ("metoprolol", "25mg bd"),
        METOPROLOL,
        line_mark(&a.to_string()),
    );
    let v_id = v.line_id.clone();
    assert_eq!(save_as(&state, chart, v, Some("bea")).await, "saved");
    let drop =
        "DROP TRIGGER IF EXISTS cairn_test_kill_at_commit ON medication_draft_line_deleted; \
                DROP FUNCTION IF EXISTS cairn_test_kill_at_commit() CASCADE;";
    owner.batch_execute(drop).await.unwrap();
    owner
        .batch_execute(&format!(
            "CREATE FUNCTION cairn_test_kill_at_commit() RETURNS trigger LANGUAGE plpgsql AS $$
             BEGIN
                 IF NEW.line_id = '{a}'::uuid THEN
                     PERFORM pg_terminate_backend(pg_backend_pid());
                 END IF;
                 RETURN NULL;
             END $$;
             CREATE CONSTRAINT TRIGGER cairn_test_kill_at_commit
                 AFTER INSERT ON medication_draft_line_deleted
                 DEFERRABLE INITIALLY DEFERRED
                 FOR EACH ROW EXECUTE FUNCTION cairn_test_kill_at_commit();"
        ))
        .await
        .unwrap();
    let signed = sign(&state, chart, &[a.to_string()], credential("bea")).await;
    owner.batch_execute(drop).await.unwrap();
    let (outcome, charted, _, _, unknown) = done(signed.unwrap());
    assert_eq!(charted, 0, "{outcome}");
    assert_eq!(unknown, [a.to_string(), v_id]);
    assert!(
        outcome.contains("metoprolol 25mg bd — Whether this medication was charted is NOT known"),
        "{outcome}"
    );
    assert!(!outcome.contains("version kept"), "{outcome}");
}
