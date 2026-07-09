//! Deterministic HLC-overlay tiebreaker (#115): the shared `cairn_hlc_overlay_wins()`
//! predicate, and arrival-order-independent convergence of the five state overlays when
//! two DISTINCT events share an identical (wall, counter, origin) triple (a Byzantine /
//! broken-signer collision). Real Postgres, gated on `$CAIRN_TEST_PG`, serialized
//! cluster-wide via `db::test_serial_guard`.
use cairn_node::db;
use tokio_postgres::Client;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// Evaluate the pure predicate in the DB. `na`/`ca` are the content-address bytes (current
/// side nullable — the "no winner yet" case an overlay hits on its first insert).
#[allow(clippy::too_many_arguments)] // mirrors the same-shaped helpers in match_veto.rs / demographics_names.rs
async fn wins(
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
        "SELECT cairn_hlc_overlay_wins($1,$2,$3,$4,$5,$6,$7,$8)",
        &[&nw, &nc, &no, &na, &cw, &cc, &co, &ca],
    )
    .await
    .unwrap()
    .get(0)
}

#[tokio::test]
async fn overlay_predicate_is_a_deterministic_total_order() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    // Higher wall wins regardless of everything after it.
    assert!(
        wins(
            &c,
            2,
            0,
            "a",
            vec![1],
            Some(1),
            Some(9),
            Some("z"),
            Some(vec![9])
        )
        .await
    );
    // Lower wall loses regardless of everything after it.
    assert!(
        !wins(
            &c,
            1,
            9,
            "z",
            vec![9],
            Some(2),
            Some(0),
            Some("a"),
            Some(vec![1])
        )
        .await
    );
    // Equal (wall, counter, origin): the content_address breaks the tie deterministically.
    assert!(
        wins(
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
    assert!(
        !wins(
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
    // A full tie (same address too) is NOT a win — strict-greater, so an idempotent re-apply
    // never churns the row.
    assert!(
        !wins(
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
    // No current winner yet (COALESCE path): a real event always beats the absent row.
    assert!(wins(&c, 0, 0, "", vec![0], None, None, None, None).await);
}
