//! #787 — `chart_link::standing_link` on a `patient_link` row with no content address.
//!
//! # Why this can happen
//!
//! db/018 creates `patient_link.content_address` as `NOT NULL`, but a database that predates #115
//! received the column through an idempotent `ALTER TABLE … ADD COLUMN IF NOT EXISTS`, which
//! cannot add `NOT NULL` to a table that already holds rows. On such a database a pair's standing
//! row can carry a NULL address until the identity projections are replayed.
//!
//! # Why it matters now
//!
//! R6 runs `standing_link` on every review-panel open and inside every verdict's link-lock (CARNLK)
//! transaction. It used to read the column with `Row::get`, which PANICS on a NULL. A panic left
//! the review panel blank, with no message and no verdict, and the verdict path lost its
//! transaction mid-way. The read now says what is wrong and what fixes it. Until an operator acts,
//! a retry is pointless, so the error is a node-state refusal.
//!
//! # The remedy is a claim, so the test runs it
//!
//! The refusal names two commands, lightest first: a HEAL of the identity projections
//! (`cairn-node reproject --prefix identity.`, which deletes nothing), and a whole `--rebuild` as
//! the fallback (it truncates every projection and holds chart reads while it replays). The test
//! nulls the address, runs the heal and checks the address comes back. Then it nulls it again,
//! runs the rebuild and checks again. Each time the address must equal the one the applier first
//! wrote.
//!
//! # The fixture
//!
//! A fresh test database has the column `NOT NULL`. The test therefore reproduces the legacy shape
//! inside ONE transaction that it rolls back: `DROP NOT NULL` (Postgres DDL is transactional), then
//! null a real link's address. Only the committed link fixture outlives the test.
//!
//! Real Postgres, gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.
use cairn_node::chart_link::{canonical_pair, legacy_address_refusal, standing_link};
use cairn_node::db;
use cairn_node::db_diagnosis::{refusal_scope, RefusalScope};
use tokio_postgres::Transaction;
use uuid::Uuid;

mod common;
use common::{cs, medication_setup as setup, register_pair, submit_link_event};

/// Null every standing row's address (the pre-#115 shape), then check the read refuses it, worded
/// as [`legacy_address_refusal`] and scoped to the node's state.
async fn null_and_expect_refusal(tx: &Transaction<'_>, low: Uuid, high: Uuid) {
    tx.batch_execute("UPDATE patient_link SET content_address = NULL")
        .await
        .unwrap();
    let err = standing_link(tx, low, high)
        .await
        .expect_err("a standing row with no address cannot be compared");
    let text = format!("{err:#}");
    assert!(
        text.contains(&legacy_address_refusal(low, high)),
        "the error is the worded refusal: {text}"
    );
    assert_eq!(
        refusal_scope(&err),
        Some(RefusalScope::NodeState),
        "a retry is pointless until an operator acts: {text}"
    );
}

/// The standing row's address after a remedy ran.
async fn address_now(tx: &Transaction<'_>, low: Uuid, high: Uuid) -> Vec<u8> {
    standing_link(tx, low, high)
        .await
        .expect("the remedied row reads")
        .expect("the link still stands after the remedy")
        .content_address
}

#[tokio::test]
async fn a_standing_row_without_an_address_is_refused_and_both_named_remedies_restore_it() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE patient_link, person_member")
        .await
        .unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let (low, high) = canonical_pair(Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk, &kid, low, high).await;
    submit_link_event(&c, &sk, &kid, low, high, 10, true).await;
    let written = address_now_on(&c, low, high).await;

    let tx = c.transaction().await.unwrap();
    // The pre-#115 shape: the column exists, but nothing forbids a NULL in it.
    tx.batch_execute("ALTER TABLE patient_link ALTER COLUMN content_address DROP NOT NULL")
        .await
        .unwrap();

    // The first remedy named: `cairn-node reproject --prefix identity.` (heal: no deletes).
    null_and_expect_refusal(&tx, low, high).await;
    tx.execute("SELECT cairn_reproject('identity.', false, 'cli')", &[])
        .await
        .unwrap();
    assert_eq!(
        address_now(&tx, low, high).await,
        written,
        "the heal replays the winner, and its address beats the NULL"
    );

    // The fallback named: `cairn-node reproject --rebuild` (prefix '', rebuild true).
    null_and_expect_refusal(&tx, low, high).await;
    tx.execute("SELECT cairn_reproject('', true, 'cli')", &[])
        .await
        .unwrap();
    assert_eq!(
        address_now(&tx, low, high).await,
        written,
        "the rebuild writes the winner's address back"
    );

    tx.rollback().await.unwrap();
}

/// [`address_now`] outside a transaction, for the fixture's first read.
async fn address_now_on(c: &tokio_postgres::Client, low: Uuid, high: Uuid) -> Vec<u8> {
    standing_link(c, low, high)
        .await
        .unwrap()
        .expect("the link stands")
        .content_address
}

#[test]
fn the_legacy_refusal_names_the_heal_first_and_the_rebuild_as_the_fallback() {
    let (low, high) = (Uuid::from_u128(1), Uuid::from_u128(2));
    let words = legacy_address_refusal(low, high);
    assert!(words.contains("no recorded content address"), "{words}");
    // The review panel shows this to a clinician, so the cause is said in words; the issue
    // number stays only as the operator's reference.
    assert!(
        words.contains("written by an older version of this node"),
        "{words}"
    );
    assert!(words.contains("nothing was done"), "{words}");
    assert!(words.contains("owner-privileged connection"), "{words}");
    let heal = words
        .find("`cairn-node reproject --prefix identity.`")
        .expect("names the heal");
    let rebuild = words
        .find("`cairn-node reproject --rebuild`")
        .expect("names the rebuild as a fallback");
    assert!(heal < rebuild, "the lighter remedy comes first: {words}");
    assert!(
        !words.contains("--prefix identity. --rebuild"),
        "cairn_reproject refuses that combination: {words}"
    );
}
