//! #619 / ADR-0073 — every function that writes an event log calls the substitution refusal.
//!
//! Checked over the DATABASE CATALOGUE (`pg_proc`, what actually runs), like
//! `late_custody_guards.rs` rule 2.
//!
//! # Why a catalogue rule and not a list
//!
//! Until #619 the inventory of guarded doors was a hand-written list in
//! `substitution_guard_is_single_source.rs`, and it was WRONG: #615 and ADR-0072's first draft said
//! "two of the three write doors" refuse a substitution, counting the two `event_log` doors beside
//! the restore door and omitting `node_event`'s other two writers entirely — `submit_node_event` and
//! `apply_remote_node_event`, five unguarded sites, one of them the live federation admission gate.
//! A list says what its author believed. This rule derives the writer set from the functions that
//! exist, so a writer nobody thought of is found rather than trusted.
//!
//! The derived set is still PINNED by name, so a sixth writer fails here and becomes a decision —
//! give it the call (and say why) — rather than a drift.
//!
//! # Honest residuals
//!
//! * It reads a function's OWN body. A write through a helper, a `MERGE`, or a dynamic `EXECUTE
//!   format(...)` is not recognised. None exists; review a new one by hand.
//! * It covers the two EVENT logs, `event_log` and `node_event`. The actor registry's `actor_event`
//!   has the same silent-discard shape at db/052's door — that is #569, open, and widening this rule
//!   to it would fail today and pull #569 into #619.
//! * Comments are stripped before matching (`common/sql_text.rs`), so prose neither satisfies nor
//!   trips it; the stripper is literal-blind.

#[path = "common/sql_text.rs"]
mod sql_text;

use cairn_node::db;
use sql_text::normalised;

/// The test database, or `None` when `$CAIRN_TEST_PG` is unset — the repo-wide self-skip,
/// policed by `tests/db_gate_actually_ran.rs`.
fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// Does this body `INSERT` into one of the two event logs? **Pure.**
///
/// The table name must be followed by a space or `(`, so `node_event_quarantine` (a pen written
/// from Rust, not a door) never matches `node_event`. A `public.`-qualified write counts too.
fn writes_an_event_log(body: &str) -> bool {
    let n = normalised(body);
    ["EVENT_LOG", "NODE_EVENT"].iter().any(|table| {
        [
            format!("INSERT INTO {table} "),
            format!("INSERT INTO {table}("),
            format!("INSERT INTO PUBLIC.{table} "),
            format!("INSERT INTO PUBLIC.{table}("),
        ]
        .iter()
        .any(|needle| n.contains(needle.as_str()))
    })
}

/// Does this body call the shared refusal? **Pure.**
fn refuses_substitution(body: &str) -> bool {
    normalised(body).contains("CAIRN_REFUSE_SUBSTITUTION(")
}

#[test]
fn the_predicates_read_code_and_ignore_prose() {
    assert!(writes_an_event_log(
        "INSERT INTO node_event (node_event_id) VALUES (x)"
    ));
    assert!(writes_an_event_log(
        "insert into\n   event_log\n(event_id) values (x)"
    ));
    assert!(writes_an_event_log(
        "INSERT INTO public.event_log (event_id) VALUES (x)"
    ));
    assert!(!writes_an_event_log(
        "INSERT INTO node_event_quarantine (content_digest) VALUES (x)"
    ));
    assert!(!writes_an_event_log(
        "-- INSERT INTO node_event happens in the door\nRETURN;"
    ));
    assert!(refuses_substitution(
        "PERFORM cairn_refuse_substitution(v_found, v_ca, v_eid, 'd');"
    ));
    assert!(!refuses_substitution(
        "-- cairn_refuse_substitution(v_found) is called by the door\nRETURN;"
    ));
    assert!(!refuses_substitution(
        "/* PERFORM cairn_refuse_substitution(a, b, c, d); */ RETURN;"
    ));
}

#[tokio::test]
async fn every_event_log_writer_refuses_a_substitution() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let rows = c
        .query(
            "SELECT p.proname, p.prosrc FROM pg_proc p \
               JOIN pg_namespace n ON n.oid = p.pronamespace \
               JOIN pg_language l ON l.oid = p.prolang \
              WHERE n.nspname = 'public' AND l.lanname IN ('plpgsql', 'sql')",
            &[],
        )
        .await
        .unwrap();

    let mut writers: Vec<(String, bool)> = rows
        .iter()
        .filter(|r| writes_an_event_log(r.get::<_, &str>(1)))
        .map(|r| (r.get(0), refuses_substitution(r.get::<_, &str>(1))))
        .collect();
    writers.sort();

    // POSITIVE CONTROL and PIN in one: the rule must SEE the five writers it exists for (a rule
    // that sees none passes over anything — #586's lesson), and a sixth is a decision.
    assert_eq!(
        writers.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
        vec![
            "apply_remote_event",
            "apply_remote_node_event",
            "restore_node_event",
            "submit_event",
            "submit_node_event",
        ],
        "the event-log writers are these five doors. A sixth is a DECISION: give it the \
         cairn_refuse_substitution call (ADR-0072/0073) and add it here, saying why"
    );
    let unguarded: Vec<&str> = writers
        .iter()
        .filter(|(_, guarded)| !guarded)
        .map(|(n, _)| n.as_str())
        .collect();
    assert!(
        unguarded.is_empty(),
        "these functions write an event log but never call cairn_refuse_substitution, so a \
         second, different event under a held id vanishes behind ON CONFLICT DO NOTHING (#619): \
         {unguarded:?}"
    );
}
