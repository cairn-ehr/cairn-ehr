//! #603 — a late key and connect-time re-adjudication for the SAME deferred event, in two
//! concurrent transactions.
//!
//! # The defect
//!
//! A deferred sealed event (admitted uninterpreted, so it carries an `event_deferred` marker) whose
//! type is now classified, held WITHOUT its key. Two writers can bring it to the chart:
//!
//! * the door that lands its key (step 9 writes `event_clear`; `cairn_project_late_custody` then
//!   projects it — but only if the event is replay-eligible, i.e. carries no marker), and
//! * `cairn_readjudicate_deferred` (db/043), whose gate 4 projects it through the same heal-safe
//!   dispatch and then deletes the marker.
//!
//! Under READ COMMITTED each one ran on what the other had COMMITTED, so in either order both
//! skipped: re-adjudication projected nothing (it could not see the uncommitted clear view) and
//! promoted anyway; the late key saw the still-uncommitted marker and did not project. Both
//! committed, the marker was gone, custody was held, and the medication list was empty — with
//! nothing left to retry it (the heal replay runs only on a schema change, and `requeue` reads
//! custody `held`). A record silently missing from the chart.
//!
//! # The fix under test
//!
//! The shared dispatch takes the event's custody lock (`cairn_lock_custody`, #604) before it runs
//! any applier, and the door already holds that lock from step 9. So the second to arrive waits for
//! the first to commit and then reads committed state: re-adjudication sees the key, or the late
//! key sees the marker gone.
//!
//! Harness: `late_custody_kit`'s two-transaction race (hold one open, wait until the other has
//! finished or is parked on a lock, then commit). Real Postgres, gated on `$CAIRN_TEST_PG`,
//! serialized via `db::test_serial_guard`.

mod common;
#[path = "common/late_custody_kit.rs"]
mod late_custody_kit;

use cairn_node::db;
use late_custody_kit::*;
use tokio_postgres::Client;
use uuid::Uuid;

/// The connect-time re-adjudication pass, as the loader runs it: one statement.
const READJUDICATE: &str = "SELECT count(*) FROM cairn_readjudicate_deferred()";

/// A deferred sealed assert this node holds WITHOUT its key: the precondition #603 names.
async fn deferred_without_key(c: &Client, keys: &Keys) -> SealedAssert {
    let e = sealed_assert(
        keys,
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        "amoxicillin",
        WALL,
    );
    apply_without_key(c, &e)
        .await
        .expect("premise: admitted without custody");
    c.execute(
        "INSERT INTO event_deferred (event_id, event_type) \
         VALUES ($1::text::uuid, 'clinical.medication.asserted')",
        &[&e.event_id.to_string()],
    )
    .await
    .expect("mark it deferred, as an event awaiting re-adjudication is");
    e
}

/// What both writers together must leave: promoted (no marker), custody held, ON the chart.
async fn assert_promoted_and_charted(c: &Client, e: &SealedAssert, how: &str) {
    let markers: i64 = c
        .query_one(
            "SELECT count(*) FROM event_deferred WHERE event_id = $1::text::uuid",
            &[&e.event_id.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(markers, 0, "{how}: the event is promoted");
    assert_eq!(
        custody(c, e.event_id).await,
        (1, 1),
        "{how}: custody is held"
    );
    assert_eq!(
        (
            statement_rows(c, e.medication_id).await,
            dose_seed_rows(c, e.medication_id).await
        ),
        (1, 1),
        "A PROMOTED EVENT WHOSE KEY IS HELD IS ON THE CHART ({how}): each writer skipped the \
         projection because it could not see the other's uncommitted half, and nothing retries \
         it (#603)"
    );
}

/// The issue's interleaving: the late key holds first (custody written, uncommitted); the
/// re-adjudication pass runs meanwhile. Unfixed it cannot see the clear view, projects nothing and
/// promotes; the key had already seen the marker and skipped.
#[tokio::test]
async fn re_adjudication_racing_an_uncommitted_late_key_still_charts_it() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let keys = fresh_node(&c).await;
    remove_probe(&c).await;
    let e = deferred_without_key(&c, &keys).await;

    c.batch_execute("BEGIN").await.unwrap();
    apply_with_key(&c, &e).await.expect("the key is admitted");
    assert_eq!(
        statement_rows(&c, e.medication_id).await,
        0,
        "premise: still deferred, so the late key does not project it"
    );

    let (c2, pid2) = second_connection(&base).await;
    let adjudicating = tokio::spawn(async move { c2.execute(READJUDICATE, &[]).await });
    let (observer, _) = second_connection(&base).await;
    until_finished_or_parked(&observer, pid2, &adjudicating).await;

    c.batch_execute("COMMIT").await.unwrap();
    adjudicating
        .await
        .unwrap()
        .expect("re-adjudication completes");
    assert_promoted_and_charted(&c, &e, "re-adjudication during an uncommitted late key").await;
}

/// The reverse: re-adjudication holds first (promoted with nothing to project, uncommitted); the
/// key lands meanwhile. Unfixed the key still sees the uncommitted marker and does not project.
#[tokio::test]
async fn a_late_key_racing_an_uncommitted_re_adjudication_still_charts_it() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let keys = fresh_node(&c).await;
    remove_probe(&c).await;
    let e = deferred_without_key(&c, &keys).await;

    c.batch_execute("BEGIN").await.unwrap();
    c.execute(READJUDICATE, &[])
        .await
        .expect("re-adjudication runs");

    let (c2, pid2) = second_connection(&base).await;
    let signed = e.signed.clone();
    let dek = e.dek.clone();
    let key_landing = tokio::spawn(async move {
        c2.execute(
            "SELECT apply_remote_event($1, NULL, NULL, $2)",
            &[&signed, &dek],
        )
        .await
    });
    let (observer, _) = second_connection(&base).await;
    until_finished_or_parked(&observer, pid2, &key_landing).await;

    c.batch_execute("COMMIT").await.unwrap();
    key_landing.await.unwrap().expect("the key is admitted");
    assert_promoted_and_charted(&c, &e, "late key during an uncommitted re-adjudication").await;
}

/// Put the real structural check back on `clinical.medication.asserted` and drop the probe —
/// called at test START (a predecessor that panicked may have left it, the #583 reset-at-start
/// rule) and at the end, so the registry other suites pin is never left modified.
async fn remove_gate0_probe(c: &Client) {
    c.batch_execute(
        "UPDATE cairn_event_twin_check SET check_fn = 'cairn_check_medication_assertion' \
          WHERE event_type = 'clinical.medication.asserted'; \
         DROP FUNCTION IF EXISTS cairn_test_gate0_probe(text, jsonb); \
         DROP TABLE IF EXISTS cairn_test_gate0_runs;",
    )
    .await
    .unwrap();
}

/// Swap the medication assert's structural check for a probe that records WHICH backend ran it
/// and then runs the real check, so a test can ask "did re-adjudication's own connection check
/// this body?" without changing what the check refuses.
async fn install_gate0_probe(c: &Client) {
    remove_gate0_probe(c).await;
    c.batch_execute(
        "CREATE TABLE cairn_test_gate0_runs (pid int NOT NULL); \
         CREATE FUNCTION cairn_test_gate0_probe(p_type text, b jsonb) RETURNS void \
           LANGUAGE plpgsql AS $$ BEGIN \
             INSERT INTO cairn_test_gate0_runs VALUES (pg_backend_pid()); \
             PERFORM cairn_check_medication_assertion(p_type, b); \
           END $$; \
         UPDATE cairn_event_twin_check SET check_fn = 'cairn_test_gate0_probe' \
          WHERE event_type = 'clinical.medication.asserted';",
    )
    .await
    .unwrap();
}

/// Review finding on #603's fix: gate 0 (the per-type structural check, db/043) reads the clear
/// view too, and it ran BEFORE the custody lock that gate 4 takes. In the interleaving above it
/// therefore saw no body (the key was uncommitted), skipped the check, and gate 4 — having waited
/// — then projected a body no re-adjudication gate had checked. Gates 0-4 must read ONE committed
/// state, so re-adjudication takes the lock at the top of the row.
#[tokio::test]
async fn re_adjudication_checks_the_body_a_racing_late_key_opened() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    remove_gate0_probe(&c).await;
    let keys = fresh_node(&c).await;
    remove_probe(&c).await;
    let e = deferred_without_key(&c, &keys).await;
    install_gate0_probe(&c).await;

    c.batch_execute("BEGIN").await.unwrap();
    apply_with_key(&c, &e).await.expect("the key is admitted");

    let (c2, pid2) = second_connection(&base).await;
    let adjudicating = tokio::spawn(async move { c2.execute(READJUDICATE, &[]).await });
    let (observer, _) = second_connection(&base).await;
    until_finished_or_parked(&observer, pid2, &adjudicating).await;

    c.batch_execute("COMMIT").await.unwrap();
    adjudicating
        .await
        .unwrap()
        .expect("re-adjudication completes");

    let gate0_runs: i64 = c
        .query_one(
            "SELECT count(*) FROM cairn_test_gate0_runs WHERE pid = $1",
            &[&pid2],
        )
        .await
        .unwrap()
        .get(0);
    remove_gate0_probe(&c).await;
    assert_eq!(
        gate0_runs, 1,
        "RE-ADJUDICATION MUST CHECK WHAT IT PROMOTES: gate 0 ran before the custody lock, saw no \
         body while the key was uncommitted, and skipped the structural check that gate 4 then \
         projected past"
    );
    assert_promoted_and_charted(
        &c,
        &e,
        "re-adjudication after checking the racing key's body",
    )
    .await;
}
