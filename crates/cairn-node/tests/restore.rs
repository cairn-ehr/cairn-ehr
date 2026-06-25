//! ADR-0026 slice C — restore (apply) + new-identity supersede.
//! DB-gated: needs CAIRN_TEST_PG (local PG with cairn_pgx installed).

use cairn_node::{db, identity, keystore};

fn cs() -> Option<String> { std::env::var("CAIRN_TEST_PG").ok() }

/// A live node can author a node.superseded event naming a dead node-id; it lands as an
/// op='supersede' row and node_lineage resolves the edge (new <- old).
#[tokio::test]
async fn author_supersede_records_a_lineage_edge() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let a = db::connect_and_load_schema(&base).await.unwrap();
    db::reset_node_federation_tables(&a).await.ok();

    let tmp = tempfile::tempdir().unwrap();
    let (sk, kid) = keystore::generate_plaintext(&tmp.path().join("a.key")).unwrap();
    identity::provision(&a, &sk, &kid, "A", "127.0.0.1:7920").await.unwrap();
    let id = identity::load_local(&a).await.unwrap();

    // Author supersede naming a fabricated "old" node-id (any hex node-id works here;
    // the full restore flow supplies the real one in Task 5).
    let old_hex = "1220".to_string() + &"ab".repeat(32);
    identity::author_supersede(&a, &sk, &kid, &id.node_id_hex, &old_hex).await.unwrap();

    let row = a.query_one(
        "SELECT encode(superseded_node_id,'hex') AS old, encode(new_node_id,'hex') AS new
         FROM node_lineage", &[]).await.unwrap();
    let old: String = row.get("old");
    let new: String = row.get("new");
    assert_eq!(old, old_hex, "lineage subject == the dead node-id");
    assert_eq!(new, id.node_id_hex, "lineage author == the live (new) node-id");
}
