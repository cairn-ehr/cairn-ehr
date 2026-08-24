//! ADR-0066 decision 6 — the node's custody key is PROVISIONED, and the write path checks.
//!
//! WHY THIS SUITE EXISTS (for a junior reader). Every clinical body Cairn writes is sealed
//! under a fresh per-event DEK, and that DEK is wrapped to this NODE's X25519 unwrap key so
//! the ADR-0005 erasure ladder stays reachable forever. Before ADR-0066 the write path
//! *registered* that key on every sealed write, deriving it from whichever signing key
//! happened to be in hand. Two things were wrong with that:
//!
//! 1. It made a node's custody key an implicit consequence of who signed first, when a
//!    custody key is a **provisioned fact about the node**.
//! 2. It tied custody to identity — and disaster recovery deliberately mints a FRESH
//!    signing seed, so a restored node's derived unwrap secret changed and every inherited
//!    wrapped-DEK row went permanently dark (#495/#500).
//!
//! So registration moved to `cairn-node init` / `cairn-node establish-unwrap-key`, and the
//! write path now VERIFIES. This suite pins both halves of that verification: it refuses
//! when nothing is registered (and names the remedy, because a refusal an operator cannot
//! act on is not a safety control), and it admits when a key IS registered — the positive
//! control, without which a permanently-refusing implementation would still look green.

mod common;

use cairn_node::db;

/// Clear the custody-key singleton, putting the database in the exact state a freshly
/// created one is in before `init`/`establish-unwrap-key` has run.
///
/// `node_unwrap_key` has no foreign key to `event_log`, so the CASCADE the other suites
/// use never reaches it — a key left behind by whichever suite ran before us would make
/// this test pass for the wrong reason. Deleting explicitly is what makes the "no key
/// registered" precondition real rather than assumed.
async fn clear_registered_unwrap_key(c: &tokio_postgres::Client) {
    c.execute("DELETE FROM node_unwrap_key", &[])
        .await
        .expect("clearing the unwrap-key singleton");
}

#[tokio::test]
async fn a_sealed_write_fails_loudly_when_no_unwrap_key_is_registered() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    // The advisory-lock guard is taken BEFORE the schema load: every DB-gated suite
    // truncates shared tables on entry, so overlapping runs race.
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    clear_registered_unwrap_key(&c).await;

    let err = cairn_node::medication::sealed_submit::ensure_unwrap_key(&c)
        .await
        .expect_err("an unprovisioned node must refuse, not silently write without custody");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("establish-unwrap-key"),
        "the refusal must name the remedy the operator can actually run; got: {msg}"
    );
}

#[tokio::test]
async fn the_check_admits_a_node_whose_unwrap_key_is_registered() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    clear_registered_unwrap_key(&c).await;
    // House rule 6: the key material is COMPUTED at runtime by the same generator
    // provisioning uses, never written as a literal.
    let secret = cairn_event::seal::generate_unwrap_secret().expect("mint an unwrap secret");
    let public = cairn_event::seal::unwrap_public(&secret);
    c.execute(
        "SELECT cairn_register_unwrap_key($1)",
        &[&public.as_slice()],
    )
    .await
    .expect("registering a freshly minted unwrap key");

    cairn_node::medication::sealed_submit::ensure_unwrap_key(&c)
        .await
        .expect("a provisioned node must be admitted — otherwise the refusal above is vacuous");
}
