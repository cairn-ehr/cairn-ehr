//! #864 — a medication list is read from ONE snapshot.
//!
//! The list is assembled from many statements; under READ COMMITTED a stop committed between
//! two of them gave a line that read stopped with no reason. `medication::snapshot::in_snapshot`
//! runs the read in one `REPEATABLE READ READ ONLY` transaction, and every production entry
//! point reads through it.
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.
use cairn_medication_view::MedicationStatus;
use cairn_node::db;
use cairn_node::medication::read::list_patient_medications;
use cairn_node::medication::snapshot::in_snapshot;
use cairn_node::medication::{cease_medication, CeaseMedicationInput};

mod common;
#[path = "common/medication_kit.rs"]
mod medication_kit;
use medication_kit::*;

#[tokio::test]
async fn a_stop_committed_after_the_read_began_is_not_seen() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let mut colleague = db::connect_and_load_schema(&base).await.unwrap();

    let list = in_snapshot(&mut c, async |tx| {
        // The snapshot is taken at the transaction's first statement ...
        tx.query_one("SELECT 1", &[]).await?;
        // ... and a colleague's stop commits after it, before the list's own statements.
        let input = CeaseMedicationInput {
            stopped: None,
            stopped_precision: None,
            reason: Some("angioedema"),
        };
        cease_medication(
            &mut colleague,
            &sk,
            &kid,
            "test-node",
            p,
            med,
            &input,
            None,
            None,
        )
        .await?;
        list_patient_medications(tx, p).await
    })
    .await
    .unwrap();
    assert_eq!(
        list.rows[0].status,
        MedicationStatus::Active,
        "every statement of the read sees the state it started from"
    );

    // The next read sees the stop whole: stopped, with its reason.
    let list = in_snapshot(&mut c, async |tx| list_patient_medications(tx, p).await)
        .await
        .unwrap();
    assert_eq!(list.rows[0].status, MedicationStatus::Ceased);
    assert_eq!(list.rows[0].cessation_reason.as_deref(), Some("angioedema"));
}

/// Every production entry point to a list read goes through `in_snapshot`: the CLI's
/// `medication-list` and the cease notes, the sign-off's two reads, the window's chart read.
#[test]
fn production_list_reads_run_in_one_snapshot() {
    let sources = [
        ("main.rs", include_str!("../src/main.rs")),
        ("signoff.rs", include_str!("../src/medication/signoff.rs")),
        (
            "cairn-gui-tauri commands.rs",
            include_str!("../../../cairn-gui/cairn-gui-tauri/src/commands.rs"),
        ),
    ];
    let readers = [
        "list_patient_medications(",
        "medication::already_stopped_note(",
        "medication::still_current_note(",
    ];
    let mut calls = 0;
    for (file, src) in sources {
        for reader in readers {
            for (at, _) in src.match_indices(reader) {
                if src[..at].ends_with("fn ") {
                    continue;
                }
                let before = &src[src.floor_char_boundary(at.saturating_sub(160))..at];
                assert!(
                    before.contains("in_snapshot("),
                    "{file}: a call to `{reader}` is not inside `in_snapshot` (#864)"
                );
                calls += 1;
            }
        }
    }
    // CLI list + two cease notes, two sign-off reads, the window's read: a call that moves
    // out of these files must not let the guard pass vacuously.
    assert_eq!(calls, 6, "expected exactly six production list reads");
}
