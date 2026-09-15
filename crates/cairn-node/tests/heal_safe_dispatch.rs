//! #584 / ADR-0070 — the two SQL helpers a late key reaches the chart through, called directly.
//!
//! `cairn_projection_dispatch_heal_safe(event_log)` runs ONE stored event's heal-safe registered
//! appliers; `db/043`'s gate 4 and the late-custody path share it, so "which appliers may run
//! again over a live row" is spelled once. `cairn_project_late_custody(uuid)` is what the two
//! doors call: it loads the stored row and dispatches only when the row is replay-eligible.
//!
//! The door behaviour is `late_custody_reaches_the_chart.rs`; this file pins the helpers on their
//! own, with the clear view written BY HAND, so a failure here is about the helper and never about
//! a door.
//!
//! Real Postgres, gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.

mod common;
#[path = "common/late_custody_kit.rs"]
mod late_custody_kit;

use cairn_node::db;
use late_custody_kit::*;
use tokio_postgres::Client;
use uuid::Uuid;

/// Admit `e` with no key, then write its clear view directly — the state a late custody landing
/// leaves, minus the door. `event_dek` is omitted: no projection reads it.
async fn admitted_then_made_readable(c: &Client, e: &SealedAssert) {
    apply_without_key(c, e)
        .await
        .expect("admitted without custody");
    // Bound as TEXT and cast: cairn-node's tokio-postgres has no serde_json feature, so a
    // `serde_json::Value` cannot be a jsonb parameter directly.
    c.execute(
        "INSERT INTO event_clear (event_id, body, twin) VALUES ($1::text::uuid, $2::text::jsonb, $3)",
        &[&e.event_id.to_string(), &e.clear_payload.to_string(), &e.twin],
    )
    .await
    .expect("write the clear view by hand");
}

/// The dispatch runs the heal-safe applier again and never the other one.
///
/// The FIRST admission runs both through the `AFTER INSERT` trigger, which ignores `heal_safe`
/// (a fresh insert is not a replay). Only the direct call distinguishes them.
#[tokio::test]
async fn the_dispatch_runs_only_heal_safe_appliers() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let keys = fresh_node(&c).await;
    install_probe(&c, "clinical.medication.asserted").await;

    let e = sealed_assert(
        &keys,
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        "amoxicillin",
        WALL,
    );
    let admitted = apply_without_key(&c, &e).await;
    let dispatched = c
        .execute(
            "SELECT cairn_projection_dispatch_heal_safe(el) FROM event_log el \
             WHERE el.event_id = $1::text::uuid",
            &[&e.event_id.to_string()],
        )
        .await;
    let (safe, unsafe_) = (probe_runs(&c, "safe").await, probe_runs(&c, "unsafe").await);
    remove_probe(&c).await; // BEFORE asserting: no residue in a pinned-count registry

    admitted.expect("admitted without custody");
    dispatched.expect("the dispatch runs");
    assert_eq!(safe, 2, "admission ran it once and the dispatch once more");
    assert_eq!(
        unsafe_, 1,
        "a heal_safe = false applier is never re-run over a live row — that is what the flag means"
    );
}

/// With the body readable and the row eligible, the helper builds the chart: BOTH appliers of the
/// type (statement and dose seed).
#[tokio::test]
async fn late_custody_projection_builds_the_chart_for_an_eligible_row() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let keys = fresh_node(&c).await;
    remove_probe(&c).await;

    let e = sealed_assert(
        &keys,
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        "amoxicillin",
        WALL,
    );
    admitted_then_made_readable(&c, &e).await;
    assert_eq!(
        statement_rows(&c, e.medication_id).await,
        0,
        "premise: writing event_clear alone projects nothing — the trigger is on event_log"
    );

    c.execute(
        "SELECT cairn_project_late_custody($1::text::uuid)",
        &[&e.event_id.to_string()],
    )
    .await
    .expect("the helper runs");
    assert_eq!(statement_rows(&c, e.medication_id).await, 1);
    assert_eq!(dose_seed_rows(&c, e.medication_id).await, 1);
}

/// A row carrying an `event_deferred` marker is never projected — not even with its body readable.
/// The marker means its classification-gated checks have not passed (ADR-0056); only
/// `cairn_readjudicate_deferred` may grant it power.
#[tokio::test]
async fn late_custody_projection_skips_a_deferred_row() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let keys = fresh_node(&c).await;
    remove_probe(&c).await;

    let e = sealed_assert(
        &keys,
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        "amoxicillin",
        WALL,
    );
    admitted_then_made_readable(&c, &e).await;
    c.execute(
        "INSERT INTO event_deferred (event_id, event_type) \
         VALUES ($1::text::uuid, 'clinical.medication.asserted')",
        &[&e.event_id.to_string()],
    )
    .await
    .expect("mark the row deferred, as a failed re-adjudication leaves it");

    c.execute(
        "SELECT cairn_project_late_custody($1::text::uuid)",
        &[&e.event_id.to_string()],
    )
    .await
    .expect("the helper runs");
    assert_eq!(
        statement_rows(&c, e.medication_id).await,
        0,
        "a deferred row must not project through the late-custody path"
    );
}

/// An id with no `event_log` row is a silent no-op, not an error: both doors call the helper only
/// after their own INSERT, so this arm is defensive, and a defensive arm must not become a refusal.
#[tokio::test]
async fn late_custody_projection_of_an_unknown_event_does_nothing() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.execute(
        "SELECT cairn_project_late_custody($1::text::uuid)",
        &[&Uuid::now_v7().to_string()],
    )
    .await
    .expect("an unknown id is not an error");
}
