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
    device_actor_enrolled, device_actor_standing, enroll_device_actor, not_enrolled_refusal,
    require_device_actor, ActorStanding,
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

/// Revoke every actor this key is enrolled under, the way the registry's own suites do.
///
/// A `revoke` row carries a NULL `signing_key_id` by design (db/004), so it is written against
/// the `actor_id` — which is why "has this key any history?" cannot be answered by looking at
/// revoke rows alone.
async fn revoke_every_actor_for(c: &tokio_postgres::Client, kid: &str) {
    c.execute(
        "INSERT INTO actor_event (actor_id, op) \
         SELECT DISTINCT actor_id, 'revoke' FROM actor_event WHERE signing_key_id = $1",
        &[&kid],
    )
    .await
    .expect("revoke the actors this key is enrolled under");
}

/// ⚠️ A REVOKED ACTOR MUST NOT BE SENT TO A COMMAND THAT CANNOT HELP IT.
///
/// `actor_current` excludes revoked actors, so a revoked key reads as *not enrolled* — and the
/// obvious refusal would tell the operator to run `cairn-node enroll-device-actor`. That command
/// **cannot work**: db/004's `cairn_actor_id_key_conflict` refuses a fresh enroll onto an
/// `actor_id` with prior revoke/supersede history, deliberately, because a post-revoke enroll
/// would outrank the revoke in `actor_current`'s order and silently **resurrect a retired actor**
/// (#152).
///
/// Refusing the resurrection is correct. Sending the operator there is not: they would meet an
/// opaque `P0001` about actor-id collisions while trying to follow the remedy the previous
/// message gave them. The two states are therefore told apart, and the retired one says what
/// actually happened.
///
/// Found by the PR #661 review.
#[tokio::test]
async fn a_revoked_actor_is_not_told_to_re_enrol_a_key_that_cannot_be_resurrected() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let _ = setup(&c, &[]).await;
    let kid = a_key_id("revoked");

    assert!(enroll_device_actor(&c, &kid).await.unwrap());
    revoke_every_actor_for(&c, &kid).await;

    assert_eq!(
        device_actor_standing(&c, &kid).await.unwrap(),
        ActorStanding::Retired,
        "a revoked key is NOT the same state as a key nobody ever enrolled"
    );

    let e = require_device_actor(&c, &kid)
        .await
        .expect_err("a revoked key may not author");
    let rendered = format!("{e:#}");
    assert!(
        rendered.contains("revoked") || rendered.contains("retired"),
        "it must say what actually happened, not merely that the key is not enrolled; got: \
         {rendered}"
    );
    // It DOES name `enroll-device-actor`, and must — the operator may have just been sent there
    // by the never-enrolled refusal, so naming it in order to withdraw it is more use than
    // silence. What it must not do is PRESCRIBE it: db/004 refuses that enroll as a
    // resurrection (#152) with an opaque P0001, and the operator would meet it while following
    // our own advice.
    assert!(
        rendered.contains("will NOT help"),
        "the message must withdraw the other refusal's remedy in so many words, or an operator \
         who read both is left to guess which one applies; got: {rendered}"
    );
    assert!(
        rendered.contains("NEW signing key"),
        "and it must name the remedy that DOES exist; got: {rendered}"
    );

    // And the command itself refuses in OUR words rather than letting db/004 raise its
    // actor-id-collision message at somebody who only did what they were told.
    let e = enroll_device_actor(&c, &kid)
        .await
        .expect_err("re-enrolling a retired key must not reach db/004's resurrection guard");
    assert!(
        is_deliberate_refusal(&e),
        "it is a verdict — the same key is refused identically forever"
    );
}

/// ⚠️ A SOURCE GUARD, BECAUSE `init` HAS NO BEHAVIOURAL TEST AND DELETING ONE LINE IS SILENT.
///
/// `Cmd::Init` calls `enroll_device_actor`, and that call is what keeps the paper-parity count
/// at `M = 0` for an ordinary operator (#654): an initialised node can author immediately, so no
/// write path has to provision to make that true. **Delete that line and the entire workspace
/// gate still passes**, while every freshly-initialised node silently loses its ability to author
/// until somebody runs `enroll-device-actor` — which they have no reason to suspect.
///
/// A behavioural test would need a VIRGIN database: `init` refuses over a registered custody key
/// (`refuse_init_over_a_registered_custody_key`) and mints a signing key, an unwrap key and a
/// local-state escrow, so it cannot run against the shared `cairn_test` fixture every other suite
/// uses. Creating a scratch database per run is a rig of its own — filed as
/// [#662](https://github.com/cairn-ehr/cairn-ehr/issues/662) rather than faked, with the shape a
/// real `cli_init.rs` would take and the seven other `init` effects it would also cover.
///
/// **What this guard does NOT catch**, stated so nobody mistakes it for the real thing: that the
/// call is reached (it could sit behind a condition), that it is passed the right key, or that
/// `enroll_device_actor` does what it says. It catches deletion, which is the failure the PR #661
/// review actually named.
///
/// Found by the PR #661 review.
#[test]
fn init_still_enrols_the_device_actor() {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/main.rs"))
        .expect("main.rs must be readable from the crate it belongs to");
    let arm = src
        .split_once("Cmd::Init {")
        .expect("the Init arm must still exist")
        .1;
    // Bounded to this arm rather than the whole file: `enroll_device_actor` is also called by
    // `Cmd::EnrollDeviceActor`, so an unbounded search would stay green with the `init` call
    // gone — which is precisely the deletion this guard exists to catch.
    let arm = &arm[..arm
        .find("\n        Cmd::")
        .expect("another subcommand must follow Init")];
    assert!(
        arm.contains("actor_enrolment::enroll_device_actor"),
        "`cairn-node init` no longer enrols the node's device actor. If that removal was \
         deliberate, note that it makes an ordinary operator run one extra command before their \
         node can author anything — `M = 0` becomes `M = 1` in the §1.2 benchmark of #654 — and \
         update that benchmark rather than deleting this guard. Init arm read:\n{arm}"
    );
}
