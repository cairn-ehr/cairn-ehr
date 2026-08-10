//! The §5.9 sensitivity ladder (ADR-0062).
//!
//! The one thing to understand before editing: an UNRECOGNISED grade ranks MAX here,
//! which is the exact opposite of `cairn_clock_grade_rank`'s `ELSE 0`. See the comment
//! on `cairn_sensitivity_rank` in db/048 — a "fix" that aligns the two is a leak.
mod common;
use common::{cs, db_msg};

#[tokio::test]
async fn the_ladder_orders_the_named_grades_and_ranks_the_unknown_maximum() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();

    let rank = |g: &'static str| {
        let c = &c;
        async move {
            c.query_one("SELECT cairn_sensitivity_rank($1)", &[&g])
                .await
                .map(|r| r.get::<_, i32>(0))
                .map_err(|e| db_msg(&e))
                .unwrap()
        }
    };

    assert_eq!(rank("routine").await, 0, "no protection asserted");
    assert!(rank("sensitive").await < rank("restricted").await);
    assert!(rank("restricted").await < rank("sequestered").await);

    // The inverted unknown. A future peer's grade must coarsen, never expose.
    assert_eq!(
        rank("grade:protected-witness").await,
        i32::MAX,
        "an unrecognised grade must rank MAX: ranking it 0 would let an older node read a \
         peer's newer grade as 'not sensitive' and render the body in the clear"
    );

    // NULL lands on the same safe side (a NOT NULL column makes this unreachable, but the
    // function is public API and must not have an unsafe corner).
    let null_rank: i32 = c
        .query_one("SELECT cairn_sensitivity_rank(NULL)", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(null_rank, i32::MAX);
}
