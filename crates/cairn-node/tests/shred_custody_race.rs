//! #604 — a crypto-shred and a late key for the SAME event, in two concurrent transactions.
//!
//! # The defect
//!
//! Both doors' step 9 grants custody only `IF NOT EXISTS (erasure_shred_log row for E)`, and
//! `cairn_execute_shred` (db/037) records that row and scrubs E's custody and projections. Under
//! READ COMMITTED neither side can see the other's uncommitted rows, and nothing made them wait for
//! each other, so two interleavings ended with a shred on record and the body still readable:
//!
//! 1. **The shred holds first.** T_shred records the shred but has not committed; T_key's check sees
//!    no shred row and writes `event_dek` + `event_clear` (and, since ADR-0070, the medication list
//!    row). T_shred already ran its deletes, so it scrubbed nothing of T_key's.
//! 2. **The key holds first.** T_key has written custody but not committed; T_shred's deletes cannot
//!    see those rows, delete nothing, and commit. T_key commits and the custody is back.
//!
//! ADR-0005 / ADR-0052 promise custody never comes back after a shred, in either arrival order. The
//! fix makes both sides take one per-event transaction lock (`cairn_lock_custody`, db/005) before
//! reading or scrubbing custody, so the second one to arrive waits and then reads committed state.
//!
//! # How the interleaving is forced
//!
//! The FIRST transaction is opened on one connection and left uncommitted. The SECOND runs on a
//! separate connection inside a spawned task. The test then waits until that task has either
//! FINISHED (the unfixed door: nothing makes it wait) or is provably PARKED on a lock
//! (`pg_stat_activity.wait_event_type = 'Lock'`, the fixed door) — a real signal, never a guessed
//! sleep — and only then commits the first. Both outcomes are deterministic, so the final
//! assertion is red before the fix and green after it, with no timing luck either way.
//!
//! Real Postgres, gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.

mod common;
#[path = "common/late_custody_kit.rs"]
mod late_custody_kit;

use cairn_node::db;
use late_custody_kit::*;
use tokio_postgres::Client;
use uuid::Uuid;

/// INTERLEAVING 1 — the shred is recorded but uncommitted when the key arrives through `door`
/// (the SQL that hands the door E's bytes and DEK). The key must wait for the shred and then find
/// it, so custody is never granted. Shared by the sync-door and strict-door tests below, because
/// each door takes its own lock in its own step 9 and a mutation dropping either must go red.
async fn a_key_racing_an_uncommitted_shred(door: &'static str) {
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
    apply_without_key(&c, &e)
        .await
        .expect("premise: admitted without custody");

    // T_shred: open, shred E, and hold the transaction open.
    c.batch_execute("BEGIN").await.unwrap();
    c.execute("SELECT apply_remote_event($1)", &[&shred_of(&keys, &e)])
        .await
        .expect("the shred is admitted");

    // T_key: E's key, on its own connection, while T_shred is still open.
    let (c2, pid2) = second_connection(&base).await;
    let signed = e.signed.clone();
    let dek = e.dek.clone();
    let key_landing = tokio::spawn(async move { c2.execute(door, &[&signed, &dek]).await });
    let (observer, _) = second_connection(&base).await;
    until_finished_or_parked(&observer, pid2, &key_landing).await;

    c.batch_execute("COMMIT").await.unwrap();
    key_landing
        .await
        .unwrap()
        .expect("the key's event is admitted (set-union), custody or not");

    assert_eq!(
        custody(&c, e.event_id).await,
        (0, 0),
        "A SHRED ON RECORD MEANS NO CUSTODY ({door}): the key raced an uncommitted shred and \
         must have waited for it, then found it — instead it wrote event_dek/event_clear the \
         shred could no longer see (#604)"
    );
    assert_eq!(
        statement_rows(&c, e.medication_id).await,
        0,
        "and the late-custody landing (ADR-0070) must not have put the shredded record back on \
         the medication list ({door})"
    );
}

/// Interleaving 1 at the SYNC door (db/020's step 9 lock).
#[tokio::test]
async fn a_key_racing_an_uncommitted_shred_is_never_granted_custody() {
    a_key_racing_an_uncommitted_shred("SELECT apply_remote_event($1, NULL, NULL, $2)").await;
}

/// Interleaving 1 at the STRICT door (db/005's step 9 lock). The chart already holds E (keyless),
/// so #345's registration-precedence rule does not refuse this re-submit.
#[tokio::test]
async fn a_key_racing_an_uncommitted_shred_at_the_strict_door_is_never_granted_custody() {
    a_key_racing_an_uncommitted_shred("SELECT submit_event($1, NULL, NULL, $2)").await;
}

/// INTERLEAVING 2 — the key has landed but is uncommitted when the shred arrives. The shred must
/// wait for it and then scrub what it wrote, so the committed key does not resurrect custody.
#[tokio::test]
async fn a_shred_racing_an_uncommitted_key_still_scrubs_it() {
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
    apply_without_key(&c, &e)
        .await
        .expect("premise: admitted without custody");

    // T_key: E's key lands (custody + the late-custody projection) and the transaction stays open.
    c.batch_execute("BEGIN").await.unwrap();
    apply_with_key(&c, &e).await.expect("the key is admitted");
    assert_eq!(
        custody(&c, e.event_id).await,
        (1, 1),
        "premise: inside T_key, custody is written"
    );

    // T_shred, on its own connection, while T_key is still open.
    let (c2, pid2) = second_connection(&base).await;
    let shred = shred_of(&keys, &e);
    let shredding =
        tokio::spawn(async move { c2.execute("SELECT apply_remote_event($1)", &[&shred]).await });
    let (observer, _) = second_connection(&base).await;
    until_finished_or_parked(&observer, pid2, &shredding).await;

    c.batch_execute("COMMIT").await.unwrap();
    shredding
        .await
        .unwrap()
        .expect("the shred is admitted and executes");

    assert_eq!(
        custody(&c, e.event_id).await,
        (0, 0),
        "A SHRED ON RECORD MEANS NO CUSTODY: the shred raced an uncommitted key and must have \
         waited for it, then scrubbed it — instead its deletes ran before the key committed and \
         the custody came back (#604)"
    );
    assert_eq!(
        statement_rows(&c, e.medication_id).await,
        0,
        "and the medication list row the late key projected is scrubbed too"
    );
}

// =============================================================================================
// #773 — the two paths that READ event_clear and WRITE projection rows without being a door:
// the heal-mode `cairn_reproject` (db/039, run by the loader after a schema change) and deferred
// re-adjudication's gate 4 (db/043). #604's lock covered only the doors and the shred, so either
// path racing a shred could put the shredded record back on the medication list. Custody stays
// gone; the projection comes back — and ADR-0005 / ADR-0052 promise the projection is scrubbed too.
// =============================================================================================

/// The heal pass the loader runs at connect after a schema change, as one statement.
const HEAL: &str = "SELECT count(*) FROM cairn_reproject('', false, 'test')";

/// The connect-time re-adjudication pass, as one statement.
const READJUDICATE: &str = "SELECT count(*) FROM cairn_readjudicate_deferred()";

/// A sealed assert admitted WITH its key, so it is on the chart: the state a shred acts on.
async fn charted_assert(c: &Client, keys: &Keys) -> SealedAssert {
    let e = sealed_assert(
        keys,
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        "amoxicillin",
        WALL,
    );
    apply_with_key(c, &e)
        .await
        .expect("premise: admitted with custody");
    assert_eq!(
        statement_rows(c, e.medication_id).await,
        1,
        "premise: on the chart"
    );
    e
}

/// Drop `e`'s projection rows by hand, leaving custody in place: the projection DEBT a heal pass
/// exists to repay. A heal over a row that already exists UPDATEs it (and so takes a row lock the
/// shred's DELETE waits on); only a heal that INSERTS a fresh row can slip past a shred's delete.
async fn owe_projection(c: &Client, e: &SealedAssert) {
    c.execute(
        "DELETE FROM medication_statement WHERE medication_id = $1::text::uuid",
        &[&e.medication_id.to_string()],
    )
    .await
    .unwrap();
    c.execute(
        "DELETE FROM medication_dose_event WHERE medication_id = $1::text::uuid",
        &[&e.medication_id.to_string()],
    )
    .await
    .unwrap();
}

/// Mark `e` deferred: admitted uninterpreted, awaiting re-adjudication (db/020 writes this marker;
/// writing it by hand reaches the same state without an unclassified event type).
async fn mark_deferred(c: &Client, e: &SealedAssert) {
    c.execute(
        "INSERT INTO event_deferred (event_id, event_type) \
         VALUES ($1::text::uuid, 'clinical.medication.asserted')",
        &[&e.event_id.to_string()],
    )
    .await
    .expect("mark it deferred");
}

/// What a shred must leave behind for `e`: no custody, and nothing of it on the medication list.
async fn assert_shredded(c: &Client, e: &SealedAssert, how: &str) {
    assert_eq!(
        custody(c, e.event_id).await,
        (0, 0),
        "{how}: custody is gone"
    );
    assert_eq!(
        (
            statement_rows(c, e.medication_id).await,
            dose_seed_rows(c, e.medication_id).await
        ),
        (0, 0),
        "A SHRED ON RECORD MEANS THE RECORD IS OFF THE CHART ({how}): the projection pass read \
         the body before the shred committed and wrote the drug back onto the medication list \
         after the shred's scrub (#773)"
    );
}

/// The shred holds first; the heal pass runs while it is uncommitted. Unfixed, the heal reads the
/// still-committed `event_clear`, its upsert waits on the row the shred deleted, and when the
/// shred commits the upsert finds no row and inserts one.
#[tokio::test]
async fn a_heal_racing_an_uncommitted_shred_does_not_re_project_it() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let keys = fresh_node(&c).await;
    remove_probe(&c).await;
    let e = charted_assert(&c, &keys).await;

    c.batch_execute("BEGIN").await.unwrap();
    c.execute("SELECT apply_remote_event($1)", &[&shred_of(&keys, &e)])
        .await
        .expect("the shred is admitted and executes");

    let (c2, pid2) = second_connection(&base).await;
    let healing = tokio::spawn(async move { c2.execute(HEAL, &[]).await });
    let (observer, _) = second_connection(&base).await;
    until_finished_or_parked(&observer, pid2, &healing).await;

    c.batch_execute("COMMIT").await.unwrap();
    healing.await.unwrap().expect("the heal pass completes");
    assert_shredded(&c, &e, "heal after an uncommitted shred").await;
}

/// The heal pass holds first, repaying a projection debt with a FRESH row; the shred runs while it
/// is uncommitted. Unfixed, the shred's DELETE cannot see the heal's uncommitted insert, deletes
/// nothing, and commits; the heal then commits the row.
#[tokio::test]
async fn a_shred_racing_an_uncommitted_heal_still_scrubs_what_it_wrote() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let keys = fresh_node(&c).await;
    remove_probe(&c).await;
    let e = charted_assert(&c, &keys).await;
    owe_projection(&c, &e).await;

    c.batch_execute("BEGIN").await.unwrap();
    c.execute(HEAL, &[]).await.expect("the heal pass runs");
    assert_eq!(
        statement_rows(&c, e.medication_id).await,
        1,
        "premise: inside the heal, the debt is repaid with a fresh row"
    );

    let (c2, pid2) = second_connection(&base).await;
    let shred = shred_of(&keys, &e);
    let shredding =
        tokio::spawn(async move { c2.execute("SELECT apply_remote_event($1)", &[&shred]).await });
    let (observer, _) = second_connection(&base).await;
    until_finished_or_parked(&observer, pid2, &shredding).await;

    c.batch_execute("COMMIT").await.unwrap();
    shredding
        .await
        .unwrap()
        .expect("the shred is admitted and executes");
    assert_shredded(&c, &e, "shred during an uncommitted heal").await;
}

/// The shred holds first; re-adjudication promotes the same (deferred, keyed) event while the
/// shred is uncommitted. Unfixed, gate 4 reads the still-committed `event_clear` and projects the
/// event into a row the shred never saw.
#[tokio::test]
async fn re_adjudication_racing_an_uncommitted_shred_does_not_project_it() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let keys = fresh_node(&c).await;
    remove_probe(&c).await;
    let e = charted_assert(&c, &keys).await;
    owe_projection(&c, &e).await;
    mark_deferred(&c, &e).await;

    c.batch_execute("BEGIN").await.unwrap();
    c.execute("SELECT apply_remote_event($1)", &[&shred_of(&keys, &e)])
        .await
        .expect("the shred is admitted and executes");

    let (c2, pid2) = second_connection(&base).await;
    let adjudicating = tokio::spawn(async move { c2.execute(READJUDICATE, &[]).await });
    let (observer, _) = second_connection(&base).await;
    until_finished_or_parked(&observer, pid2, &adjudicating).await;

    c.batch_execute("COMMIT").await.unwrap();
    adjudicating
        .await
        .unwrap()
        .expect("re-adjudication completes");
    assert_shredded(&c, &e, "re-adjudication after an uncommitted shred").await;
}

/// Re-adjudication holds first, having projected the promoted event; the shred runs while it is
/// uncommitted. Unfixed, the shred cannot see gate 4's uncommitted rows and scrubs nothing of them.
#[tokio::test]
async fn a_shred_racing_an_uncommitted_re_adjudication_still_scrubs_it() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let keys = fresh_node(&c).await;
    remove_probe(&c).await;
    let e = charted_assert(&c, &keys).await;
    owe_projection(&c, &e).await;
    mark_deferred(&c, &e).await;

    c.batch_execute("BEGIN").await.unwrap();
    c.execute(READJUDICATE, &[])
        .await
        .expect("re-adjudication runs");
    assert_eq!(
        statement_rows(&c, e.medication_id).await,
        1,
        "premise: inside re-adjudication, gate 4 projected the promoted event"
    );

    let (c2, pid2) = second_connection(&base).await;
    let shred = shred_of(&keys, &e);
    let shredding =
        tokio::spawn(async move { c2.execute("SELECT apply_remote_event($1)", &[&shred]).await });
    let (observer, _) = second_connection(&base).await;
    until_finished_or_parked(&observer, pid2, &shredding).await;

    c.batch_execute("COMMIT").await.unwrap();
    shredding
        .await
        .unwrap()
        .expect("the shred is admitted and executes");
    assert_shredded(&c, &e, "shred during an uncommitted re-adjudication").await;
}
