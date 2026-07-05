//! Integration coverage for the db/026 blob self-verification floor (ADR-0013 point 11).
//! Real Postgres, gated on `$CAIRN_TEST_PG`, serialized cluster-wide via
//! `db::test_serial_guard`. Proves the hostile-client matrix: bytes that do not
//! BLAKE3-hash to the `blob_address` naming them can never sit `present = TRUE` in
//! `blob_store` — not via INSERT, not via a content swap under an already-present
//! row, not via re-keying — even for a caller with raw SQL access (principle 12).
//! The floor was previously an L2 promise only (cairn-sync verified before flipping
//! `present`); db/003 recorded that gap, db/026 closes it via `cairn_blob_verify`.

use cairn_event::{blob_address, blob_outboard};
use cairn_node::db;
use tokio_postgres::Client;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// The server-side message of a refused statement (tokio_postgres's Display is
/// just "db error"), falling back to the client-side rendering.
fn db_msg(e: &tokio_postgres::Error) -> String {
    e.as_db_error().map(|d| d.message().to_string()).unwrap_or_else(|| e.to_string())
}

/// Clean the blob tier tables so each test starts from an empty store.
async fn setup(c: &Client) {
    c.batch_execute("TRUNCATE blob_store, blob_chunk").await.unwrap();
}

/// Raw INSERT of a fully-present blob row, exactly as a bypassing client would
/// write it (correct byte_len so db/003's length CHECK is satisfied — the length
/// floor alone must NOT be enough to pass off wrong bytes).
async fn insert_present(
    c: &Client,
    addr: &[u8],
    content: &[u8],
) -> Result<u64, tokio_postgres::Error> {
    let outboard = blob_outboard(content);
    let len = content.len() as i64;
    c.execute(
        "INSERT INTO blob_store (blob_address, media_type, byte_len, content, outboard,
                                 present, fetched_at)
         VALUES ($1, 'application/octet-stream', $2, $3, $4, TRUE, clock_timestamp())",
        &[&addr, &len, &content, &outboard],
    )
    .await
}

#[tokio::test]
async fn wrong_bytes_cannot_be_marked_present() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    setup(&c).await;

    // The named blob is A; the hostile client stores B's bytes under A's address,
    // with a self-consistent byte_len so the db/003 length CHECK passes.
    let named = b"the genuine CT bytes".to_vec();
    let forged = b"not those bytes -----".to_vec(); // same spirit, different content
    let addr = blob_address(&named);
    let err = insert_present(&c, &addr, &forged)
        .await
        .expect_err("wrong bytes must not enter blob_store as present");
    let msg = db_msg(&err);
    assert!(
        msg.contains("does not hash to blob_address"),
        "rejection must be legible, got: {msg}"
    );

    // Nothing landed — not even a present=FALSE residue of the refused row.
    let n: i64 = c
        .query_one("SELECT count(*) FROM blob_store", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 0, "the refused row must not exist in any form");
}

#[tokio::test]
async fn verified_bytes_insert_present_ok() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    setup(&c).await;

    let content = b"a genuine wound photograph".to_vec();
    let addr = blob_address(&content);
    insert_present(&c, &addr, &content)
        .await
        .expect("verified bytes are accepted");
    let present: bool = c
        .query_one("SELECT present FROM blob_store WHERE blob_address = $1", &[&addr])
        .await
        .unwrap()
        .get(0);
    assert!(present);
}

#[tokio::test]
async fn content_swap_under_present_row_refused() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    setup(&c).await;

    let content = b"original verified bytes".to_vec();
    let addr = blob_address(&content);
    insert_present(&c, &addr, &content).await.unwrap();

    // Swap the bytes under the already-present row, keeping byte_len consistent so
    // only the hash floor can catch it.
    let swapped = b"tampered  bytes  here!!".to_vec();
    assert_eq!(swapped.len(), content.len(), "test premise: length CHECK stays satisfied");
    let len = swapped.len() as i64;
    let err = c
        .execute(
            "UPDATE blob_store SET content = $1, byte_len = $2 WHERE blob_address = $3",
            &[&swapped, &len, &addr],
        )
        .await
        .expect_err("content swap under a present row must be refused");
    assert!(db_msg(&err).contains("does not hash to blob_address"));

    // The original bytes survived.
    let stored: Vec<u8> = c
        .query_one("SELECT content FROM blob_store WHERE blob_address = $1", &[&addr])
        .await
        .unwrap()
        .get(0);
    assert_eq!(stored, content);
}

#[tokio::test]
async fn rekeying_present_row_refused() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    setup(&c).await;

    let content = b"bytes bound to their true address".to_vec();
    let addr = blob_address(&content);
    insert_present(&c, &addr, &content).await.unwrap();

    // Point the present row at a DIFFERENT (legitimately referenced) address.
    let other_addr = blob_address(b"a different blob entirely");
    let err = c
        .execute(
            "UPDATE blob_store SET blob_address = $1 WHERE blob_address = $2",
            &[&other_addr, &addr],
        )
        .await
        .expect_err("re-keying a present row to another address must be refused");
    assert!(db_msg(&err).contains("does not hash to blob_address"));
}

#[tokio::test]
async fn metadata_only_update_on_present_row_allowed() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    setup(&c).await;

    let content = b"stable verified bytes".to_vec();
    let addr = blob_address(&content);
    insert_present(&c, &addr, &content).await.unwrap();

    // Touch only metadata: the guard's WHEN clause must not fire (no re-hash, no
    // false rejection).
    c.execute(
        "UPDATE blob_store SET media_type = 'image/jpeg', fetched_at = clock_timestamp()
         WHERE blob_address = $1",
        &[&addr],
    )
    .await
    .expect("metadata-only update on a present row stays allowed");
}

#[tokio::test]
async fn present_without_content_refused() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    setup(&c).await;

    let addr = blob_address(b"whatever");
    let res = c
        .execute(
            "INSERT INTO blob_store (blob_address, media_type, present)
             VALUES ($1, 'application/dicom', TRUE)",
            &[&addr],
        )
        .await;
    assert!(res.is_err(), "present = TRUE without content bytes must be refused");
}

#[tokio::test]
async fn reference_then_verified_flip_is_the_honest_path() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    setup(&c).await;

    // 1. Reference learned from a signed event (present = FALSE) — the floor must
    //    not price or block the reference-eager half.
    let content = b"lazy-fetched imaging bytes".to_vec();
    let addr = blob_address(&content);
    let len = content.len() as i64;
    c.execute(
        "SELECT blob_note_reference($1, 'application/dicom', $2)",
        &[&addr, &len],
    )
    .await
    .expect("reference-only rows are untouched by the floor");

    // 2. Hostile flip first: right length, wrong bytes (the do_blobd assembly shape,
    //    but bypassing L2's verify) — refused.
    let forged = b"assembled garbage bytes...".to_vec();
    assert_eq!(forged.len(), content.len());
    let forged_ob = blob_outboard(&forged);
    let err = c
        .execute(
            "UPDATE blob_store SET content = $1, outboard = $2, present = TRUE,
                    fetched_at = clock_timestamp()
             WHERE blob_address = $3",
            &[&forged, &forged_ob, &addr],
        )
        .await
        .expect_err("an unverified assembly flip must be refused");
    assert!(db_msg(&err).contains("does not hash to blob_address"));

    // 3. The genuine bytes flip fine (the honest do_blobd path is unchanged).
    let ob = blob_outboard(&content);
    c.execute(
        "UPDATE blob_store SET content = $1, outboard = $2, present = TRUE,
                fetched_at = clock_timestamp()
         WHERE blob_address = $3",
        &[&content, &ob, &addr],
    )
    .await
    .expect("verified bytes flip a reference row to present");
    let present: bool = c
        .query_one("SELECT present FROM blob_store WHERE blob_address = $1", &[&addr])
        .await
        .unwrap()
        .get(0);
    assert!(present);
}
