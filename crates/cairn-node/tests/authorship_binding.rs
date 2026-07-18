//! ADR-0053 authorship binding (db/005 `cairn_authorship_bound`). DB-gated on
//! $CAIRN_TEST_PG. The predicate is the floor's answer to forged authorship: a
//! responsibility-bearing contributor must be the event's signer or the verified
//! attester (the #195 binding, one field over). Contributory roles are exempt.
use cairn_node::db;
use tokio_postgres::Client;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

async fn bound(c: &Client, contributors: serde_json::Value, signer: &str) -> bool {
    // tokio-postgres in this crate has no serde_json ToSql (no with-serde_json-1
    // feature): pass the body as a text string and cast with $1::text::jsonb — the
    // project convention (see matcher_actor.rs and tests/twin_registry.rs: a bare
    // $1::jsonb cast either fails to compile or silently false-greens).
    let b = serde_json::json!({"contributors": contributors}).to_string();
    // p_attester_key NULL: the pure-authorship (no-token) path.
    c.query_one(
        "SELECT cairn_authorship_bound($1::text::jsonb, $2, NULL)",
        &[&b, &signer],
    )
    .await
    .unwrap()
    .get::<_, bool>(0)
}

#[tokio::test]
async fn authorship_binding_predicate() {
    let Some(cs) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&cs).await.unwrap();
    // connect_and_load_schema (not plain connect): the project convention for every
    // DB-gated test (see e.g. tests/contributor_roles.rs) — schema is CREATE OR
    // REPLACE'd idempotently on every load, and this is how a reconnect is guaranteed
    // to see a just-added db/005 function rather than depending on some other test
    // binary having happened to load it first into the shared, persistent test DB.
    let c = db::connect_and_load_schema(&cs).await.unwrap();

    // bearing author == signer -> bound.
    assert!(
        bound(
            &c,
            serde_json::json!([{"actor_id": "H", "role": "authored"},
                                         {"actor_id": "N", "role": "recorded"}]),
            "H"
        )
        .await
    );
    // bearing author != signer, no token -> NOT bound (forged authorship).
    assert!(
        !bound(
            &c,
            serde_json::json!([{"actor_id": "H", "role": "authored"},
                                          {"actor_id": "N", "role": "recorded"}]),
            "N"
        )
        .await
    );
    // contributory-only (recorded) -> bound (device path exempt).
    assert!(
        bound(
            &c,
            serde_json::json!([{"actor_id": "N", "role": "recorded"}]),
            "N"
        )
        .await
    );
}
