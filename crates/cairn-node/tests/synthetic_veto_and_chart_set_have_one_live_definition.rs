//! db/088 re-declares two functions that db/016 and db/054 define first (ADR-0115).
//!
//! `cairn_match_veto` and `cairn_person_charts` are `LANGUAGE sql`, whose bodies Postgres
//! validates at CREATE time, and the rule ADR-0115 adds to each calls a function that only
//! exists from db/088 on. So db/088 `CREATE OR REPLACE`s them, and on every schema replay
//! the earlier definition is created and then superseded.
//!
//! That leaves two copies of each body in the tree, and the earlier one is the copy a reader
//! of db/016 or db/054 sees first. If someone fixes a bug THERE, the fix is silently undone
//! a few files later. This guard holds the two in step: the live body must be the earlier
//! body plus exactly the ADR-0115 addition, and nothing else.
//!
//! Source-level (no database), so it runs in every `cargo test`.

use std::path::PathBuf;

fn db_file(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../db")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{} is readable: {e}", path.display()))
}

/// The `$$ … $$` body of `function` as defined in `sql`, with comments dropped and
/// whitespace collapsed, so the comparison is about SQL and not about layout.
fn body_of(sql: &str, function: &str) -> String {
    let start = sql
        .find(&format!("CREATE OR REPLACE FUNCTION {function}("))
        .unwrap_or_else(|| panic!("{function} is defined"));
    let open = start + sql[start..].find("$$").expect("a body opens") + 2;
    let close = open + sql[open..].find("$$").expect("a body closes");
    sql[open..close]
        .lines()
        .map(|line| line.split("--").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn the_live_veto_is_the_original_plus_the_synthetic_line_arm() {
    let original = body_of(&db_file("016_match_veto.sql"), "cairn_match_veto");
    let live = body_of(&db_file("088_synthetic_patient.sql"), "cairn_match_veto");
    let original = original.trim_end_matches(';').trim();

    assert!(
        live.starts_with(original),
        "db/088's cairn_match_veto must begin with db/016's body unchanged.\n db/016: {original}\n db/088: {live}"
    );
    let added = live[original.len()..].trim();
    assert!(
        added.starts_with("UNION ALL SELECT 'synthetic_line'::text, 'hard_veto'::text"),
        "the only addition is the synthetic-line arm, got: {added}"
    );
    assert_eq!(
        added.matches("UNION ALL").count(),
        1,
        "exactly one arm was added: {added}"
    );
}

#[test]
fn the_live_chart_set_is_the_original_plus_one_predicate() {
    let original = body_of(&db_file("054_person_charts.sql"), "cairn_person_charts");
    let live = body_of(&db_file("088_synthetic_patient.sql"), "cairn_person_charts");
    let predicate = " AND NOT cairn_charts_straddle_synthetic_line(m.patient_id, p_patient)";

    assert_eq!(
        live.matches(predicate.trim()).count(),
        1,
        "db/088's cairn_person_charts carries the line predicate exactly once: {live}"
    );
    assert_eq!(
        live.replacen(predicate, "", 1),
        original,
        "with that predicate removed, db/088's cairn_person_charts must equal db/054's"
    );
}

/// The guard's own parser, checked against a known string: a guard that silently extracts
/// an empty body would pass every comparison above.
#[test]
fn the_body_extractor_reads_a_body_and_drops_comments() {
    let sql = "CREATE OR REPLACE FUNCTION f(a int)\nAS $$\n  SELECT 1  -- one\n   + 2;\n$$;";
    assert_eq!(body_of(sql, "f"), "SELECT 1 + 2;");
}
