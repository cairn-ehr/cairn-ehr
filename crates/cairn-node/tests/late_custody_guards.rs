//! #584 / ADR-0070 — the two invariants a late key's projection rests on, checked over the
//! DATABASE CATALOGUE (what actually runs), not over `db/*.sql` text.
//!
//! 1. **A registered applier that reads custody is heal-safe.** The doors re-run only heal-safe
//!    appliers when a key lands late. An applier that reads the clear view but is registered
//!    `heal_safe = false` would be skipped at the landing and leave the chart owed a rebuild —
//!    the debt `requeue`'s `reproject_owed` used to report, now made unrepresentable instead.
//! 2. **Every PL/pgSQL or SQL function that `INSERT`s into `event_clear` calls
//!    `cairn_project_late_custody`.** A third function inserting into `event_clear` that did not
//!    would reopen #584 through its own entrance. The set of such functions is pinned by name, so
//!    a new one is a decision rather than a drift.
//!
//! # Honest residual
//!
//! Both read a function's OWN body (`pg_proc.prosrc`). An applier that reads custody only through a
//! helper it calls is invisible to rule 1. Every custody reader today calls `cairn_clear_payload`
//! directly in its own body, which is what the positive control asserts.
//!
//! Rule 2 matches literal text, so it recognises `INSERT INTO event_clear` and
//! `INSERT INTO public.event_clear` and nothing else. A `MERGE INTO event_clear`, or a write built
//! as a string and run through a dynamic `EXECUTE format(...)`, is NOT recognised. None exists
//! today; a new one must be reviewed by hand for the late-custody call.
//!
//! Comments — `--` line comments and `/* ... */` block comments — are stripped before matching, so
//! prose naming a function neither satisfies nor trips a rule. (A block comment that named the
//! call used to satisfy rule 2: the unsafe direction.) The pure predicates are exercised without a
//! database below, so this file proves something even where `$CAIRN_TEST_PG` is unset.
//!
//! The comment stripper is literal-blind: a `--` inside a string literal hides the rest of its
//! line, and a `/*` inside one hides everything up to the next `*/`, so a custody read or a
//! late-custody call written after such a literal would be missed. No call site today follows a
//! comment marker inside a literal; keep each of these calls clear of one.

use cairn_node::db;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// The body with its SQL comments removed: `--` line comments and `/* ... */` block comments.
/// **Pure, and literal-blind.**
///
/// One left-to-right pass, because the two comment kinds hide each other and the order matters:
/// a `/*` inside a line comment opens nothing (our own SQL writes `-- ... db/*.sql ...` in
/// function bodies), and a `--` inside a block comment is just comment text. Stripping block
/// comments first and line comments second would get the first case wrong and swallow real code.
///
/// Block comments NEST in Postgres (`/* a /* b */ still comment */`), so a depth counter tracks
/// them; a comment is replaced by one space because it separates tokens exactly as a space does.
/// An unterminated block comment swallows the rest of the body — Postgres would refuse such a
/// body anyway.
///
/// Literal-blind: there is no awareness of string or dollar-quoted literals, so a `--` inside a
/// literal hides the rest of that line and a `/*` inside one hides everything up to its `*/`.
fn without_sql_comments(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    // > 0 while inside a block comment; counts how many `/*` are still open.
    let mut block_depth = 0usize;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("/*") {
            // Checked first so a `/*` opens (or nests) a block comment wherever it starts —
            // unless an earlier `--` on this line already skipped it (the branch below).
            block_depth += 1;
            rest = after;
        } else if block_depth > 0 {
            if let Some(after) = rest.strip_prefix("*/") {
                block_depth -= 1;
                if block_depth == 0 {
                    out.push(' ');
                }
                rest = after;
            } else {
                rest = without_first_char(rest);
            }
        } else if let Some(after) = rest.strip_prefix("--") {
            // Skip to the end of the line, keeping the newline itself.
            rest = after.find('\n').map_or("", |at| &after[at..]);
        } else {
            let mut chars = rest.chars();
            if let Some(ch) = chars.next() {
                out.push(ch);
            }
            rest = chars.as_str();
        }
    }
    out
}

/// `text` without its first character (UTF-8 aware, so a multi-byte character such as an em dash
/// in a comment is never split). **Pure.**
fn without_first_char(text: &str) -> &str {
    let mut chars = text.chars();
    chars.next();
    chars.as_str()
}

/// Uppercase with every whitespace run collapsed to one space, so `insert  into\n event_clear`
/// matches. **Pure.**
fn normalised(body: &str) -> String {
    without_sql_comments(body)
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

/// Does this function body `INSERT` into the clear view? **Pure.**
///
/// Both spellings a function body can use: bare, and schema-qualified with `public.` (a qualified
/// write is as real as a bare one). Only `INSERT` is recognised — see the header's residual for
/// `MERGE` and dynamic `EXECUTE`. `reads_custody` needs no such twin: it matches the bare names as
/// substrings, and a `public.`-qualified name contains its bare name. (Here the bare pattern
/// `INSERT INTO EVENT_CLEAR` is NOT a substring of the qualified write, hence the second check.)
fn writes_custody(body: &str) -> bool {
    let n = normalised(body);
    n.contains("INSERT INTO EVENT_CLEAR") || n.contains("INSERT INTO PUBLIC.EVENT_CLEAR")
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
    // A schema-qualified write is a write.
    assert!(writes_custody(
        "INSERT INTO public.event_clear (event_id) VALUES (x)"
    ));
    assert!(projects_late_custody(
        "PERFORM cairn_project_late_custody(v_event_id);"
    ));
    assert!(!projects_late_custody(
        "-- see cairn_project_late_custody(v_event_id)\nRETURN;"
    ));

    // Block comments are prose too. A call that exists ONLY inside one must not satisfy rule 2 —
    // counting it is the unsafe direction (a writer without the call would pass) — whether the
    // comment is on one line, spans lines, or is nested (Postgres nests block comments).
    assert!(!projects_late_custody(
        "/* PERFORM cairn_project_late_custody(v_event_id); */\nRETURN;"
    ));
    assert!(!projects_late_custody(
        "/*\n   PERFORM cairn_project_late_custody(v_event_id);\n*/\nRETURN;"
    ));
    assert!(!projects_late_custody(
        "/* outer /* inner */ PERFORM cairn_project_late_custody(v_event_id); */ RETURN;"
    ));
    // ...and stripping must not eat real code: a call after a block comment on another line
    // counts, and a `/*` inside a LINE comment (our SQL writes `db/*.sql` in comments) opens
    // nothing, so the call on the next line still counts.
    assert!(projects_late_custody(
        "/* the late-custody call\n   follows */\nPERFORM cairn_project_late_custody(v_event_id);"
    ));
    assert!(projects_late_custody(
        "-- the loader replays db/*.sql on every connect\nPERFORM cairn_project_late_custody(v_event_id);"
    ));
    assert!(writes_custody(
        "/* custody */ INSERT INTO event_clear (event_id) VALUES (x)"
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
    // LEFT JOIN, not JOIN: an inner join silently DROPS a registry row whose apply_fn does not
    // resolve to a `(event_log)` function, and a dropped row is a row this rule never checked.
    let rows = c
        .query(
            "SELECT r.event_type, r.apply_fn, r.heal_safe, p.prosrc \
               FROM cairn_projection_apply r \
               LEFT JOIN pg_proc p ON p.oid = to_regprocedure(r.apply_fn || '(event_log)')",
            &[],
        )
        .await
        .unwrap();

    // Every registry row must resolve to a function (a NULL prosrc means the LEFT JOIN found
    // none), or the rule below would pass over it unseen.
    let unresolved: Vec<(String, String)> = rows
        .iter()
        .filter(|r| r.get::<_, Option<&str>>(3).is_none())
        .map(|r| (r.get(0), r.get(1)))
        .collect();
    assert!(
        unresolved.is_empty(),
        "these registry rows name an apply_fn that does not resolve to a `(event_log)` function, \
         so rule 1 cannot read their bodies: {unresolved:?}"
    );

    let readers: Vec<(String, String, bool)> = rows
        .iter()
        .filter(|r| r.get::<_, Option<&str>>(3).is_some_and(reads_custody))
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

/// Rule 2 — every PL/pgSQL or SQL function that `INSERT`s into `event_clear` calls
/// `cairn_project_late_custody` — over every such function in the schema (C and internal
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
