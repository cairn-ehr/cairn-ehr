//! #421 — the withdrawal worklist must NAME the accountable actor.
//!
//! The view's `judged` CTE has always computed `responsible_actor_id` (the vouched R1
//! attester when exactly one human maps to the attester key, else the withdrawal's own
//! actor); the outer SELECT dropped it, so every consumer could report THAT a withdrawal
//! was ineffective but not WHO authored it. This guard fails if the column is ever dropped
//! again — a silent regression would leave the operator surface printing an empty field
//! rather than erroring, which is the failure mode this whole slice exists to end.
mod common;
use common::cs;

#[tokio::test]
async fn the_worklist_projects_the_accountable_actor() {
    let Some(base) = cs() else {
        return; // self-skips when CAIRN_TEST_PG is unset, like every suite here
    };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();

    // Ask the catalogue, not a row: a row-shaped assertion could be satisfied by accident
    // by a projection default, and this is a CONTRACT about the view's shape.
    let rows = c
        .query(
            "SELECT data_type FROM information_schema.columns
              WHERE table_schema = 'public'
                AND table_name = 'sensitivity_withdrawal_worklist'
                AND column_name = 'responsible_actor_id'",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(
        rows.len(),
        1,
        "sensitivity_withdrawal_worklist must project responsible_actor_id (#421)"
    );
    let ty: String = rows[0].get(0);
    assert_eq!(
        ty, "bytea",
        "actor_id is BYTEA in db/001 and db/004; the view must not retype it"
    );
}
