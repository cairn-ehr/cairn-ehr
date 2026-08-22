//! #473 — a failing advisory lookup on the clinical write path must name its cause.
//!
//! `safety::advisory_or_withheld` exists so that a degraded safety projection is
//! distinguishable from a correctly empty one — its own doc comment says exactly that.
//! Rendering the error as `tokio_postgres::Error`'s `Display`, the literal string
//! `db error`, made it indistinguishable from *another kind of* degradation instead, on
//! the highest-consequence surface in the tree: `medication/coding.rs` and
//! `medication/assert.rs` reach this on every coded medication write.
//!
//! The five SQLSTATEs it hid are five different operator actions — `42P01`/`42883`
//! (db/049 never loaded here: schema skew, every write degrades until fixed), `42501` (a
//! revoked grant after a restore), `57014` (statement timeout, may self-heal), `55P03` /
//! `40P01` (lock contention, self-heals), `53300` (connections exhausted).
//!
//! Each test drives the REAL production feed against a statement the server refuses, so
//! what is pinned is the composition an operator actually meets rather than a restatement
//! of it.
//!
//! # How the failure is forced, and why it leaves no residue
//!
//! `SET search_path TO pg_catalog` on this connection alone. The lookups' functions then
//! cannot be resolved and the server answers `42883`. It is SESSION-local and dies with
//! the connection — unlike a `REVOKE` or a decoy trigger, which persist if a test panics
//! and poison every later suite in a shared test database.

use cairn_event::medication::SubstanceCoding;
use cairn_node::safety;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// Open a connection whose `search_path` cannot resolve this project's functions.
///
/// No schema load and no TRUNCATE: these tests write nothing, so they need neither the
/// serial guard nor a loaded database — a bare connection is the whole fixture.
async fn a_client_that_cannot_resolve_our_functions(base: &str) -> tokio_postgres::Client {
    let c = cairn_node::db::connect(base).await.unwrap();
    c.batch_execute("SET search_path TO pg_catalog")
        .await
        .unwrap();
    c
}

/// A SQLSTATE the server can only have supplied, rendered in the bracketed shape
/// `compose_db_diagnosis` agreed with `cairn-sync`. `42883` is "function does not exist";
/// `42P01` is "relation does not exist" — which of the two arrives depends on how the
/// planner resolves the unqualified name first, and either proves the same point.
fn names_a_resolution_sqlstate(text: &str) -> bool {
    text.contains("[42883]") || text.contains("[42P01]")
}

/// The acceptance criterion of #473, verbatim: *a failing safety lookup names the SQLSTATE
/// and the server's message*.
#[tokio::test]
async fn a_failing_class_lookup_names_the_sqlstate_and_the_message() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let c = a_client_that_cannot_resolve_our_functions(&base).await;

    let coding = SubstanceCoding {
        system: "drugref-moiety",
        code: "00000000-0000-0000-0000-000000000000",
        display: "atorvastatin",
    };
    let e = safety::lookup_class(&c, &coding)
        .await
        .expect_err("an unresolvable function always fails");
    let text = format!("{e}");

    assert_ne!(text, "db error", "the whole of #473: {text}");
    assert!(
        text.contains("the safety class lookup"),
        "the line must say WHICH lookup failed \u{2014} `advisory_or_withheld` prints it \
         beside its own `what`, and a bare cause leaves the two indistinguishable: {text}"
    );
    assert!(
        names_a_resolution_sqlstate(&text),
        "\u{2026}and carry the SQLSTATE, the only machine-stable part and therefore the \
         only part an operator can search a manual for: {text}"
    );
}

/// The same for the rung feed, which is the other half of §5.9 emission. Without it a
/// medication event ships at `SafetyRung::Existence` — the chart's grade never consulted —
/// and the only artifact of that said `db error`.
#[tokio::test]
async fn a_failing_rung_lookup_names_the_sqlstate_and_the_message() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let c = a_client_that_cannot_resolve_our_functions(&base).await;

    let e = safety::prospective_rung(&c, uuid::Uuid::nil(), None)
        .await
        .expect_err("an unresolvable function always fails");
    let text = format!("{e}");

    assert_ne!(text, "db error", "the whole of #473: {text}");
    assert!(
        text.contains("the standing sensitivity grade"),
        "the line must say WHICH lookup failed: {text}"
    );
    assert!(
        names_a_resolution_sqlstate(&text),
        "\u{2026}and carry the SQLSTATE: {text}"
    );
}

/// The chart-wide READ (`cairn-node patient-safety`) is the same species in the same file,
/// and it is the operator's own query — so a failure there is the one they are staring at
/// while they try to work out what is wrong.
#[tokio::test]
async fn a_failing_chart_report_names_the_sqlstate() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let c = a_client_that_cannot_resolve_our_functions(&base).await;

    // `expect_err` would need `SafetyLine: Debug`, and deriving it on a production type to
    // satisfy one test is the tail wagging the dog. A match says the same thing.
    let e = match safety::chart_safety(&c, uuid::Uuid::nil()).await {
        Ok(lines) => panic!(
            "an unresolvable function always fails, got {} line(s)",
            lines.len()
        ),
        Err(e) => e,
    };
    let text = format!("{e}");

    assert_ne!(text, "db error", "{text}");
    assert!(
        names_a_resolution_sqlstate(&text),
        "the SQLSTATE must survive: {text}"
    );
    // The assertion its two siblings have and this one was missing: without it, deleting
    // the context from `chart_safety`'s `map_err` left this test green (PR #478 review).
    // This is the operator's OWN `patient-safety` query, so the line they are staring at
    // must say which read failed, not merely that a read did.
    assert!(
        text.contains("reading the chart safety report"),
        "the line must name what was being done: {text}"
    );
}
