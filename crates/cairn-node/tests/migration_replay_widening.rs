//! Issue #207 (2026-07-15 review, finding D2) — widened CREATE TABLE IF NOT EXISTS
//! must be accompanied by an additive ALTER.
//!
//! The schema loader re-runs every db/*.sql on every connect, and `CREATE TABLE IF NOT
//! EXISTS` NO-OPS on a table that already exists. So a column added by editing the CREATE
//! body (the #115 `content_address` widenings) never reaches a database created before the
//! edit — and every trigger INSERT naming the column then fails at trigger depth inside
//! the write door: a total write outage for those event types (ADR-0012 additive-migration
//! rule violated). The correct pattern is the one db/001 already uses: an idempotent
//! `ALTER TABLE … ADD COLUMN IF NOT EXISTS` after the CREATE.
//!
//! This suite simulates exactly that upgraded-in-place database: load the schema, DROP the
//! widened column (recreating the pre-#115 table shape), reconnect (replaying every
//! migration), and assert the column is back. With only the widened CREATE, it is not.
//!
//! Real Postgres, gated on `$CAIRN_TEST_PG`, serialized cluster-wide via
//! `db::test_serial_guard`.
use cairn_node::db;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// The five #115 widenings (table, column) — extend this list when a future slice widens
/// an existing CREATE TABLE IF NOT EXISTS (and add the matching ALTER in its migration).
const WIDENED: &[(&str, &str)] = &[
    ("patient_chart", "demo_content_address"),
    ("patient_link", "content_address"),
    ("chart_dispute", "content_address"),
    ("chart_identity_state", "content_address"),
    ("name_repudiation", "content_address"),
    // #194: the two set-union demographic projections gained the same tiebreak column.
    ("patient_identifier", "content_address"),
    ("patient_demographic", "content_address"),
];

#[tokio::test]
async fn replayed_schema_restores_widened_columns_on_a_pre_widening_table() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();

    // Load once, then knock each widened column off — this is the shape a database
    // created before the widening commit is in when the new binary connects to it.
    let c = db::connect_and_load_schema(&base).await.unwrap();
    for (table, column) in WIDENED {
        // CASCADE: dependent views/indexes drop with the column; the replay below
        // recreates them (CREATE OR REPLACE VIEW / CREATE INDEX IF NOT EXISTS).
        c.batch_execute(&format!(
            "ALTER TABLE {table} DROP COLUMN IF EXISTS {column} CASCADE"
        ))
        .await
        .unwrap();
    }
    drop(c);

    // Reconnect = replay every migration, exactly what a node does on startup.
    let c = db::connect_and_load_schema(&base).await.unwrap();
    for (table, column) in WIDENED {
        let present: bool = c
            .query_one(
                "SELECT EXISTS (SELECT 1 FROM information_schema.columns
                  WHERE table_schema = 'public' AND table_name = $1 AND column_name = $2)",
                &[table, column],
            )
            .await
            .unwrap()
            .get(0);
        assert!(
            present,
            "{table}.{column} must be restored by schema replay on a pre-widening table \
             (additive ALTER missing — issue #207: write outage at trigger depth)"
        );
    }
}
