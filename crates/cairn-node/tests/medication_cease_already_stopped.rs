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
use cairn_node::medication::{already_stopped_note, cease_medication, CeaseMedicationInput};
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
}

/// The CLI arm asks BEFORE it ceases (after, its own cease would read as "already stopped")
/// and prints the note after the success line.
#[test]
fn the_cease_verb_reads_the_note_first_and_prints_it_last() {
    let main = include_str!("../src/main.rs");
    let arm = &main[main.find("Cmd::MedicationCease {").unwrap()..];
    let arm = &arm[..arm.find("Cmd::MedicationWithhold {").unwrap()];
    let asked = arm
        .find("medication::already_stopped_note(&db, patient, medication_id)")
        .expect("the arm reads the note");
    let ceased = arm.find("medication::cease_medication(").unwrap();
    let said = arm.find("println!(\"ceased medication thread").unwrap();
    let noted = arm
        .find("println!(\"{note}\")")
        .expect("the arm prints the note");
    assert!(asked < ceased && ceased < said && said < noted, "{arm}");
    // A display-only read must never block the stop: its failure becomes a sentence, not `?`.
    let flat: String = arm.split_whitespace().collect();
    assert!(
        !flat.contains("already_stopped_note(&db,patient,medication_id).await?"),
        "{arm}"
    );
    assert!(
        flat.contains("Err(e)=>Some(cairn_node::medication::not_known_whether_stopped(&e)),"),
        "{arm}"
    );
}
