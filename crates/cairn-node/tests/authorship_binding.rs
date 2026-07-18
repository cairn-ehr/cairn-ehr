//! ADR-0053 authorship binding (db/005 `cairn_authorship_bound`). DB-gated on
//! $CAIRN_TEST_PG. The predicate is the floor's answer to forged authorship: a
//! responsibility-bearing contributor must be the event's signer or the verified
//! attester (the #195 binding, one field over). Contributory roles are exempt.
use cairn_node::db;
use tokio_postgres::Client;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// Evaluate the predicate with an explicit attester key — `None` is the pure-authorship
/// (no-token) path, `Some(bytes)` the verified-attester arm.
async fn bound_with(
    c: &Client,
    contributors: serde_json::Value,
    signer: &str,
    attester_key: Option<&[u8]>,
) -> bool {
    // tokio-postgres in this crate has no serde_json ToSql (no with-serde_json-1
    // feature): pass the body as a text string and cast with $1::text::jsonb — the
    // project convention (see matcher_actor.rs and tests/twin_registry.rs: a bare
    // $1::jsonb cast either fails to compile or silently false-greens).
    let b = serde_json::json!({"contributors": contributors}).to_string();
    c.query_one(
        "SELECT cairn_authorship_bound($1::text::jsonb, $2, $3)",
        &[&b, &signer, &attester_key],
    )
    .await
    .unwrap()
    .get::<_, bool>(0)
}

/// The common no-token case: p_attester_key NULL.
async fn bound(c: &Client, contributors: serde_json::Value, signer: &str) -> bool {
    bound_with(c, contributors, signer, None).await
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

/// The VERIFIED-ATTESTER arm: a bearing contributor who did NOT sign is still bound if
/// it names the human whose attestation token the door just verified. This is the arm
/// the deferred token-backed-author path (#242, verbal/telephone orders, AI-scribe)
/// authenticates through — a regression that dropped it would wrongly refuse every
/// lawful authored-but-not-signed shape, so it needs its own coverage.
#[tokio::test]
async fn authorship_binding_verified_attester_arm() {
    let Some(cs) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&cs).await.unwrap();
    let c = db::connect_and_load_schema(&cs).await.unwrap();

    // House rule 6: crypto-shaped test material is DERIVED at runtime, never a literal
    // (a byte-array literal here trips CodeQL's hard-coded-cryptographic-value query).
    let attester_key: [u8; 32] = std::array::from_fn(|i| (i as u8).wrapping_mul(7).wrapping_add(3));
    let attester_hex = hex::encode(attester_key);
    let node = "NODEKID";

    // Bearing author == the verified attester, signer is the node -> bound. The
    // attestation token is what authenticated them; they never touched the signature.
    assert!(
        bound_with(
            &c,
            serde_json::json!([{"actor_id": attester_hex, "role": "attested",
                                "responsibility": {"held_by": attester_hex}},
                               {"actor_id": node, "role": "recorded"}]),
            node,
            Some(&attester_key),
        )
        .await
    );

    // Bearing author is NEITHER the signer NOR the verified attester -> NOT bound.
    // A token being present must not launder a claim about some third party (#195).
    assert!(
        !bound_with(
            &c,
            serde_json::json!([{"actor_id": "STRANGER", "role": "authored"},
                               {"actor_id": node, "role": "recorded"}]),
            node,
            Some(&attester_key),
        )
        .await
    );

    // The signer arm still holds when a token is ALSO present: the two arms are OR'd,
    // so an author who signed is bound even though a different human vouched.
    assert!(
        bound_with(
            &c,
            serde_json::json!([{"actor_id": "AUTHORKID", "role": "authored"},
                               {"actor_id": node, "role": "recorded"}]),
            "AUTHORKID",
            Some(&attester_key),
        )
        .await
    );

    // EVERY bearing contributor must authenticate — one good + one forged is NOT bound.
    assert!(
        !bound_with(
            &c,
            serde_json::json!([{"actor_id": attester_hex, "role": "attested",
                                "responsibility": {"held_by": attester_hex}},
                               {"actor_id": "STRANGER", "role": "authored"}]),
            node,
            Some(&attester_key),
        )
        .await
    );
}
