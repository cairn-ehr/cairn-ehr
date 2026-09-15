//! #584 / ADR-0070 — the two invariants a late key's projection rests on, checked over the
//! DATABASE CATALOGUE (what actually runs), not over `db/*.sql` text.
//!
//! 1. **A registered applier that reads custody is heal-safe.** The doors re-run only heal-safe
//!    appliers when a key lands late. An applier that reads the clear view but is registered
//!    `heal_safe = false` would be skipped at the landing and leave the chart owed a rebuild —
//!    the debt `requeue`'s `reproject_owed` used to report, now made unrepresentable instead.
//! 2. **Every function that writes `event_clear` calls `cairn_project_late_custody`.** A third
//!    custody writer that did not would reopen #584 through its own entrance. The writer set is
//!    pinned by name, so a new one is a decision rather than a drift.
//!
//! # Honest residual
//!
//! Both read an applier's OWN body (`pg_proc.prosrc`). An applier that reads custody only through a
//! helper it calls is invisible to rule 1. Every custody reader today calls `cairn_clear_payload`
//! directly in its own body, which is what the positive control asserts.
//!
//! `--` line comments are stripped before matching, so prose naming a function neither satisfies
//! nor trips a rule. The pure predicates are exercised without a database below, so this file
//! proves something even where `$CAIRN_TEST_PG` is unset.
//!
//! The comment stripper is per-line and literal-blind: a `--` inside a string literal truncates
//! the rest of its line, so a custody read or a late-custody call written AFTER such a literal on
//! the same line would be missed. No call site today shares a line with a `--`; keep each of
//! these calls on its own line.

use cairn_node::db;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// The body with every `--` line comment removed. **Pure, and literal-blind**: it truncates a
/// line at its first `--` with no awareness of string or dollar-quoted literals, so a `--`
/// inside a literal would hide the rest of that line rather than being recognised as data.
fn without_line_comments(body: &str) -> String {
    body.lines()
        .map(|line| match line.find("--") {
            Some(at) => &line[..at],
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Uppercase with every whitespace run collapsed to one space, so `insert  into\n event_clear`
/// matches. **Pure.**
fn normalised(body: &str) -> String {
    without_line_comments(body)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_uppercase()
}

/// Does this function body read custody — the clear view a sealed body opens into? **Pure.**
fn reads_custody(body: &str) -> bool {
    let n = normalised(body);
    n.contains("CAIRN_CLEAR_PAYLOAD") || n.contains("EVENT_CLEAR")
}

/// Does this function body write the clear view? **Pure.**
fn writes_custody(body: &str) -> bool {
    normalised(body).contains("INSERT INTO EVENT_CLEAR")
}

/// Does this function body call the late-custody projection? **Pure.**
fn projects_late_custody(body: &str) -> bool {
    normalised(body).contains("CAIRN_PROJECT_LATE_CUSTODY(")
}

#[test]
fn the_predicates_read_code_and_ignore_prose() {
    assert!(reads_custody("p jsonb := cairn_clear_payload(e);"));
    assert!(!reads_custody(
        "-- cairn_clear_payload is not called here\nRETURN;"
    ));
    assert!(writes_custody(
        "insert  into\n   event_clear (event_id) VALUES (x)"
    ));
    assert!(!writes_custody(
        "-- INSERT INTO event_clear happens in the door\nRETURN;"
    ));
    assert!(projects_late_custody(
        "PERFORM cairn_project_late_custody(v_event_id);"
    ));
    assert!(!projects_late_custody(
        "-- see cairn_project_late_custody(v_event_id)\nRETURN;"
    ));
}

/// Rule 1, over every registry row.
#[tokio::test]
async fn every_custody_reading_applier_is_heal_safe() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let rows = c
        .query(
            "SELECT r.event_type, r.apply_fn, r.heal_safe, p.prosrc \
               FROM cairn_projection_apply r \
               JOIN pg_proc p ON p.oid = to_regprocedure(r.apply_fn || '(event_log)')",
            &[],
        )
        .await
        .unwrap();

    let readers: Vec<(String, String, bool)> = rows
        .iter()
        .filter(|r| reads_custody(r.get::<_, &str>(3)))
        .map(|r| (r.get(0), r.get(1), r.get(2)))
        .collect();

    // POSITIVE CONTROL: a rule that sees no readers passes over anything (#586's lesson).
    assert!(
        readers
            .iter()
            .any(|(_, f, _)| f == "medication_statement_apply"),
        "the guard must see the custody readers it exists for; saw {readers:?}"
    );
    assert!(
        readers.len() >= 9,
        "expected at least the nine medication appliers to read custody; saw {readers:?}"
    );

    let unsafe_readers: Vec<_> = readers.iter().filter(|(_, _, safe)| !safe).collect();
    assert!(
        unsafe_readers.is_empty(),
        "these appliers read custody but are registered heal_safe = false, so a key that lands \
         late would skip them and leave the chart owed a rebuild (ADR-0070 decision 3). Make the \
         applier idempotent and heal-safe: {unsafe_readers:?}"
    );
}

/// Rule 2, over every PL/pgSQL and SQL-language function in the schema (C and internal
/// functions have no SQL body to read).
#[tokio::test]
async fn every_custody_writer_projects_a_late_key() {
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
        .filter(|r| writes_custody(r.get::<_, &str>(1)))
        .map(|r| (r.get(0), projects_late_custody(r.get::<_, &str>(1))))
        .collect();
    writers.sort();

    assert_eq!(
        writers.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
        vec!["apply_remote_event", "submit_event"],
        "the custody writers are the two doors. A third is a DECISION: give it the late-custody \
         call (ADR-0070) and add it here"
    );
    for (name, calls) in &writers {
        assert!(
            calls,
            "{name} writes event_clear but never calls cairn_project_late_custody — a key landing \
             through it would leave the chart empty (#584)"
        );
    }
}
