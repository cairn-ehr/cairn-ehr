//! The one enrolment rule (#654): a device actor is PROVISIONED, never minted on a write path.
//!
//! Before this, `cairn-node`'s fifteen write subcommands each enrolled the node's signing key as
//! a `device` actor on first use, while `cairn-gui-live` deliberately did not — so a node's
//! behaviour depended on which surface touched it first, and the reference window's first
//! registration refused with a message naming a key rather than a remedy.
//!
//! Two of these tests need no database at all, and that is deliberate: the refusal's *words* are
//! the part an operator actually meets, and they must not be gated behind a rig.

mod common;

use cairn_node::actor_enrolment::{
    device_actor_enrolled, enroll_device_actor, not_enrolled_refusal, require_device_actor,
};
use cairn_node::db;
use cairn_node::db_diagnosis::is_deliberate_refusal;
use common::{cs, setup};

/// A distinct, deterministic key id per test, derived rather than written.
///
/// Derived because a byte-array literal in a crypto-adjacent context trips CodeQL's
/// `rust/hard-coded-cryptographic-value`; the parameter is named `lineage` rather than
/// `salt`/`nonce`/`iv` because CodeQL picks its sink by the NAME of the binding a value flows
/// into, and this discriminates test rows — it constructs nothing cryptographic whatsoever.
/// See `CLAUDE.md` rule 6 and `crates/cairn-node/tests/crypto_sink_names_are_genuine.rs`.
fn a_key_id(lineage: &str) -> String {
    let marks = lineage.as_bytes();
    let bytes: [u8; 32] = std::array::from_fn(|i| {
        (i as u8)
            .wrapping_mul(7)
            .wrapping_add(marks[i % marks.len()])
    });
    hex::encode(bytes)
}

/// How many `actor_current` rows this key maps to. **ONE is the only safe answer** — see
/// `device_actor_enrolled`'s doc for what two does to attribution.
async fn rows_for(c: &tokio_postgres::Client, kid: &str) -> i64 {
    c.query_one(
        "SELECT count(*) FROM actor_current WHERE signing_key_id = $1",
        &[&kid],
    )
    .await
    .expect("count the actors this key maps to")
    .get(0)
}

/// The refusal names the COMMAND, not just the key.
///
/// Pure, so it runs with no database. This is the sentence's whole job: `submit_event`'s own
/// refusal — *"signer 9f3c… is not an enrolled, non-revoked actor"* — is true, legible, and
/// tells nobody what to do about it. The precedent is `submit_event`'s unwrap-key refusal,
/// which names `establish-unwrap-key`.
#[test]
fn the_refusal_names_the_command_that_fixes_it() {
    let e = not_enrolled_refusal("9f3cdeadbeef");
    let rendered = format!("{e:#}");
    assert!(
        rendered.contains("enroll-device-actor"),
        "a refusal that does not name its remedy leaves the operator exactly where the floor's \
         own message left them — got: {rendered}"
    );
    assert!(
        rendered.contains("9f3cdeadbeef"),
        "and it must still name the key, so an operator with several can tell which — got: \
         {rendered}"
    );
}

/// It is a VERDICT, so a window can classify it without reaching into the database.
///
/// The same discriminator #651 added. A pre-flight refusal that reached a clerk as an outage
/// would offer a retry that can never work — and this one can only ever be fixed by an operator
/// running a command, so a retry button on it is the worst possible advice.
#[test]
fn the_refusal_is_a_deliberate_verdict_not_an_accident() {
    assert!(is_deliberate_refusal(&not_enrolled_refusal("9f3cdeadbeef")));
}

/// On a node nobody provisioned, the requirement REFUSES rather than provisioning.
#[tokio::test]
async fn an_unprovisioned_node_refuses_rather_than_enrolling_on_the_write_path() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    // `setup` truncates `actor_event`, so no predecessor run's actor is still enrolled (#583's
    // shape). Its own returned key is irrelevant here — this test is about a key nothing knows.
    let _ = setup(&c, &[]).await;
    let kid = a_key_id("unprovisioned");

    assert!(!device_actor_enrolled(&c, &kid).await.unwrap());
    let e = require_device_actor(&c, &kid)
        .await
        .expect_err("a write path must never provision");
    assert!(
        is_deliberate_refusal(&e),
        "the refusal must be a verdict, or the window offers a retry for it"
    );
    assert!(
        !device_actor_enrolled(&c, &kid).await.unwrap(),
        "REFUSING MUST NOT ENROL. A check with a side effect is precisely what #654 is about — \
         it is trap 2's shape, one subsystem over"
    );
}

/// Enrolling is idempotent, and says whether it did anything.
#[tokio::test]
async fn enrolling_twice_enrols_once_and_reports_it() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let _ = setup(&c, &[]).await;
    let kid = a_key_id("idempotent");

    assert!(
        enroll_device_actor(&c, &kid).await.unwrap(),
        "the first call enrols"
    );
    assert!(
        !enroll_device_actor(&c, &kid).await.unwrap(),
        "the second finds it already there and says so"
    );
    assert_eq!(rows_for(&c, &kid).await, 1);
    require_device_actor(&c, &kid)
        .await
        .expect("an enrolled key passes the requirement");
}

/// ⚠️ THE PROPERTY THE WHOLE DESIGN TURNS ON: the existence check is KIND-AGNOSTIC.
///
/// `submit_event` resolves a signer to an actor purely by `signing_key_id`. If one key maps to
/// MORE than one `actor_current` row, db/005 sets `actor_id = NULL` for EVERY event that key
/// authors node-wide (`array_length(v_actor_ids, 1) = 1`) — silently and irreversibly degrading
/// attribution. So a key already enrolled as something that is not a `device` must be left
/// ALONE, not given a second row.
///
/// A `kind = 'device'`-scoped check would pass every other test in this file and cause exactly
/// that. `common::setup` enrols an `agent`, which is what a matcher is, so its key is a real
/// second-kind fixture rather than a hand-composed one.
#[tokio::test]
async fn a_key_already_enrolled_under_another_kind_is_left_alone() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (_sk, kid) = setup(&c, &[]).await;

    assert!(
        device_actor_enrolled(&c, &kid).await.unwrap(),
        "already-authoring means already enrolled, whatever kind it wears"
    );
    assert!(
        !enroll_device_actor(&c, &kid).await.unwrap(),
        "and enrolling must report that it did nothing"
    );
    assert_eq!(
        rows_for(&c, &kid).await,
        1,
        "a SECOND actor for one key nulls the actor_id of every event that key ever authors"
    );
}
