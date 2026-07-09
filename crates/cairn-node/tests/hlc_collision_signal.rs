//! Advisory Byzantine-collision signal (#157): the pure `cairn_hlc_triple_collision` predicate
//! and the convergent `cairn_record_hlc_collision` recorder, tested directly (no event builders).
//! The five per-overlay integration assertions live in `overlay_tiebreaker.rs` (they reuse that
//! file's event builders). Real Postgres, gated on `$CAIRN_TEST_PG`, serialized cluster-wide via
//! `db::test_serial_guard`.
use cairn_node::db;
use tokio_postgres::Client;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// Evaluate the pure collision predicate in the DB. Current side is nullable (mirrors `wins()` in
/// overlay_tiebreaker.rs) — the null-current case is the "no winner yet" row.
#[allow(clippy::too_many_arguments)] // mirrors the same-shaped `wins()` helper in overlay_tiebreaker.rs
async fn collides(
    c: &Client,
    nw: i64,
    nc: i32,
    no: &str,
    na: Vec<u8>,
    cw: Option<i64>,
    cc: Option<i32>,
    co: Option<&str>,
    ca: Option<Vec<u8>>,
) -> bool {
    c.query_one(
        "SELECT cairn_hlc_triple_collision($1,$2,$3,$4,$5,$6,$7,$8)",
        &[&nw, &nc, &no, &na, &cw, &cc, &co, &ca],
    )
    .await
    .unwrap()
    .get(0)
}

#[tokio::test]
async fn triple_collision_predicate_is_equal_triple_distinct_address() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    // Equal (wall, counter, origin) but DISTINCT content_address → collision.
    assert!(
        collides(
            &c,
            5,
            3,
            "peer",
            vec![1],
            Some(5),
            Some(3),
            Some("peer"),
            Some(vec![2])
        )
        .await
    );
    assert!(
        collides(
            &c,
            5,
            3,
            "peer",
            vec![2],
            Some(5),
            Some(3),
            Some("peer"),
            Some(vec![1])
        )
        .await
    );
    // Identical address (same event, an idempotent re-apply) → NOT a collision.
    assert!(
        !collides(
            &c,
            5,
            3,
            "peer",
            vec![7],
            Some(5),
            Some(3),
            Some("peer"),
            Some(vec![7])
        )
        .await
    );
    // Any triple difference → NOT a collision, even with distinct addresses.
    assert!(
        !collides(
            &c,
            6,
            3,
            "peer",
            vec![1],
            Some(5),
            Some(3),
            Some("peer"),
            Some(vec![2])
        )
        .await
    ); // wall
    assert!(
        !collides(
            &c,
            5,
            4,
            "peer",
            vec![1],
            Some(5),
            Some(3),
            Some("peer"),
            Some(vec![2])
        )
        .await
    ); // counter
    assert!(
        !collides(
            &c,
            5,
            3,
            "peerX",
            vec![1],
            Some(5),
            Some(3),
            Some("peer"),
            Some(vec![2])
        )
        .await
    );
    // origin
    // Null current side (absent winner, e.g. a note-only patient_chart row) → NOT a collision.
    assert!(!collides(&c, 5, 3, "peer", vec![1], None, None, None, None).await);
}

#[tokio::test]
async fn recorder_canonicalizes_and_dedups_the_unordered_pair() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE hlc_collision_log").await.unwrap();

    let a: Vec<u8> = vec![0x11, 0x20, 0x01];
    let b: Vec<u8> = vec![0x11, 0x20, 0x02]; // b > a by byte comparison

    // Record the SAME collision twice with the address arguments in OPPOSITE order — the
    // set-union convergence claim: arrival order must not change the stored row.
    c.execute(
        "SELECT cairn_record_hlc_collision('t','s',5,3,'peer',$1,$2)",
        &[&a, &b],
    )
    .await
    .unwrap();
    c.execute(
        "SELECT cairn_record_hlc_collision('t','s',5,3,'peer',$1,$2)",
        &[&b, &a],
    )
    .await
    .unwrap();

    let row = c
        .query_one(
            "SELECT count(*)::int, min(addr_lo), max(addr_hi) FROM hlc_collision_log WHERE overlay='t'",
            &[],
        )
        .await
        .unwrap();
    let n: i32 = row.get(0);
    let lo: Vec<u8> = row.get(1);
    let hi: Vec<u8> = row.get(2);
    assert_eq!(n, 1, "the unordered pair dedups to exactly one row");
    assert_eq!(lo, a, "addr_lo is the byte-lesser address");
    assert_eq!(hi, b, "addr_hi is the byte-greater address");
}
