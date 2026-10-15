//! #787 — `chart_link::standing_link` on a `patient_link` row with no content address.
//!
//! # Why this can happen
//!
//! db/018 creates `patient_link.content_address` as `NOT NULL`, but a database that predates #115
//! received the column through an idempotent `ALTER TABLE … ADD COLUMN IF NOT EXISTS`, which
//! cannot add `NOT NULL` to a table that already holds rows. On such a database a pair's standing
//! row can carry a NULL address until the projection is rebuilt.
//!
//! # Why it matters now
//!
//! R6 runs `standing_link` on every review-panel open and inside every verdict's link-lock (CARNLK)
//! transaction. It used to read the column with `Row::get`, which PANICS on a NULL. A panic left
//! the review panel blank, with no message and no verdict, and the verdict path lost its
//! transaction mid-way. The read now says what is wrong and what fixes it: an operator's rebuild
//! of the projections. Until that rebuild, a retry is pointless, so the error is a node-state
//! refusal.
//!
//! # The remedy is a claim, so the test runs it
//!
//! The refusal names `cairn-node reproject --rebuild`. A narrower `--prefix identity.` would read
//! better but is REFUSED by `cairn_reproject` (it would truncate `patient_chart`, which
//! registrations also feed). So the test runs the named rebuild and checks that the address comes
//! back, equal to the one the applier first wrote.
//!
//! # The fixture
//!
//! A fresh test database has the column `NOT NULL`. The test therefore reproduces the legacy shape
//! inside ONE transaction that it rolls back: `DROP NOT NULL` (Postgres DDL is transactional), then
//! null a real link's address. Nothing outside the committed link fixture outlives the test.
//!
//! Real Postgres, gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.
use cairn_node::chart_link::{canonical_pair, legacy_address_refusal, standing_link};
use cairn_node::db;
use cairn_node::db_diagnosis::{refusal_scope, RefusalScope};
use uuid::Uuid;

mod common;
use common::{cs, medication_setup as setup, register_pair, submit_link_event};

#[tokio::test]
async fn a_standing_row_without_an_address_is_refused_and_the_named_rebuild_restores_it() {
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
    let written = standing_link(&c, low, high)
        .await
        .unwrap()
        .expect("the link stands")
        .content_address;

    let tx = c.transaction().await.unwrap();
    // The pre-#115 shape: the column exists, but nothing forbids a NULL in it.
    tx.batch_execute(
        "ALTER TABLE patient_link ALTER COLUMN content_address DROP NOT NULL; \
         UPDATE patient_link SET content_address = NULL",
    )
    .await
    .unwrap();

    let err = standing_link(&tx, low, high)
        .await
        .expect_err("a standing row with no address cannot be judged as shown");
    let text = format!("{err:#}");
    assert!(
        text.contains(&legacy_address_refusal(low, high)),
        "the error is the worded refusal: {text}"
    );
    assert_eq!(
        refusal_scope(&err),
        Some(RefusalScope::NodeState),
        "a retry is pointless until an operator rebuilds: {text}"
    );

    // The act the refusal names (`cairn-node reproject --rebuild` = prefix '', rebuild true).
    tx.execute("SELECT cairn_reproject('', true, 'cli')", &[])
        .await
        .unwrap();
    let rebuilt = standing_link(&tx, low, high)
        .await
        .expect("the rebuilt row reads")
        .expect("the link still stands after the rebuild");
    assert_eq!(
        rebuilt.content_address, written,
        "the rebuild writes the winner's address back"
    );

    tx.rollback().await.unwrap();
}

#[test]
fn the_legacy_refusal_names_the_rebuild_that_fixes_it() {
    let (low, high) = (Uuid::from_u128(1), Uuid::from_u128(2));
    let words = legacy_address_refusal(low, high);
    assert!(words.contains("no recorded content address"), "{words}");
    assert!(words.contains("nothing was done"), "{words}");
    assert!(
        words.ends_with("`cairn-node reproject --rebuild`"),
        "names the whole-log rebuild (a narrower prefix is refused): {words}"
    );
}
