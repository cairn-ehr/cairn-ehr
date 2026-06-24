// Gated on a live Postgres (set CAIRN_TEST_PG). Loads the schema into a fresh
// throwaway database, provisions a node, and asserts the genesis identity lands.
use cairn_node::{db, identity, keystore};

fn conn_str() -> Option<String> { std::env::var("CAIRN_TEST_PG").ok() }

#[tokio::test]
async fn provision_writes_genesis_identity() {
    let Some(cs) = conn_str() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&cs).await.unwrap(); // serialize shared-DB tests
    let client = db::connect_and_load_schema(&cs).await.unwrap();

    // Reset state so the test is re-runnable. The append-only trigger blocks DELETE
    // but NOT TRUNCATE; we connect as superuser so this is safe.
    client.batch_execute("TRUNCATE node_event, local_node").await.ok();

    let tmp = tempfile::tempdir().unwrap();
    let keypath = tmp.path().join("node.key");
    let (sk, kid) = keystore::generate_plaintext(&keypath).unwrap();

    let node_id = identity::provision(&client, &sk, &kid, "Clinic-A", "127.0.0.1:7800").await.unwrap();
    let loaded = identity::load_local(&client).await.unwrap();

    assert_eq!(loaded.node_id_hex, node_id);
    assert_eq!(loaded.pubkey_hex, kid);
    assert_eq!(loaded.fingerprint, cairn_event::short_fingerprint(&kid).unwrap());

    // Genesis is once-only: a second provision must error.
    assert!(identity::provision(&client, &sk, &kid, "Clinic-A", "127.0.0.1:7800").await.is_err());
}
