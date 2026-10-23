//! #857 — a later stop must not hide an earlier stop's reason on the medication list.
//!
//! `medication_cessation` keeps ONE row per thread (the newest stop wins, db/031) and
//! `medication_group_cessation` ONE per line (db/033), so a second stop — admitted on purpose,
//! ADR-0082 decision 4 — used to replace "angioedema" with "patient request", or with nothing.
//! db/060 keeps one row per stop that gave a reason (`medication_cessation_reason`), and the
//! list carries every OTHER distinct reason beside the one it shows
//! (`MedicationRow::other_cessation_reasons`).
//!
//! #859 — `medication-cease` stops ONE thread; a line kept current by a sibling (one whose dose
//! is effective after this stop's date) now says so after the stop
//! (`medication::still_current_note`).
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.
use cairn_event::SigningKey;
use cairn_node::db;
use cairn_node::medication::read::list_patient_medications;
use cairn_node::medication::{
    cease_medication, reconcile_medications, still_current_note, CeaseMedicationInput,
    ReconcileInput,
};
use cairn_node::shred::shred_event;
use tokio_postgres::Client;
use uuid::Uuid;

mod common;
#[path = "common/medication_kit.rs"]
mod medication_kit;
use medication_kit::*;

/// Stop `med` with `reason`; returns the cessation's event id.
async fn stop(
    c: &mut Client,
    sk: &SigningKey,
    kid: &str,
    p: Uuid,
    med: Uuid,
    reason: Option<&str>,
) -> Uuid {
    let input = CeaseMedicationInput {
        stopped: None,
        stopped_precision: None,
        reason,
    };
    cease_medication(c, sk, kid, "test-node", p, med, &input, None, None)
        .await
        .unwrap()
}

/// The one line on `p`'s list: (the reason it shows, every other reason).
async fn reasons(c: &Client, p: Uuid) -> (Option<String>, Vec<String>) {
    let list = list_patient_medications(c, p).await.unwrap();
    assert_eq!(list.rows.len(), 1, "one line expected");
    let row = &list.rows[0];
    (
        row.cessation_reason.clone(),
        row.other_cessation_reasons.clone(),
    )
}

#[tokio::test]
async fn a_second_stop_keeps_the_first_stops_reason_on_the_list() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    stop(&mut c, &sk, &kid, p, med, Some("angioedema")).await;
    assert_eq!(reasons(&c, p).await, (Some("angioedema".into()), vec![]));

    stop(&mut c, &sk, &kid, p, med, Some("patient request")).await;
    assert_eq!(
        reasons(&c, p).await,
        (Some("patient request".into()), vec!["angioedema".into()]),
        "the list shows the latest stop's reason and keeps the earlier one beside it"
    );
}

#[tokio::test]
async fn a_reasonless_stop_does_not_hide_an_adverse_reaction() {
    // The issue's worst case: `--reason` is optional, so a later stop with none used to
    // leave the line saying nothing at all.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    stop(&mut c, &sk, &kid, p, med, Some("angioedema")).await;
    stop(&mut c, &sk, &kid, p, med, None).await;
    assert_eq!(reasons(&c, p).await, (None, vec!["angioedema".into()]));
}

#[tokio::test]
async fn a_repeated_reason_is_listed_once_and_never_beside_itself() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    stop(&mut c, &sk, &kid, p, med, Some("rash")).await;
    stop(&mut c, &sk, &kid, p, med, Some("angioedema")).await;
    stop(&mut c, &sk, &kid, p, med, Some("rash")).await;
    stop(&mut c, &sk, &kid, p, med, Some("angioedema")).await;
    assert_eq!(
        reasons(&c, p).await,
        (Some("angioedema".into()), vec!["rash".into()])
    );
}

#[tokio::test]
async fn a_sibling_threads_stop_reason_reaches_the_line() {
    // The second place a reason was lost: the line keeps ONE member's stop (db/033).
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
    stop(&mut c, &sk, &kid, p, x, Some("angioedema")).await;
    stop(&mut c, &sk, &kid, p, y, Some("patient request")).await;
    assert_eq!(
        reasons(&c, p).await,
        (Some("patient request".into()), vec!["angioedema".into()])
    );
}

#[tokio::test]
async fn a_shredded_stop_takes_its_reason_with_it() {
    // ADR-0005 rung 3: a shred that leaves the body's text readable in ANY projection is not
    // a shred. The new per-stop rows carry clinical plaintext, so db/037 scrubs them too.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let first = stop(&mut c, &sk, &kid, p, med, Some("angioedema")).await;
    stop(&mut c, &sk, &kid, p, med, Some("patient request")).await;
    shred_event(
        &mut c,
        &sk,
        &kid,
        "test-node",
        first,
        "retention ceiling",
        None,
    )
    .await
    .expect("shred the first stop");
    assert_eq!(
        reasons(&c, p).await,
        (Some("patient request".into()), vec![])
    );
    let left: i64 = c
        .query_one(
            "SELECT count(*) FROM medication_cessation_reason WHERE reason = 'angioedema'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(left, 0, "the shredded stop's reason row is gone");
}

#[tokio::test]
async fn a_heal_backfills_the_reasons_of_stops_recorded_before_db_060() {
    // A node upgrading to generation 60 already holds stops: the loader's generation-change
    // heal (`cairn_reproject('', false, 'loader')`) must fill the new table from the log.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    stop(&mut c, &sk, &kid, p, med, Some("angioedema")).await;
    stop(&mut c, &sk, &kid, p, med, Some("patient request")).await;
    c.batch_execute("TRUNCATE medication_cessation_reason")
        .await
        .unwrap();
    assert_eq!(reasons(&c, p).await.1, Vec::<String>::new());
    c.execute(
        "SELECT count(*) FROM cairn_reproject('', false, 'test')",
        &[],
    )
    .await
    .unwrap();
    assert_eq!(
        reasons(&c, p).await,
        (Some("patient request".into()), vec!["angioedema".into()])
    );
}

#[tokio::test]
async fn a_stop_of_one_thread_on_a_line_kept_current_names_the_sibling() {
    // #859: X stopped, sibling Y still active, so the line still reads current.
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
    // Backdated: an undated stop is effective NOW, which outranks Y's older dose, and the line
    // then reads stopped (latest-effective-wins, ADR-0047). A stop dated before Y's dose is
    // the case that leaves the line current.
    cease(&mut c, &sk, &kid, p, x, Some("2020")).await;
    let note = still_current_note(&c, p, x)
        .await
        .unwrap()
        .expect("the line still reads current");
    assert!(note.contains("still reads current"), "{note}");
    assert!(note.contains(&y.to_string()), "names the sibling: {note}");
    assert!(
        !note.contains(&x.to_string()),
        "never the stopped thread: {note}"
    );

    stop(&mut c, &sk, &kid, p, y, None).await;
    assert_eq!(
        still_current_note(&c, p, y).await.unwrap(),
        None,
        "a line that now reads stopped says nothing extra"
    );
}

#[tokio::test]
async fn a_stop_of_a_lone_thread_says_nothing_extra() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    stop(&mut c, &sk, &kid, p, med, None).await;
    assert_eq!(still_current_note(&c, p, med).await.unwrap(), None);
    assert_eq!(
        still_current_note(&c, p, Uuid::now_v7()).await.unwrap(),
        None,
        "a thread on no line here: the note read before the stop already said so"
    );
}

#[tokio::test]
async fn other_reasons_come_newest_recorded_first() {
    // Pins the read's ORDER BY: with one "other" reason, ascending order passes too.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    for reason in ["rash", "angioedema", "patient request"] {
        stop(&mut c, &sk, &kid, p, med, Some(reason)).await;
    }
    assert_eq!(
        reasons(&c, p).await,
        (
            Some("patient request".into()),
            vec!["angioedema".into(), "rash".into()]
        )
    );
}

#[tokio::test]
async fn a_sibling_already_stopped_is_not_named_as_unstopped() {
    // Pins still_current_note's filter: of X, Y and Z, Y was stopped before, Z keeps the line
    // current, and X is stopped now. Only Z has no stop.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, x) = chart_with_one_med(&mut c, &sk, &kid).await;
    let y = add_med(&mut c, &sk, &kid, p).await;
    let z = add_med(&mut c, &sk, &kid, p).await;
    let input = ReconcileInput {
        provenance: "clinician-judgment",
        reason: None,
    };
    for other in [y, z] {
        reconcile_medications(
            &mut c,
            &sk,
            &kid,
            "test-node",
            p,
            x,
            other,
            &input,
            None,
            None,
        )
        .await
        .unwrap();
    }
    cease(&mut c, &sk, &kid, p, y, Some("2020")).await;
    cease(&mut c, &sk, &kid, p, x, Some("2020")).await;
    let note = still_current_note(&c, p, x).await.unwrap().unwrap();
    assert!(note.contains(&z.to_string()), "names Z: {note}");
    assert!(
        !note.contains(&y.to_string()),
        "never the stopped Y: {note}"
    );
}
