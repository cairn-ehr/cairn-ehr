//! #75 — the §3.13 twin blank-test must mean the SAME thing in Rust and in Postgres.
//!
//! The floor decides "did the author supply a twin, or must one be derived?" in two
//! languages: Rust (`cairn_event::twin_is_present`, used by `resolve_twin` /
//! `materialise_generic_twin` before signing) and SQL (`cairn_twin_is_present`, used by the
//! write gate `cairn_event_twin` in db/005 and by the read predicates in db/015). Those two
//! answers are only ever compared by a human reading two files — until this test.
//!
//! WHY IT MATTERS: `cairn_event_twin` is also the REMOTE-APPLY gate (db/020) and it RAISEs
//! for a hard-require type (demographics / identity / medication). If SQL called a twin
//! "authored" that Rust called blank, the same signed event could apply on one node and
//! raise on another — set-union convergence broken, not a cosmetic difference. The original
//! defect was exactly this: SQL used `\s`, whose membership Postgres decides from the
//! *collation's* ctype, so the verdict differed between a libc UTF-8 database and a
//! `C`/`ucs_basic` one.
//!
//! The check is EXHAUSTIVE rather than a sample: every scalar value in the Basic
//! Multilingual Plane is classified on both sides and the two sets must be equal. A
//! hand-picked battery would pass while a class typo silently mis-classified its neighbour.
//! The BMP is enough — the highest Unicode `White_Space` code point is U+3000, and the sweep
//! still covers ~62k non-whitespace controls, marks and CJK as negative cases.

use cairn_event::twin_is_present;
use tokio_postgres::Client;

/// Surrogate code points: not Unicode scalar values, so `char::from_u32` rejects them and
/// Postgres `chr()` refuses them. Excluded from the sweep on both sides.
const SURROGATES: std::ops::RangeInclusive<u32> = 0xD800..=0xDFFF;

/// The sweep's upper bound (inclusive) — the whole Basic Multilingual Plane.
const SWEEP_MAX: u32 = 0xFFFF;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// Code points in the sweep that Postgres can actually represent in `text`.
///
/// U+0000 is skipped: Postgres `text` cannot hold a NUL byte at all (`chr(0)` errors), so
/// there is no SQL-side answer to compare against. Rust would say a "\0" twin is present;
/// that string can never reach the database, so the disagreement is unreachable.
fn sweep() -> impl Iterator<Item = u32> {
    (1..=SWEEP_MAX).filter(|cp| !SURROGATES.contains(cp))
}

/// The set of code points the SQL floor judges BLANK — i.e. `cairn_twin_is_present` is false
/// for a twin consisting of just that one character. One round trip, not 65k.
async fn sql_blank_set(client: &Client) -> Vec<u32> {
    let rows = client
        .query(
            "SELECT cp FROM generate_series($1::int, $2::int) AS g(cp)
              WHERE cp NOT BETWEEN $3::int AND $4::int
                AND NOT cairn_twin_is_present(chr(cp))
              ORDER BY cp",
            &[
                &1_i32,
                &(SWEEP_MAX as i32),
                &(*SURROGATES.start() as i32),
                &(*SURROGATES.end() as i32),
            ],
        )
        .await
        .expect("sweep query");
    rows.iter().map(|r| r.get::<_, i32>(0) as u32).collect()
}

/// The same set as Rust sees it.
fn rust_blank_set() -> Vec<u32> {
    sweep()
        .filter(|cp| {
            let c = char::from_u32(*cp).expect("surrogates already excluded");
            !twin_is_present(Some(&c.to_string()))
        })
        .collect()
}

fn fmt(cps: &[u32]) -> String {
    cps.iter()
        .map(|cp| format!("U+{cp:04X}"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[tokio::test]
async fn sql_and_rust_agree_on_every_bmp_code_point() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let client = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();

    let from_sql = sql_blank_set(&client).await;
    let from_rust = rust_blank_set();

    // Anti-vacuity: a broken sweep that returned nothing on both sides would "agree".
    // The blank set is the 25 Unicode White_Space code points — assert the shape before
    // asserting equality, so an empty-vs-empty pass is impossible.
    assert_eq!(
        from_rust.len(),
        25,
        "Rust blank set should be the 25 Unicode White_Space code points, got: {}",
        fmt(&from_rust)
    );
    assert!(
        from_rust.contains(&0x00A0) && from_rust.contains(&0x3000),
        "sweep must include the non-ASCII whitespace this test exists for"
    );

    let only_sql: Vec<u32> = from_sql
        .iter()
        .filter(|cp| !from_rust.contains(cp))
        .copied()
        .collect();
    let only_rust: Vec<u32> = from_rust
        .iter()
        .filter(|cp| !from_sql.contains(cp))
        .copied()
        .collect();

    assert!(
        only_sql.is_empty() && only_rust.is_empty(),
        "twin blank-test disagrees across the Rust/SQL boundary.\n  \
         blank in SQL only:  {}\n  blank in Rust only: {}",
        fmt(&only_sql),
        fmt(&only_rust)
    );
}

/// The zero-width look-alikes are the easy thing to get wrong in the other direction:
/// U+200B ZERO WIDTH SPACE and U+FEFF BOM have Unicode `White_Space=No`, so BOTH sides must
/// call them present. (Issue #75's own text mis-listed U+FEFF as Rust whitespace.)
#[tokio::test]
async fn zero_width_look_alikes_are_present_on_both_sides() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let client = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();

    for s in ["\u{200B}", "\u{FEFF}", "\u{00A0}BP 120/80"] {
        let in_sql: bool = client
            .query_one("SELECT cairn_twin_is_present($1)", &[&s])
            .await
            .unwrap()
            .get(0);
        assert!(in_sql, "SQL must call {s:?} present");
        assert_eq!(in_sql, twin_is_present(Some(s)), "boundary disagreement");
    }
}
