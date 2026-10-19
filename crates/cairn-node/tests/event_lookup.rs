//! #822: `cairn-node event <id>` — the check an unknown-outcome commit asks the operator to make.
//!
//! Every marked commit failure says "check whether it was recorded before retrying". Before
//! #822 the only way to check was raw psql against `event_log`. These pin the read the command
//! is built on: an event this node holds is found, with its type and chart; one it does not hold
//! is reported absent, not as an error; and a lookup that FAILS is an error, never "absent" —
//! "absent" invites a re-run, and a re-run of a write that did land records it twice. The last
//! test runs the real binary, so the command the remedy line names is the command that exists.
//!
//! DB-gated on $CAIRN_TEST_PG, guard-before-connect.
mod common;

use cairn_node::db;
use cairn_node::event_lookup::recorded_event;
use common::cs;
use std::process::Command;
use uuid::Uuid;

/// A registration this node holds is found, naming its type and the chart it is filed in.
#[tokio::test]
async fn an_event_this_node_holds_is_found_with_its_type_and_chart() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = common::setup(&c, &[]).await;
    let patient = Uuid::now_v7();
    let event = common::submit_registration(&c, &sk, &kid, patient, 1).await;

    // As the role production's `--conn` login is a member of, not as the test's superuser.
    c.batch_execute("SET ROLE cairn_node").await.unwrap();
    let found = recorded_event(&c, event)
        .await
        .unwrap()
        .expect("the registration is recorded");
    assert_eq!(found.event_type, "identity.registration.asserted");
    assert_eq!(found.chart, patient);
    assert!(!found.recorded_at.is_empty(), "says when it was stored");
    c.batch_execute("RESET ROLE").await.unwrap();
}

/// An event this node does not hold is absent — `Ok(None)`, an answer — never an error.
#[tokio::test]
async fn an_event_this_node_does_not_hold_is_absent_not_an_error() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    assert!(recorded_event(&c, Uuid::now_v7()).await.unwrap().is_none());
}

/// A lookup that cannot read `event_log` is an ERROR, not `Ok(None)`: "not recorded" from a
/// lookup that never looked would send the operator to run a write again that may have landed.
/// The fault is injected by switching to a role with no grant on `event_log`.
#[tokio::test]
async fn a_lookup_that_cannot_read_is_an_error_not_absent() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute(
        "DO $$ BEGIN CREATE ROLE cairn_test_no_event_log NOLOGIN; \
         EXCEPTION WHEN duplicate_object THEN NULL; END $$; \
         SET ROLE cairn_test_no_event_log",
    )
    .await
    .unwrap();
    let looked = recorded_event(&c, Uuid::now_v7()).await;
    c.batch_execute("RESET ROLE").await.unwrap();
    let e = looked.expect_err("a lookup that could not read must not answer \"absent\"");
    assert!(format!("{e:#}").contains("event_log"), "{e:#}");
}

/// The command itself, through the real binary: `--conn` goes BEFORE the subcommand (it is not a
/// global flag), as the remedy line writes it; a held event says so, an absent one says so.
#[tokio::test]
async fn the_event_command_answers_both_ways() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = common::setup(&c, &[]).await;
    let held = common::submit_registration(&c, &sk, &kid, Uuid::now_v7(), 1).await;

    let run = |id: Uuid| {
        let out = Command::new(env!("CARGO_BIN_EXE_cairn-node"))
            .env_remove("CAIRN_CONN")
            .args(["--conn", &base, "event", &id.to_string()])
            .output()
            .expect("the binary Cargo just built must be runnable");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    assert!(run(held).contains(&format!("event {held} IS recorded")));
    let absent = Uuid::now_v7();
    assert!(run(absent).contains(&format!("event {absent} is NOT recorded")));
}
