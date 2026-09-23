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
    device_actor_standing, enroll_device_actor, not_enrolled_refusal, require_device_actor,
    ActorStanding,
};
use cairn_node::db;
use cairn_node::db_diagnosis::carries_refusal_marker;
use common::{cs, setup};

/// Strip a line comment, so a call that has been commented OUT does not read as a live one.
///
/// The same helper `enrolment_is_never_a_write_side_effect.rs` carries, for the mirror reason —
/// see `init_still_enrols_the_device_actor`.
fn strip_comment(line: &str) -> &str {
    match line.find("//") {
        Some(i) => &line[..i],
        None => line,
    }
}

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
/// `ActorStanding::Ambiguous`'s doc for what two does to attribution.
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
    let kid = a_key_id("refusal");
    let e = not_enrolled_refusal(&kid);
    let rendered = format!("{e:#}");
    assert!(
        rendered.contains("enroll-device-actor"),
        "a refusal that does not name its remedy leaves the operator exactly where the floor's \
         own message left them — got: {rendered}"
    );
    assert!(
        rendered.contains(&kid),
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
    assert!(carries_refusal_marker(&not_enrolled_refusal(&a_key_id(
        "verdict"
    ))));
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

    assert_eq!(
        device_actor_standing(&c, &kid).await.unwrap(),
        ActorStanding::NeverEnrolled
    );
    let e = require_device_actor(&c, &kid)
        .await
        .expect_err("a write path must never provision");
    assert!(
        carries_refusal_marker(&e),
        "the refusal must be a verdict, or the window offers a retry for it"
    );
    assert!(
        device_actor_standing(&c, &kid).await.unwrap() == ActorStanding::NeverEnrolled,
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
        device_actor_standing(&c, &kid).await.unwrap() == ActorStanding::Enrolled,
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

/// Stage a key that maps to TWO current actors, the way a non-adjudicating door would.
///
/// `enroll_actor` refuses this outright since #166, and correctly — so the only way to reach
/// the state is the way the real doors that produce it do: a direct `actor_event` INSERT.
/// db/052's `restore_actor_registry` is the shipped one (it replays a medium's rows and
/// deliberately bypasses db/004's collision guards), and ADR-0044 §3's future actor-sync apply
/// door is the anticipated one. `actor_id` is computed exactly as those doors compute it,
/// `cairn_actor_id(pinned)`, so the two rows are two genuinely distinct actors rather than a
/// duplicate of one — the same staging `recall_epoch.rs` uses.
async fn bind_key_to_a_second_actor(c: &tokio_postgres::Client, kid: &str, variant: &str) {
    let pinned = format!("{{\"node_key\":\"{kid}\",\"variant\":\"{variant}\"}}");
    c.execute(
        "INSERT INTO actor_event (actor_id, op, kind, pinned, signing_key_id) \
         VALUES (cairn_actor_id($1::text::jsonb), 'enroll', 'device', $1::text::jsonb, $2)",
        &[&pinned, &kid],
    )
    .await
    .expect("stage a dual-mapped key the way a non-adjudicating door would");
}

/// ⚠️ A KEY MAPPING TO TWO ACTORS MUST REFUSE, NOT WRITE AN EVENT NOBODY CAN BE HELD TO.
///
/// This is the worst outcome in the module and the only one that is silent: `submit_event`
/// resolves a signer by `signing_key_id` alone and sets `actor_id = NULL` when that resolves to
/// more than one row (db/005, `array_length(v_actor_ids, 1) = 1`). Not for one event — for
/// **every event that key ever authors, node-wide, irreversibly**. An event whose author cannot
/// be named is a permanent hole in the accountability record (principle 10), and unlike a
/// refusal it puts nothing on screen.
///
/// **The mutation this kills, which is not hypothetical.** Rewrite `device_actor_standing`'s
/// `CASE` to `WHEN EXISTS(SELECT 1 FROM actor_current WHERE signing_key_id = $1) THEN 'enrolled'`
/// — the obvious simplification, and the exact shape the retired `ensure_registration_actor`
/// used before #654. Every other test in this file stays green: none of them ever stages two
/// rows, so `count(*) > 1` and `EXISTS` are indistinguishable to them. The `Ambiguous` branch
/// and its refusal vanish in silence, and the next dual-mapped key writes unattributed events.
///
/// Reachable **today**, not only through a future door: see `ActorStanding::Ambiguous`.
///
/// Found by the PR #661 review (the branch had no test at all).
#[tokio::test]
async fn a_key_mapping_to_two_current_actors_refuses_rather_than_unattributing() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let _ = setup(&c, &[]).await;
    let kid = a_key_id("ambiguous");

    assert!(enroll_device_actor(&c, &kid).await.unwrap());
    bind_key_to_a_second_actor(&c, &kid, "a-second-actor").await;
    assert_eq!(
        rows_for(&c, &kid).await,
        2,
        "the fixture must actually stage the dual mapping, or this test passes vacuously"
    );

    assert_eq!(
        device_actor_standing(&c, &kid).await.unwrap(),
        ActorStanding::Ambiguous,
        "two current actors is NOT `Enrolled`: authoring under it destroys attribution silently"
    );

    let e = require_device_actor(&c, &kid)
        .await
        .expect_err("a dual-mapped key may not author");
    let rendered = format!("{e:#}");
    assert!(
        carries_refusal_marker(&e),
        "it is a verdict about this node's state, not an outage — offering a retry-now would \
         be offering one that cannot work; got: {rendered}"
    );
    assert!(
        rendered.contains("MORE THAN ONE"),
        "the operator must be told to INVESTIGATE rather than to re-run a command: this is not \
         a state any command fixes, and the words are how they know that; got: {rendered}"
    );

    // And the provisioning command refuses too, rather than adding a THIRD mapping on top.
    assert!(
        enroll_device_actor(&c, &kid).await.is_err(),
        "enrolling again must not deepen an ambiguity it cannot resolve"
    );
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
        carries_refusal_marker(&e),
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
/// ⚠️ **The "needs a virgin database" claim this doc used to make was FALSE, and the test that
/// disproves it is in this same file.** `init` does refuse over a registered custody key, but a
/// suite may truncate its way back to virgin state against the shared `cairn_test` fixture —
/// which is what `init_enrols_this_nodes_own_key_as_a_device_actor` (below) now does, covering
/// all three things this guard admits it cannot. #662's stated blocker was wrong; the issue
/// stays open for `init`'s *seven other* unpinned effects, which is the part that is still true.
///
/// **This guard is kept anyway, for a different reason than it was written for:** it needs no
/// database, so it runs on a `CAIRN_ALLOW_DB_SKIP=1` gate where the behavioural test silently
/// skips — which is the gate a developer sees green before pushing (PR #661 review).
///
/// **What this guard does NOT catch**, stated so nobody mistakes it for the real thing: that the
/// call is reached (it could sit behind a condition), that it is passed the right key, or that
/// `enroll_device_actor` does what it says — `init_enrols_this_nodes_own_key_as_a_device_actor`
/// covers all three, whenever a database is present. It catches deletion, which is the failure
/// the PR #661 review actually named, on the gate that has no database.
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
    // Comment-stripped, like `enrolment_is_never_a_write_side_effect.rs`'s scan — and for the
    // converse reason. There, a MENTION in prose must not read as a call; here, a call that has
    // been COMMENTED OUT must not read as one either. Commenting a line out is the commonest
    // way a line gets "deleted" while debugging, and this guard's whole job is catching the
    // deletion (PR #661 review).
    let arm_code: String = arm
        .lines()
        .map(strip_comment)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        arm_code.contains("actor_enrolment::enroll_device_actor"),
        "`cairn-node init` no longer enrols the node's device actor. If that removal was \
         deliberate, note that it makes an ordinary operator run one extra command before their \
         node can author anything — `M = 0` becomes `M = 1` in the §1.2 benchmark of #654 — and \
         update that benchmark rather than deleting this guard. Init arm read:\n{arm}"
    );
}

/// ⇒ THE BEHAVIOURAL TEST #662 SAID WAS BLOCKED. IT WAS NOT.
///
/// `init_still_enrols_the_device_actor` above is a source guard, and it was filed as the best
/// available answer because a real test *"would need a VIRGIN database"* — `init` refuses over a
/// registered custody key (`refuse_init_over_a_registered_custody_key`) and over an existing
/// unwrap-key file, so it looked unable to run against the shared `cairn_test` fixture.
///
/// **That blocker was false, and the PR #661 review demonstrated it by running it.** The two
/// pieces of state `init` refuses over are both clearable with helpers this tree already has:
/// `node_unwrap_key` is in `clinic_kit`'s truncation list, and `local_node` is cleared by
/// `cairn_node::db::reset_node_federation_tables`. With `--insecure-plaintext` there is no
/// passphrase and no recovery code to feed — the same shape `restore_kit::restore_cli` already
/// uses to drive a provisioning-class command against the shared database.
///
/// So this covers all three things the source guard's own doc honestly admits it cannot: that
/// the call is **reached** (not behind a condition), that it is passed the **right key**, and
/// that enrolment **actually happened**.
#[tokio::test]
async fn init_enrols_this_nodes_own_key_as_a_device_actor() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    // Clear exactly what `init` refuses over, and nothing else.
    c.batch_execute("TRUNCATE event_log, actor_event, patient_chart, node_unwrap_key CASCADE")
        .await
        .unwrap();
    db::reset_node_federation_tables(&c).await.unwrap();

    let dir = tempfile::tempdir().unwrap();
    let key = dir.path().join("node.key");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_cairn-node"))
        .args(["--conn", &base, "--key"])
        .arg(&key)
        .args([
            "init",
            "--name",
            "init-enrolment-probe",
            "--address",
            "127.0.0.1:7999",
            // No passphrase, no recovery code: that branch is taken before either is read.
            "--insecure-plaintext",
        ])
        .output()
        .expect("the binary Cargo just built must be runnable");
    assert!(
        out.status.success(),
        "init must succeed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // The key `init` enrolled must be THIS node's signing key, not merely "some" actor — the
    // source guard cannot tell those apart and that is half of why this test exists.
    let seed = std::fs::read(&key).expect("init wrote a plaintext seed");
    let sk = cairn_event::SigningKey::from_bytes(
        &<[u8; 32]>::try_from(&seed[..32]).expect("an Ed25519 seed is 32 bytes"),
    );
    let kid = hex::encode(sk.verifying_key().to_bytes());

    assert_eq!(
        device_actor_standing(&c, &kid).await.unwrap(),
        ActorStanding::Enrolled,
        "an initialised node must be able to author immediately — that is what keeps the §1.2 \
         step count at M = 0 for an ordinary operator (#654). If this fails, every fresh node \
         silently cannot write until somebody runs `enroll-device-actor`."
    );
    assert_eq!(
        rows_for(&c, &kid).await,
        1,
        "and exactly one actor, or db/005 nulls the actor_id of everything it ever writes"
    );

    // Leave the shared database as we found it.
    //
    // ⚠️ This does NOT run if an assertion above panics, and there is no `Drop` guard here on
    // purpose: cleanup needs an await, `Drop` cannot have one, and no kit in this tree carries
    // an async teardown to copy. What bounds the damage instead is the truncation at the TOP of
    // this test — it begins from clean rather than trusting the previous run's exit — so a
    // panicking run can only leak into a suite that both runs after it and does not reset
    // `local_node` itself. `test_serial_guard` serialises but does not clean up. If a
    // neighbouring suite ever starts failing for a reason that makes no sense, this is the
    // first place to look (PR #661 review).
    c.batch_execute("TRUNCATE event_log, actor_event, patient_chart, node_unwrap_key CASCADE")
        .await
        .unwrap();
    db::reset_node_federation_tables(&c).await.unwrap();
}
