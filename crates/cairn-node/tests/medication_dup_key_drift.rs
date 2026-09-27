//! R1 Task 2 / ADR-0076 — `cairn_medication_duplicate_groups` (db/054) computes the SAME
//! duplicate-drug key as `patient_medication_reconciliation_flag` (db/033, itself kept in
//! lockstep with db/031 by that view's own comment), just over a caller-chosen SET of
//! charts instead of one patient's own. Nothing in SQL keeps the two `coalesce('code:' …)`
//! expressions identical, so a future edit to one — dropping a `COLLATE "C"` pin, say —
//! would silently make the set-wide duplicate flag (a double-dose reading hazard once
//! charts are linked) disagree with the single-chart one it is supposed to generalise.
//!
//! No database: the migration SQL is `include_str!`-embedded at compile time (same as
//! `db::SCHEMA`), so this is a SOURCE-LEVEL guard, the `name_winner_order_drift.rs` idiom
//! (#159) — it runs in every `cargo test` / CI pass and needs no Postgres.

/// The two migrations whose dup_key expression must stay byte-for-byte in lockstep.
/// Paths resolve the same way `src/db.rs` does — a test file sits at the same depth
/// under the crate as `src/`, so `../../../db/…` reaches the repo-root `db/` directory.
const DB033: &str = include_str!("../../../db/033_medication_reconciliation.sql");
const DB054: &str = include_str!("../../../db/054_person_charts.sql");

/// Collapse every run of whitespace to a single space and trim — so a cosmetic reflow or
/// re-indent of one file does NOT trip the guard, while any semantic change (a reordered
/// operand, a dropped `COLLATE "C"` pin) DOES change the result.
fn normalize_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Every occurrence of the dup_key `coalesce('code:' …)` expression in `sql`, whitespace-
/// normalized, in source order.
///
/// Fixed markers, per the task brief: start at `coalesce('code:' || (` and end at the
/// FIRST `COLLATE "C"))` after that start (the double close-paren is what distinguishes
/// the whole expression's end from the two inner `… COLLATE "C")` closes inside it — see
/// db/033's own text, which nests exactly this way). Scanning resumes after each match's
/// end, so overlapping or adjacent copies are all found rather than just the first.
fn extract_all(sql: &str) -> Vec<String> {
    const START: &str = "coalesce('code:' || (";
    const END: &str = "COLLATE \"C\"))";
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(rel_start) = sql[pos..].find(START) {
        let abs_start = pos + rel_start;
        let Some(rel_end) = sql[abs_start..].find(END) else {
            break;
        };
        let abs_end = abs_start + rel_end + END.len();
        out.push(normalize_ws(&sql[abs_start..abs_end]));
        pos = abs_end;
    }
    out
}

/// TDD unit + positive control on synthetic SQL: the extractor finds every copy, tolerates
/// whitespace reflow, and reflects a semantic change (a dropped COLLATE pin) as drift. A
/// guard whose extractor silently finds zero occurrences would pass while checking
/// nothing — the anti-vacuity lesson `paper_parity_plan_section.rs` and
/// `name_winner_order_drift.rs` both learned the hard way.
#[test]
fn extractor_finds_every_copy_and_detects_drift() {
    let two_copies = "\
SELECT coalesce('code:' || (coding_system COLLATE \"C\") || '|' || (coding_code COLLATE \"C\"),
                 'term:' || lower(btrim(term) COLLATE \"C\")) AS dup_key
GROUP BY coalesce('code:' || (coding_system COLLATE \"C\") || '|' || (coding_code COLLATE \"C\"),
                   'term:' || lower(btrim(term) COLLATE \"C\"));
";
    let got = extract_all(two_copies);
    assert_eq!(got.len(), 2, "must find BOTH copies, not just the first");
    assert_eq!(
        got[0], got[1],
        "the two copies in this synthetic source are identical"
    );

    // Reflowed onto one line with different spacing — normalization makes it equal.
    let reflowed = two_copies.replace(
        "coalesce('code:' || (coding_system COLLATE \"C\") || '|' || (coding_code COLLATE \"C\"),\n                 'term:' || lower(btrim(term) COLLATE \"C\")) AS dup_key",
        "coalesce('code:'   ||   (coding_system COLLATE \"C\") || '|' || (coding_code COLLATE \"C\"),   'term:' || lower(btrim(term) COLLATE \"C\")) AS dup_key",
    );
    assert_eq!(
        extract_all(&reflowed)[0],
        got[0],
        "a cosmetic reflow must NOT read as drift"
    );

    // Dropping one COLLATE "C" pin is exactly the #69-shaped regression — it MUST change
    // the result, proving the guard is not vacuously comparing something un-derived.
    let de_collated = two_copies.replace("(coding_system COLLATE \"C\")", "(coding_system)");
    let mutated = extract_all(&de_collated);
    assert_ne!(
        mutated[0], got[0],
        "a dropped COLLATE \"C\" pin must read as drift"
    );

    assert!(extract_all("no expression here").is_empty());
}

/// The guard proper: db/033's dup_key expression (its `SELECT` list copy and its `GROUP
/// BY` copy — 2 occurrences, pinned so a guard that silently finds none cannot pass) must
/// be byte-identical (after whitespace normalization) to db/054's ONE copy (fix round 1,
/// review minor b: db/054 now writes the expression once, in its `keyed` CTE, and both
/// the outer SELECT and the HAVING-grouped subquery read it from there — so there is only
/// one place in db/054 left to drift).
#[test]
fn db033_and_db054_dup_key_are_byte_identical() {
    let db033 = extract_all(DB033);
    let db054 = extract_all(DB054);

    assert_eq!(
        db033.len(),
        2,
        "expected exactly 2 copies of the dup_key expression in db/033 (its view's SELECT \
         list, plus its GROUP BY) — got {}: {:?}",
        db033.len(),
        db033
    );
    assert_eq!(
        db054.len(),
        1,
        "expected exactly 1 copy of the dup_key expression in db/054 (the `keyed` CTE — \
         fix round 1 collapsed the earlier two-copy `t`/`u` shape into one) — got {}: {:?}",
        db054.len(),
        db054
    );
    assert_eq!(
        db033[0], db033[1],
        "db/033's own two copies (SELECT list vs GROUP BY) have already drifted from each \
         other — fix db/033 before this guard can say anything about db/054"
    );

    for (i, expr) in db054.iter().enumerate() {
        assert_eq!(
            expr, &db033[0],
            "db/054's dup_key copy #{i} has DRIFTED from db/033's \
             patient_medication_reconciliation_flag.\n\
             db/033: {}\n\
             db/054 copy {i}: {expr}\n\
             Keep every COLLATE \"C\" pin identical, or the set-wide duplicate flag and the \
             single-chart one will disagree about what counts as a duplicate.",
            db033[0]
        );
    }
}
