//! The in-DB floor, ENFORCED — the half of ADR-0017/§9.6 the existing `status`
//! test could not exercise. That test connects as a superuser and asserts the
//! floor is *present but BYPASSABLE* (a superuser can raw-INSERT around the gate).
//!
//! This proves the other half: when the runtime connects as the UNPRIVILEGED
//! `cairn_node`-granted login role (the deployment the HANDOVER calls for), the
//! floor actually BINDS — a raw `INSERT` into `node_event` is denied, yet the
//! validated `submit_node_event` door still works. The guarantee is "enforced in
//! Postgres" only for such a connection, so it must be tested over one.

use cairn_node::{db, identity, keystore};
use tokio_postgres::Client;

fn cs() -> Option<String> { std::env::var("CAIRN_TEST_PG").ok() }

/// Rewrite a `host=… dbname=…` conn string to connect as a different role. The
/// test base string sets `user=…`; we replace it so the same DB is reached as the
/// unprivileged runtime role.
fn conn_as_role(base: &str, role: &str) -> String {
    let kept: Vec<&str> = base
        .split_whitespace()
        .filter(|kv| !kv.starts_with("user="))
        .collect();
    format!("{} user={role}", kept.join(" "))
}

#[tokio::test]
async fn floor_binds_the_unprivileged_runtime_role() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();

    // Owner/superuser connection: load schema and provision the node (DDL needs it).
    let owner = db::connect_and_load_schema(&base).await.unwrap();
    owner.batch_execute("TRUNCATE node_event, local_node").await.ok();

    let tmp = tempfile::tempdir().unwrap();
    let key_path = tmp.path().join("node.key");
    let (sk, kid) = keystore::generate_and_seal(&key_path, None).unwrap();
    identity::provision(&owner, &sk, &kid, "A", "127.0.0.1:7950").await.unwrap();
    let id = identity::load_local(&owner).await.unwrap();

    // Provision the unprivileged runtime login role (the thing under test).
    let role = "cairn_runtime_test";
    db::provision_runtime_role(&owner, role).await.unwrap();

    // Connect AS that role.
    let runtime: Client = db::connect(&conn_as_role(&base, role)).await.unwrap();

    // 1) status over the runtime connection reports the floor ENFORCED.
    let st = identity::status(&runtime, &key_path).await.unwrap();
    assert_eq!(st.runtime_role, role, "status must report the connected role");
    assert!(
        st.db_floor_enforced,
        "the cairn_node-granted login role must report db_floor ENFORCED, got role {:?}",
        st.runtime_role
    );

    // 2) A raw INSERT into node_event is DENIED for this role (the hard floor).
    let raw = runtime
        .execute(
            "INSERT INTO node_event (node_event_id, op, author_node_id, subject_node_id, \
             signer_key_id, hlc_wall, hlc_counter, node_origin, signed_bytes, content_address) \
             VALUES (gen_random_uuid(), 'enroll', '\\x00', '\\x00', 'x', 0, 0, 'x', '\\x00', '\\x00')",
            &[],
        )
        .await;
    let err = raw.expect_err("raw INSERT into node_event must be denied for the runtime role");
    // The detail is in the DbError, not the outer Error's Display; assert on the
    // SQLSTATE so the check is robust to message wording (42501 = insufficient_privilege).
    assert_eq!(
        err.code(),
        Some(&tokio_postgres::error::SqlState::INSUFFICIENT_PRIVILEGE),
        "raw INSERT must fail with insufficient_privilege (42501), got: {err:?}"
    );

    // 3) The validated door STILL works for the unprivileged role: authoring a peer
    //    through submit_node_event succeeds and shows up in trust_peer.
    let (_sk_b, kid_b) = cairn_event::generate_key().unwrap();
    let b_node_id = hex::encode(cairn_event::event_address(b"B-genesis-floor-test"));
    let bundle = cairn_event::PairingBundle {
        node_id_hex: b_node_id.clone(),
        pubkey_hex: kid_b.clone(),
        address: "127.0.0.1:7951".into(),
        fingerprint: cairn_event::short_fingerprint(&kid_b).unwrap(),
        nonce: "n".into(),
        hlc: cairn_event::Hlc { wall: 0, counter: 0, node_origin: b_node_id.clone() },
    };
    identity::author_peer(&runtime, &sk, &kid, &id.node_id_hex, &bundle, Some("peer"))
        .await
        .expect("submit_node_event must work for the cairn_node-granted role");
    let peers = identity::list_peers(&runtime).await.unwrap();
    assert_eq!(peers.len(), 1, "the peer authored via the door must be visible");
    assert_eq!(peers[0].status, "active");
}

/// The role-name charset gate must reject anything that could break out of the
/// interpolated DDL — this is the SQL-injection floor for `provision_runtime_role`.
/// (No DB needed: the validation happens before any query.)
#[tokio::test]
async fn provision_runtime_role_rejects_unsafe_names() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let owner = db::connect(&base).await.unwrap();
    for bad in [
        "cairn; DROP ROLE postgres",
        "role with spaces",
        "Mixed_Case",      // we constrain to lowercase to keep the charset tight
        "1leading_digit",
        "has-hyphen",
        "",
    ] {
        let res = db::provision_runtime_role(&owner, bad).await;
        assert!(res.is_err(), "unsafe role name {bad:?} must be rejected");
    }
}
