//! DB-gated walks for the Task 4 review's fix round (rulings R1–R3, findings C1, I1, I2), beside
//! `med_list_db_tests.rs` (whose helpers these share) to keep each file under 500 lines.
//!
//! - R1/C1: a hand tick vouches only for the warnings both DRAWN on the line and found when the
//!   tick was saved; one recorded in between was never seen, and refuses the Sign.
//! - R2/I1: a save or delete is written under the writer the list was READ under.
//! - R3/I2: a reasoned tick whose check cannot run, and a Start tick on a seen mark with no
//!   reason, are per-line refusals with slice 1a's words; the other lines sign.
//! - Review Focus 4 / I3: a lost commit stops the Sign, and each untried line says so (moved here
//!   from `med_list_db_tests.rs` to keep that file under 500 lines).
use super::db_tests::{
    arg, assertions, bea_list, done, here, left, pick, read, save, save_as, sign,
};
use super::*;
use crate::allergies::{allergy_record_impl, EntryInput};
use crate::med_chart::{CHECK_DID_NOT_RUN, NEEDS_A_REASON};
use crate::med_list_sign::{MedListSigned, NOT_TRIED};
use crate::notes::review::db_tests::credential;

/// A drug allergy to `term` coded to `moiety`, signed by Bea.
async fn allergy(state: &AppState, chart: Uuid, term: &str, moiety: u128) {
    let patient = chart.to_string();
    let entry = EntryInput {
        allergen_kind: Some("drug".into()),
        term: term.into(),
        kind: Some("allergy".into()),
        category: Some("anaphylaxis".into()),
        reaction_text: None,
        onset: None,
        source: Some("patient-stated".into()),
        corrects: None,
    };
    let coding = pick(term, moiety).remove(0).coding;
    allergy_record_impl(
        state,
        &patient,
        vec![patient.clone()],
        entry,
        coding,
        credential("bea"),
    )
    .await
    .unwrap();
}

/// The line `id` as the pane drew it.
fn drawn(view: &MedListView, id: &str) -> Vec<WarningRef> {
    view.lines
        .iter()
        .find(|l| l.line_id == id)
        .unwrap()
        .against
        .clone()
}

fn changed(signed: MedListSigned) -> Vec<String> {
    match signed {
        MedListSigned::Changed { lines, .. } => lines,
        other => panic!("expected Changed, got {other:?}"),
    }
}

/// C1: the line is drawn clean, an allergy is recorded, and only then is it ticked (an already-
/// taking line, so no reason is asked). The tick vouches for what was drawn — nothing — so the
/// Sign refuses.
#[tokio::test]
async fn a_warning_recorded_before_the_tick_but_never_drawn_refuses() {
    let Some((_guard, _owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let mut line = arg(
        ("amox", "500mg tds"),
        pick("amoxicillin", 0xA40C_8001),
        false,
    );
    line.charted_as = "pre-existing".into();
    let id = line.line_id.clone();
    let bea = Some("bea");
    assert_eq!(save_as(&state, chart, line.clone(), bea).await, "saved");
    let view = read(&state, chart).await;
    assert!(drawn(&view, &id).is_empty(), "drawn clean");
    allergy(&state, chart, "amoxicillin", 0xA40C_8001).await;
    line.hand_ticked = true;
    line.seen = drawn(&view, &id);
    assert_eq!(save_as(&state, chart, line, bea).await, "saved");
    let signed = sign(&state, chart, std::slice::from_ref(&id), credential("bea"))
        .await
        .unwrap();
    assert_eq!(changed(signed), [id]);
}

/// C1, the Start-line variant: a reason typed for allergy Y does not cover allergy X, recorded
/// after the line was drawn and before the tick was saved.
#[tokio::test]
async fn a_reason_for_one_warning_does_not_cover_one_never_drawn() {
    let Some((_guard, _owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    allergy(&state, chart, "amoxicillin", 0xA40C_8002).await;
    let mut parts = pick("amoxicillin", 0xA40C_8002);
    parts.extend(pick("clavulanic acid", 0xC1A7_8002));
    let mut line = arg(("amox + clav", "1 tab bd"), parts, false);
    let id = line.line_id.clone();
    let bea = Some("bea");
    assert_eq!(save_as(&state, chart, line.clone(), bea).await, "saved");
    let view = read(&state, chart).await;
    assert_eq!(drawn(&view, &id).len(), 1, "Y is drawn");
    allergy(&state, chart, "clavulanic acid", 0xC1A7_8002).await;
    line.hand_ticked = true;
    line.reason = Some("tolerated amoxicillin before".into());
    line.seen = drawn(&view, &id);
    assert_eq!(save_as(&state, chart, line, bea).await, "saved");
    let signed = sign(&state, chart, std::slice::from_ref(&id), credential("bea"))
        .await
        .unwrap();
    assert_eq!(changed(signed), [id]);
}

/// I1: a save that lands after who's here switched to Sue files under Bea, whose list was read.
#[tokio::test]
async fn a_save_after_a_switch_files_under_the_read_writer() {
    let Some((_guard, owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let writer = read(&state, chart).await.writer;
    assert_eq!(writer.as_deref(), Some("bea"));
    here(&state, "sue", "Dr Sue").await;
    let line = arg(("metformin", "500mg bd"), vec![], false);
    let id = line.line_id.clone();
    assert_eq!(
        save_as(&state, chart, line, writer.as_deref()).await,
        "saved"
    );
    let filed: Option<String> = owner
        .query_one(
            "SELECT writer FROM medication_draft_line WHERE line_id = $1::text::uuid",
            &[&id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(filed.as_deref(), Some("bea"));
    assert!(read(&state, chart).await.lines.is_empty(), "never Sue's");
}

/// I1: a delete pressed on Bea's list, landing after the switch, deletes Bea's line.
#[tokio::test]
async fn a_delete_after_a_switch_deletes_the_read_writers_line() {
    let Some((_guard, owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let id = save(&state, chart, ("metformin", "500mg bd"), vec![], false).await;
    here(&state, "sue", "Dr Sue").await;
    let patient = chart.to_string();
    let answer = med_list_delete_impl(&state, &patient, &id.to_string(), Some("bea"))
        .await
        .unwrap();
    assert_eq!(answer.answer, "deleted");
    assert!(left(&owner, chart).await.is_empty());
}

/// I2 (R3): no drug list, so an uncoded line against a coded allergy is "not checked". Its
/// reasoned hand tick still stands; the line is refused with CHECK_DID_NOT_RUN, and the other
/// ticked line signs.
#[tokio::test]
async fn a_reasoned_tick_whose_check_cannot_run_is_one_lines_refusal() {
    let Some((_guard, owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    allergy(&state, chart, "amoxicillin", 0xA40C_8003).await;
    let mut reasoned = arg(("metformin", "500mg bd"), vec![], true);
    reasoned.reason = Some("benefit outweighs".into());
    let met = reasoned.line_id.clone();
    assert_eq!(save_as(&state, chart, reasoned, Some("bea")).await, "saved");
    let mut taking = arg(("paracetamol", "1g qid"), vec![], true);
    taking.charted_as = "pre-existing".into();
    let para = taking.line_id.clone();
    assert_eq!(save_as(&state, chart, taking, Some("bea")).await, "saved");
    let ids = [met.clone(), para];
    let (outcome, charted, of, not_charted, _) =
        done(sign(&state, chart, &ids, credential("bea")).await.unwrap());
    assert_eq!((charted, of), (1, 2), "{outcome}");
    assert_eq!(not_charted, [(met.clone(), CHECK_DID_NOT_RUN.to_string())]);
    assert_eq!(left(&owner, chart).await, [met.parse::<Uuid>().unwrap()]);
}

/// I2 (R3): a Start line hand-ticked against a mark it was drawn with, and no reason, keeps its
/// tick and is refused NEEDS_A_REASON alone; the other line signs.
#[tokio::test]
async fn a_start_tick_on_a_seen_mark_with_no_reason_is_one_lines_refusal() {
    let Some((_guard, _owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    allergy(&state, chart, "amoxicillin", 0xA40C_8004).await;
    let mut amox = arg(
        ("amox", "500mg tds"),
        pick("amoxicillin", 0xA40C_8004),
        false,
    );
    let id = amox.line_id.clone();
    assert_eq!(
        save_as(&state, chart, amox.clone(), Some("bea")).await,
        "saved"
    );
    let view = read(&state, chart).await;
    amox.hand_ticked = true;
    amox.seen = drawn(&view, &id);
    assert_eq!(amox.seen.len(), 1);
    assert_eq!(save_as(&state, chart, amox, Some("bea")).await, "saved");
    let met = save(&state, chart, ("metformin", "500mg bd"), vec![], true).await;
    let ids = [id.clone(), met.to_string()];
    let (outcome, charted, of, not_charted, _) =
        done(sign(&state, chart, &ids, credential("bea")).await.unwrap());
    assert_eq!((charted, of), (1, 2), "{outcome}");
    assert_eq!(not_charted, [(id.clone(), NEEDS_A_REASON.to_string())]);
    let view = read(&state, chart).await;
    assert!(
        view.lines[0].hand_ticked,
        "the tick stands; the line wants its reason"
    );
}

/// Review Focus 4: the second line's COMMIT is lost (a test-only deferred trigger ends the
/// backend at commit, the `identity_commit_lost.rs` technique, inert for any other line). The
/// first line is charted and gone; the second is reported NOT known; the third is never tried and
/// stays in the list.
#[tokio::test]
async fn a_lost_commit_stops_and_keeps_the_rest() {
    let Some((_guard, owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let lines = [
        save(&state, chart, ("metformin", "500mg bd"), vec![], true).await,
        save(&state, chart, ("paracetamol", "1g qid"), vec![], true).await,
        save(&state, chart, ("aspirin", "100mg daily"), vec![], true).await,
    ];
    let drop =
        "DROP TRIGGER IF EXISTS cairn_test_kill_at_commit ON medication_draft_line_deleted; \
                DROP FUNCTION IF EXISTS cairn_test_kill_at_commit() CASCADE;";
    owner.batch_execute(drop).await.unwrap();
    owner
        .batch_execute(&format!(
            "CREATE FUNCTION cairn_test_kill_at_commit() RETURNS trigger LANGUAGE plpgsql AS $$
             BEGIN
                 IF NEW.line_id = '{}'::uuid THEN
                     PERFORM pg_terminate_backend(pg_backend_pid());
                 END IF;
                 RETURN NULL;
             END $$;
             CREATE CONSTRAINT TRIGGER cairn_test_kill_at_commit
                 AFTER INSERT ON medication_draft_line_deleted
                 DEFERRABLE INITIALLY DEFERRED
                 FOR EACH ROW EXECUTE FUNCTION cairn_test_kill_at_commit();",
            lines[1]
        ))
        .await
        .unwrap();
    let ids: Vec<String> = lines.iter().map(Uuid::to_string).collect();
    let signed = sign(&state, chart, &ids, credential("bea")).await;
    owner.batch_execute(drop).await.unwrap();
    let (outcome, charted, of, not_charted, unknown) = done(signed.unwrap());
    assert_eq!((charted, of), (1, 3), "{outcome}");
    assert_eq!(unknown, [lines[1].to_string()]);
    assert_eq!(
        not_charted,
        [(lines[2].to_string(), NOT_TRIED.to_string())],
        "the third was not tried, and says so"
    );
    assert!(outcome.contains("is NOT known"), "{outcome}");
    assert_eq!(assertions(&owner, chart).await, 1);
    assert_eq!(left(&owner, chart).await, [lines[1], lines[2]]);
}
