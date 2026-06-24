//! Integration: a SEALED node provisions, both recipients unseal the key, and
//! `status` reports the sealed posture + recovery escrow (ADR-0026 slice A).
//! DB-gated like the rest of the node suite; self-serializes via the advisory lock.
use cairn_node::{db, identity, keystore, seal};

fn cs() -> Option<String> { std::env::var("CAIRN_TEST_PG").ok() }

#[tokio::test]
async fn sealed_init_produces_dual_recipient_key_and_surfaces_escrow() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let db = db::connect_and_load_schema(&base).await.unwrap();
    db.batch_execute("TRUNCATE node_event, local_node").await.ok();

    let tmp = tempfile::tempdir().unwrap();
    let kp = tmp.path().join("node.key");

    // Provision SEALED (the production path).
    let op = "correct horse battery staple";
    let code = seal::generate_recovery_code();
    let (sk, kid) = keystore::generate_sealed(&kp, op, &code).unwrap();
    identity::provision(&db, &sk, &kid, "A", "127.0.0.1:7900").await.unwrap();

    // Both recipients recover the same key; no/ wrong secret fails legibly (no panic).
    // The substantive dual-recipient claim is that BOTH secrets decrypt to the SAME
    // key, so cross-check the two recipients against each other as well as the write side.
    let sk_op  = keystore::load(&kp, Some(op)).unwrap();
    let sk_rec = keystore::load(&kp, Some(&code)).unwrap();
    assert_eq!(sk_op.to_bytes(), sk_rec.to_bytes(),
        "both recipients must decrypt the same key");
    assert_eq!(sk_op.to_bytes(), sk.to_bytes(),
        "recipient-decrypted key must equal the provisioned key");
    assert!(keystore::load(&kp, None).is_err());
    assert!(keystore::load(&kp, Some("wrong")).is_err());

    // status reflects the sealed posture + escrow.
    let st = identity::status(&db, &kp).await.unwrap();
    assert!(st.initialized, "provisioned node must report initialized=true");
    assert!(st.keystore_ok);
    assert!(st.key_at_rest.contains("SEALED"), "got {:?}", st.key_at_rest);
    assert!(st.recovery_escrow, "sealed key must report an escrow");
    assert!(st.dr_escrow.contains("recovery code set"), "got {:?}", st.dr_escrow);
}
