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
use cairn_event::{sign, SigningKey};
use cairn_medication_view::MedicationStatus;
use cairn_node::db;
use cairn_node::medication::read::list_patient_medications;
use cairn_node::medication::{
    build_cease_body, cease_medication, code_medication, reconcile_medications,
    separate_medications, CeaseMedicationInput, CodeMedicationInput, ReconcileInput,
};
use cairn_node::shred::{build_shred_body, shred_event};
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

/// A peer's PLAINTEXT stop of `med` under `p`, admitted through the sync door (which takes a
/// non-conformant peer's unsealed clinical body, ADR-0052). Returns the event id.
async fn remote_stop(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    p: Uuid,
    med: Uuid,
    reason: &str,
) -> Uuid {
    let id = Uuid::now_v7();
    let input = CeaseMedicationInput {
        stopped: None,
        stopped_precision: None,
        reason: Some(reason),
    };
    let hlc = db::next_hlc(c, "peer-node").await.unwrap();
    let body = build_cease_body(id, med, p, &input, kid, hlc);
    let signed = sign(&body, sk).unwrap();
    c.execute("SELECT apply_remote_event($1)", &[&signed.signed_bytes])
        .await
        .expect("the sync door admits a peer's plaintext stop");
    id
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

/// The posture the re-fold leaves behind when the caller's was `before`.
async fn posture_after_refold(c: &mut Client, before: &str) -> Option<String> {
    let tx = c.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('cairn.remote_apply', $1, true)",
        &[&before],
    )
    .await
    .unwrap();
    tx.query_one(
        "SELECT cairn_refold_after_shred(gen_random_uuid(), \
                                          'clinical.medication-cessation.asserted', \
                                          ARRAY[gen_random_uuid()::text]), \
                current_setting('cairn.remote_apply', true)",
        &[],
    )
    .await
    .unwrap()
    .get(1)
}

#[tokio::test]
async fn the_refold_puts_the_callers_apply_posture_back() {
    // The re-fold replays lenient. Left 'on', every later write in the strict door's
    // transaction would skip its raise-not-clamp checks — a floor bypass. A caller that chose
    // 'on' (cairn_reproject's heal runs lenient) must get 'on' back.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    assert_eq!(
        posture_after_refold(&mut c, "off").await.as_deref(),
        Some("off")
    );
    assert_eq!(
        posture_after_refold(&mut c, "on").await.as_deref(),
        Some("on")
    );
}

#[tokio::test]
async fn a_thread_is_matched_however_its_uuid_is_spelt() {
    // The floor admits any spelling Postgres casts to uuid and the appliers key on the cast; a
    // foreign client's upper-case id must still name the same thread (PR #865 review).
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let id = Uuid::now_v7();
    let named: Vec<String> = c
        .query_one(
            "SELECT cairn_medication_threads_named(jsonb_build_object( \
                 'medication_id', upper($1), 'subject_a', 'not-a-uuid'))",
            &[&id.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        named,
        vec![id.to_string()],
        "canonical, and a malformed id names nothing"
    );
}

#[tokio::test]
async fn a_shred_waits_for_a_sibling_whose_holder_rolls_back_then_folds_it() {
    // PR #865 review: the re-fold used to SKIP a sibling whose custody lock another transaction
    // held, trusting the holder to write it. A holder that rolls back (or only re-receives the
    // event) writes nothing, and the drug read current. Now the shred waits and folds it.
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
    let shredder = tokio::spawn(async move {
        shred(&mut c, &sk, &kid, second).await;
        c
    });
    // Long enough for the shred to reach the held lock; it must still be waiting there.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert!(
        !shredder.is_finished(),
        "the shred waits for the sibling's lock"
    );
    held.rollback().await.unwrap();
    let c = shredder.await.unwrap();

    let list = list_patient_medications(&c, p).await.unwrap();
    assert_eq!(list.rows[0].status, MedicationStatus::Ceased);
    assert_eq!(list.rows[0].cessation_reason.as_deref(), Some("angioedema"));
}

#[tokio::test]
async fn shredding_a_separation_restores_the_reconciliation_it_overrode() {
    // The `subject_a`/`subject_b` key: a reconciliation edge is a one-winner row per pair.
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
    let apart = separate_medications(&mut c, &sk, &kid, "test-node", p, x, y, &input, None, None)
        .await
        .unwrap();
    assert_eq!(list_patient_medications(&c, p).await.unwrap().rows.len(), 2);

    shred(&mut c, &sk, &kid, apart).await;

    assert_eq!(
        list_patient_medications(&c, p).await.unwrap().rows.len(),
        1,
        "the earlier reconciliation stands again: one line"
    );
}

/// The reason on `med`'s `medication_cessation` row, if any (one-winner: at most one row).
async fn cessation_reasons(c: &Client, med: Uuid) -> Vec<Option<String>> {
    c.query(
        "SELECT reason FROM medication_cessation WHERE medication_id = $1::text::uuid",
        &[&med.to_string()],
    )
    .await
    .unwrap()
    .iter()
    .map(|r| r.get(0))
    .collect()
}

#[tokio::test]
async fn a_peers_shred_of_a_plaintext_stop_stays_scrubbed() {
    // PR #865 review: the sync door shreds a peer's PLAINTEXT event too (it degrades honestly
    // rather than freeze the watermark). A plaintext body stays in the log, so its clear payload
    // still names the thread — the re-fold used to pick up the target itself and re-insert the
    // very rows the scrub had just deleted. The target is now excluded by id.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let target = remote_stop(&c, &sk, &kid, p, med, "angioedema").await;
    assert_eq!(
        cessation_reasons(&c, med).await,
        vec![Some("angioedema".to_string())]
    );

    let hlc = db::next_hlc(&c, "peer-node").await.unwrap();
    let tomb = build_shred_body(
        Uuid::now_v7(),
        p,
        target,
        "retention ceiling",
        &kid,
        None,
        hlc,
    );
    let signed = sign(&tomb, &sk).unwrap();
    c.execute("SELECT apply_remote_event($1)", &[&signed.signed_bytes])
        .await
        .expect("the sync door admits a peer's shred of a plaintext event");

    assert!(
        cessation_reasons(&c, med).await.is_empty(),
        "the shred's scrub must stick: the re-fold must not replay the target"
    );
}

#[tokio::test]
async fn a_deferred_sibling_is_not_promoted_by_a_shred() {
    // A deferred event (admitted uninterpreted, or failed re-adjudication) stays powerless —
    // db/005: "never silently promoted". Shredding the stop that out-ranked it must not make it
    // count on the chart.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let first = stop(&mut c, &sk, &kid, p, med, "angioedema").await;
    let second = stop(&mut c, &sk, &kid, p, med, "patient request").await;
    c.execute(
        "INSERT INTO event_deferred (event_id, event_type) \
         SELECT event_id, event_type FROM event_log WHERE event_id = $1::text::uuid",
        &[&first.to_string()],
    )
    .await
    .unwrap();

    shred(&mut c, &sk, &kid, second).await;

    assert!(
        cessation_reasons(&c, med).await.is_empty(),
        "the deferred stop must not become the thread's stop"
    );
}

#[tokio::test]
async fn a_peers_flagged_stop_is_replayed_lenient_so_the_shred_goes_through() {
    // A peer's stop filed under the WRONG patient is admitted by the sync door clamped and
    // flagged; the strict posture would RAISE on it. The re-fold replays it lenient — under the
    // strict posture a local shred of the later, local stop would abort and the erasure fail.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let other = Uuid::now_v7();
    common::submit_registration(&c, &sk, &kid, other, 0).await;
    remote_stop(&c, &sk, &kid, other, med, "angioedema").await;
    let local = stop(&mut c, &sk, &kid, p, med, "patient request").await;

    shred(&mut c, &sk, &kid, local).await;

    assert_eq!(
        cessation_reasons(&c, med).await,
        vec![Some("angioedema".to_string())],
        "the peer's stop is the thread's stop again"
    );
}

/// Run the re-fold of `med`'s stops with a test trigger that makes the replay of a stop whose
/// reason is `boom` raise `code`; returns the error. Inside a transaction that is rolled back,
/// so the trigger never outlives the test.
async fn refold_failure(c: &mut Client, med: Uuid, code: &str) -> tokio_postgres::Error {
    let tx = c.transaction().await.unwrap();
    tx.batch_execute(&format!(
        "CREATE FUNCTION cairn_test_refold_boom() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN \
           IF NEW.reason = 'boom' THEN RAISE EXCEPTION 'applier blew up' USING ERRCODE = '{code}'; \
           END IF; RETURN NEW; END $$; \
         CREATE TRIGGER boom BEFORE INSERT ON medication_cessation \
           FOR EACH ROW EXECUTE FUNCTION cairn_test_refold_boom();"
    ))
    .await
    .unwrap();
    let err = tx
        .execute(
            "SELECT cairn_refold_after_shred(gen_random_uuid(), \
                 'clinical.medication-cessation.asserted', ARRAY[$1::text])",
            &[&med.to_string()],
        )
        .await
        .expect_err("the sibling's applier raises");
    tx.rollback().await.unwrap();
    err
}

#[tokio::test]
async fn a_sibling_that_fails_to_replay_is_named_and_keeps_its_class() {
    // PR #865 review: a sibling's applier error used to surface bare, as if the shred itself
    // were refused. It now names the sibling and the re-fold, and keeps its SQLSTATE so a
    // deadlock (class 40) is still classed transient by the sync pullers.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let sibling = stop(&mut c, &sk, &kid, p, med, "boom").await;

    for code in ["P0001", "40P01"] {
        let err = refold_failure(&mut c, med, code).await;
        let db_err = err.as_db_error().expect("a database error");
        assert_eq!(db_err.code().code(), code, "the sibling's SQLSTATE is kept");
        let msg = db_err.message();
        assert!(
            msg.contains("#863")
                && msg.contains(&sibling.to_string())
                && msg.contains("applier blew up"),
            "the error names the re-fold and the sibling, and keeps the cause: {msg}"
        );
    }
}
