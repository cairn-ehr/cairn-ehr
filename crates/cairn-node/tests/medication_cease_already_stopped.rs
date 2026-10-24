//! #854 (ADR-0082 decision 4) — the CLI's `medication-cease` says when the line was ALREADY
//! stopped before its cease reached the record.
//!
//! A cease of a ceased line is admitted at both doors (its reason may be the only record of,
//! say, an adverse reaction). The window already says so (`already_stopped_note` in
//! `cairn-gui-tauri`); the CLI printed only "ceased medication thread …", so an operator
//! stopping a drug a colleague stopped a minute earlier read their stop as the one that
//! stopped it. `medication::already_stopped_note` reads the line before the cease, by the same
//! rule the window uses (`MedicationRow::already_stopped_note`).
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.
use cairn_node::db;
use cairn_node::medication::{
    already_stopped_note, cease_medication, not_known_whether_current, not_known_whether_stopped,
    reconcile_medications, still_current_note, CeaseMedicationInput, ReconcileInput,
};
use uuid::Uuid;

mod common;
#[path = "common/medication_kit.rs"]
mod medication_kit;
use medication_kit::*;

#[tokio::test]
async fn a_line_already_stopped_earns_the_note_with_the_earlier_reason() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    assert_eq!(
        already_stopped_note(&c, p, med).await.unwrap(),
        None,
        "an active line says nothing extra"
    );

    let input = CeaseMedicationInput {
        stopped: None,
        stopped_precision: None,
        reason: Some("angioedema"),
    };
    cease_medication(&mut c, &sk, &kid, "test-node", p, med, &input, None, None)
        .await
        .unwrap();
    let note = already_stopped_note(&c, p, med)
        .await
        .unwrap()
        .expect("a ceased line earns the note");
    assert!(note.contains("already been stopped"), "{note}");
    assert!(note.contains("angioedema"), "{note}");
}

#[tokio::test]
async fn a_thread_on_no_line_here_says_it_cannot_tell() {
    // Offline-first: `medication-cease` does not need the assert to be local, so a thread this
    // node has never seen is on no line. Silence would read as "not already stopped"
    // (principle 4), so the note says it cannot tell.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, _med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let note = already_stopped_note(&c, p, Uuid::now_v7())
        .await
        .unwrap()
        .expect("an unseen thread earns a sentence");
    assert!(note.contains("not known"), "{note}");
    assert!(note.contains("this patient's list"), "{note}");
}

#[tokio::test]
async fn a_thread_already_stopped_on_a_line_still_current_says_both() {
    // PR #856 review: `medication-cease` stops ONE thread, but a reconciled line reads by its
    // latest-effective member. Thread X stopped (2020) beside thread Y (from 2024) leaves the
    // line current, so the line-level rule alone said nothing to an operator re-stopping X.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, x) = chart_with_one_med(&mut c, &sk, &kid).await;
    let y = add_med(&mut c, &sk, &kid, p).await;
    let input = ReconcileInput {
        provenance: "clinician-judgment",
        reason: None,
    };
    reconcile_medications(&mut c, &sk, &kid, "test-node", p, x, y, &input, None, None)
        .await
        .unwrap();
    let stop = CeaseMedicationInput {
        stopped: Some("2020"),
        stopped_precision: Some("year"),
        reason: Some("angioedema"),
    };
    cease_medication(&mut c, &sk, &kid, "test-node", p, x, &stop, None, None)
        .await
        .unwrap();

    let note = already_stopped_note(&c, p, x)
        .await
        .unwrap()
        .expect("a thread already stopped earns the note though its line reads current");
    assert!(note.contains("already been stopped"), "{note}");
    assert!(note.contains("angioedema"), "{note}");
    // That the line still reads current is said AFTER the stop, naming the sibling (#859,
    // `still_current_note`), so this note does not repeat it.
    assert!(!note.contains("still reads current"), "{note}");
    // The line reads current, so the list shows no stop reason: the note must not promise it
    // keeps them (PR #860 review; #861).
    assert!(
        note.contains("the list shows stop reasons only on a line that reads stopped"),
        "{note}"
    );
    assert!(!note.contains("keeps every stop's reason"), "{note}");
    assert_eq!(
        already_stopped_note(&c, p, y).await.unwrap(),
        None,
        "the active sibling says nothing extra"
    );
}

#[tokio::test]
async fn a_failed_read_is_an_error_and_its_note_says_not_known() {
    // The read must surface its failure (never `Ok(None)`, which would read as "not already
    // stopped"), and the CLI's sentence for it names the cause. Seam: an aborted transaction.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let tx = c.transaction().await.unwrap();
    tx.batch_execute("SELECT 1/0").await.unwrap_err();
    let err = already_stopped_note(&tx, p, med)
        .await
        .expect_err("a read in an aborted transaction fails");
    let note = not_known_whether_stopped(&err);
    assert!(note.contains("already stopped is not known"), "{note}");
    assert!(note.contains("aborted"), "the cause is named: {note}");
    // The same holds for the re-read after the stop (#859).
    let err = still_current_note(&tx, p, med)
        .await
        .expect_err("a read in an aborted transaction fails");
    let note = not_known_whether_current(&err);
    assert!(note.contains("still reads current is not known"), "{note}");
    assert!(note.contains("aborted"), "the cause is named: {note}");
}

#[tokio::test]
async fn a_thread_whose_latest_stop_gave_no_reason_still_names_the_earlier_one() {
    // PR #860 review: the thread's newest stop gave no reason, an earlier one did. "no reason
    // was recorded" would hide the adverse reaction #857 exists for.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, x) = chart_with_one_med(&mut c, &sk, &kid).await;
    let y = add_med(&mut c, &sk, &kid, p).await;
    let input = ReconcileInput {
        provenance: "clinician-judgment",
        reason: None,
    };
    reconcile_medications(&mut c, &sk, &kid, "test-node", p, x, y, &input, None, None)
        .await
        .unwrap();
    for reason in [Some("angioedema"), None] {
        let stop = CeaseMedicationInput {
            stopped: Some("2020"),
            stopped_precision: Some("year"),
            reason,
        };
        cease_medication(&mut c, &sk, &kid, "test-node", p, x, &stop, None, None)
            .await
            .unwrap();
    }
    let note = already_stopped_note(&c, p, x).await.unwrap().unwrap();
    assert!(!note.contains("no reason was recorded"), "{note}");
    assert!(
        note.contains("one stop gave no reason; another stop's reason: angioedema"),
        "{note}"
    );
}

/// The CLI arm asks BEFORE it ceases (after, its own cease would read as "already stopped"),
/// asks whether the line still reads current AFTER it (#859), and prints both notes after the
/// success line, on stderr beside the CLI's other warnings.
#[test]
fn the_cease_verb_reads_the_note_first_and_prints_it_last() {
    let main = include_str!("../src/main.rs");
    let arm = &main[main.find("Cmd::MedicationCease {").unwrap()..];
    let arm = &arm[..arm.find("Cmd::MedicationWithhold {").unwrap()];
    let asked = arm
        .find("medication::already_stopped_note(tx, patient, medication_id)")
        .expect("the arm reads the note");
    let ceased = arm.find("medication::cease_medication(").unwrap();
    let said = arm.find("println!(\"ceased medication thread").unwrap();
    let still = arm
        .find("medication::still_current_note(tx, patient, medication_id)")
        .expect("the arm re-reads the line after the stop");
    let noted = arm
        .find("eprintln!(\"warning: {note}\")")
        .expect("the arm prints the notes");
    assert!(
        asked < ceased && ceased < said && said < still && still < noted,
        "{arm}"
    );
    // A display-only read must never block the stop: its failure becomes a sentence, not `?`.
    let flat: String = arm.split_whitespace().collect();
    // (Each read runs in its own snapshot, #864: the snapshot's result is MATCHED, never `?`.)
    assert!(
        flat.contains(
            "already_stopped_note(tx,patient,medication_id).await}).await{Ok(note)=>note,"
        ),
        "{arm}"
    );
    assert!(
        flat.contains("Err(e)=>Some(cairn_node::medication::not_known_whether_stopped(&e)),"),
        "{arm}"
    );
    assert!(
        flat.contains("Err(e)=>Some(cairn_node::medication::not_known_whether_current(&e)),"),
        "{arm}"
    );
    assert!(
        flat.contains("fornoteinnote.into_iter().chain(current)"),
        "both notes are printed: {arm}"
    );
}
