//! #863 — shredding an overlay WINNER must hand its row back to the next unshredded event.
//!
//! A one-winner projection (`medication_cessation`, `medication_coding`, … — one row per key,
//! newest wins) holds only the winner's row. db/037's scrub deletes the rows the shredded event
//! produced. When that event was the winner, the key used to be left with NO row: the runner-up
//! (an earlier stop, an earlier coding) was never re-applied, so a stopped drug read as current
//! until the next full heal. `cairn_execute_shred` now re-folds the remaining events that feed
//! the same rows (`cairn_refold_after_shred`).
//!
//! The older-stop direction (shred a runner-up, the winner stands) is pinned in
//! `medication_stop_reasons.rs::a_shredded_stop_takes_its_reason_with_it`.
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.
use cairn_event::medication::SubstanceCoding;
use cairn_event::SigningKey;
use cairn_medication_view::MedicationStatus;
use cairn_node::db;
use cairn_node::medication::read::list_patient_medications;
use cairn_node::medication::{
    cease_medication, code_medication, CeaseMedicationInput, CodeMedicationInput,
};
use cairn_node::shred::shred_event;
use tokio_postgres::Client;
use uuid::Uuid;

mod common;
#[path = "common/medication_kit.rs"]
mod medication_kit;
use medication_kit::*;

/// Shred `target` through the node's own verb (the strict door's erasure arm).
async fn shred(c: &mut Client, sk: &SigningKey, kid: &str, target: Uuid) {
    shred_event(c, sk, kid, "test-node", target, "retention ceiling", None)
        .await
        .expect("shred");
}

/// Stop `med` with `reason`; returns the cessation's event id.
async fn stop(
    c: &mut Client,
    sk: &SigningKey,
    kid: &str,
    p: Uuid,
    med: Uuid,
    reason: &str,
) -> Uuid {
    let input = CeaseMedicationInput {
        stopped: None,
        stopped_precision: None,
        reason: Some(reason),
    };
    cease_medication(c, sk, kid, "test-node", p, med, &input, None, None)
        .await
        .unwrap()
}

/// Code thread `med` as `display` (a coding OVERLAY, db/042); returns the event id.
async fn code_as(
    c: &mut Client,
    sk: &SigningKey,
    kid: &str,
    p: Uuid,
    med: Uuid,
    code: &str,
    display: &str,
) -> Uuid {
    let input = CodeMedicationInput {
        coding: SubstanceCoding {
            system: "drugref-moiety",
            code,
            display,
        },
    };
    code_medication(c, sk, kid, "test-node", p, med, &input, None, None)
        .await
        .unwrap()
}

#[tokio::test]
async fn shredding_the_newest_stop_leaves_the_earlier_stop_in_force() {
    // The issue's case: stop "angioedema", stop again "patient request", shred the second.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    stop(&mut c, &sk, &kid, p, med, "angioedema").await;
    let second = stop(&mut c, &sk, &kid, p, med, "patient request").await;

    shred(&mut c, &sk, &kid, second).await;

    let list = list_patient_medications(&c, p).await.unwrap();
    assert_eq!(list.rows.len(), 1, "one line expected");
    let row = &list.rows[0];
    assert_eq!(
        row.status,
        MedicationStatus::Ceased,
        "the earlier stop still stands: the drug must not read current"
    );
    assert_eq!(row.cessation_reason.as_deref(), Some("angioedema"));
    assert!(row.other_cessation_reasons.is_empty());
    // And the shredded stop's text is gone from every projection (ADR-0005 rung 3).
    let left: i64 = c
        .query_one(
            "SELECT count(*) FROM medication_cessation WHERE reason = 'patient request'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(left, 0, "the re-fold must not bring the shredded stop back");
}

#[tokio::test]
async fn shredding_the_winning_coding_restores_the_earlier_coding() {
    // A second one-winner key, so the fix is pinned as the registry-wide re-fold it is, not a
    // cessation special case.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let moiety = |n: u8| Uuid::from_bytes(std::array::from_fn(|i| n ^ i as u8)).to_string();
    let (first, second) = (moiety(1), moiety(2));
    code_as(&mut c, &sk, &kid, p, med, &first, "metoprolol tartrate").await;
    let newer = code_as(&mut c, &sk, &kid, p, med, &second, "metoprolol succinate").await;

    shred(&mut c, &sk, &kid, newer).await;

    let row = c
        .query_opt(
            "SELECT coding_code, coding_display FROM medication_coding WHERE medication_id = $1::text::uuid",
            &[&med.to_string()],
        )
        .await
        .unwrap()
        .expect("the earlier coding must be the thread's coding again, not no coding at all");
    let (code, display): (Option<String>, Option<String>) = (row.get(0), row.get(1));
    assert_eq!(code.as_deref(), Some(first.as_str()));
    assert_eq!(display.as_deref(), Some("metoprolol tartrate"));
}

#[tokio::test]
async fn shredding_a_lone_stop_leaves_the_drug_current() {
    // The re-fold replays the OTHER events only: with no earlier stop there is nothing to
    // re-fold, and the shredded stop itself must never come back.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let only = stop(&mut c, &sk, &kid, p, med, "angioedema").await;

    shred(&mut c, &sk, &kid, only).await;

    let list = list_patient_medications(&c, p).await.unwrap();
    assert_eq!(list.rows[0].status, MedicationStatus::Active);
    let left: i64 = c
        .query_one(
            "SELECT count(*) FROM medication_cessation WHERE medication_id = $1::text::uuid",
            &[&med.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(left, 0);
}

#[tokio::test]
async fn the_refold_puts_the_callers_apply_posture_back() {
    // The re-fold replays lenient. Left 'on', it would make every later write in the shred's
    // transaction skip the strict door's raise-not-clamp checks — a floor bypass.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let tx = c.transaction().await.unwrap();
    let posture: Option<String> = tx
        .query_one(
            "SELECT cairn_refold_after_shred(ARRAY[gen_random_uuid()::text]), \
                    current_setting('cairn.remote_apply', true)",
            &[],
        )
        .await
        .unwrap()
        .get(1);
    assert_ne!(posture.as_deref(), Some("on"));
}

#[tokio::test]
async fn a_shred_never_waits_on_a_siblings_custody_lock() {
    // Two shreds of sibling events each re-fold the other; if the re-fold WAITED on a sibling's
    // custody lock they could deadlock. It tries the lock and skips a sibling another
    // transaction is busy with.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let first = stop(&mut c, &sk, &kid, p, med, "angioedema").await;
    let second = stop(&mut c, &sk, &kid, p, med, "patient request").await;

    let mut holder = db::connect_and_load_schema(&base).await.unwrap();
    let held = holder.transaction().await.unwrap();
    held.execute(
        "SELECT cairn_lock_custody($1::text::uuid)",
        &[&first.to_string()],
    )
    .await
    .unwrap();
    // A wait would hit this timeout and fail the shred.
    c.batch_execute("SET statement_timeout = '5s'")
        .await
        .unwrap();
    shred(&mut c, &sk, &kid, second).await;
    held.rollback().await.unwrap();
}
