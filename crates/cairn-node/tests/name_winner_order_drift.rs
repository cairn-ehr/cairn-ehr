//! #159 — the `patient_name_current` winner ORDER BY is defined TWICE (db/012 and db/025's
//! anti-join re-definition), and db/025's copy — loading last — is the live one. Nothing in the
//! SQL keeps the two `ORDER BY` clauses identical, so a future change to one that misses the
//! other silently re-introduces the exact cross-node projection divergence #69/ADR-0045 closed
//! (a different displayed patient name on two honest nodes replaying the same events). A pure
//! SQL single-source-of-truth is infeasible here: `DISTINCT ON (patient_id)` forces each view to
//! carry its own ORDER BY, and db/025 must anti-join the repudiation set BEFORE the winner is
//! picked, so the ordering cannot be factored into a shared base view or window.
//!
//! This is therefore a SOURCE-LEVEL drift guard. The migration SQL is `include_str!`-embedded at
//! compile time (same as `db::SCHEMA`), so the guard reads the two clauses directly and needs NO
//! database — it runs in every `cargo test` and CI pass, catching drift in EITHER direction
//! (including db/012's otherwise-inert copy silently diverging from the live db/025 one).

/// The two migrations whose `patient_name_current` winner ordering must stay byte-for-byte in
/// lockstep. Paths resolve the same way `src/db.rs` does — a test file sits at the same depth
/// under the crate as `src/`, so `../../../db/…` reaches the repo-root `db/` directory.
const DB012: &str = include_str!("../../../db/012_demographics_names.sql");
const DB025: &str = include_str!("../../../db/025_identity_repudiate.sql");

/// Collapse every run of whitespace to a single space and trim — so a cosmetic reflow or
/// re-indent of one file does NOT trip the guard, while any semantic change (a reordered key, a
/// dropped `COLLATE "C"` pin, an added/removed tiebreak column) DOES change the result.
fn normalize_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Extract the winner `ORDER BY` clause of the `patient_name_current` view from a migration's
/// SQL source, whitespace-normalized. Returns `None` if the view (or its ORDER BY) is absent.
///
/// Pure and deterministic: locate the `CREATE OR REPLACE VIEW patient_name_current` statement,
/// take the first `ORDER BY` after it (the winner ordering — there is no other ORDER BY between
/// the view header and its statement terminator; the db/025 anti-join subquery has none), and cut
/// at the terminating `;`. Matching is case-insensitive on the SQL keywords so the guard survives
/// a future re-casing, while the compared clause keeps its real content (column names, `COLLATE
/// "C"` pins) verbatim under the whitespace fold.
fn winner_order_by(sql: &str) -> Option<String> {
    let lower = sql.to_ascii_lowercase();
    let header = lower.find("create or replace view patient_name_current")?;
    // Search only from the view header onward, so an unrelated ORDER BY in an earlier statement
    // (e.g. another view in the same file) can never be mistaken for this view's winner ordering.
    let ob_rel = lower[header..].find("order by")?;
    let ob_abs = header + ob_rel;
    let end_rel = sql[ob_abs..].find(';')?;
    Some(normalize_ws(&sql[ob_abs..ob_abs + end_rel]))
}

/// TDD unit: the extractor isolates the RIGHT ORDER BY, tolerates whitespace reflow, and reflects
/// a semantic change (a dropped COLLATE pin) as a different string. Uses synthetic SQL so the
/// extractor's behaviour is pinned independently of the real migrations it will be pointed at.
#[test]
fn extractor_isolates_normalizes_and_detects_drift() {
    // An unrelated earlier view carries its OWN `ORDER BY x DESC` — the extractor must skip past
    // it to the `patient_name_current` winner ordering, never return the decoy.
    let canonical = "\
CREATE VIEW decoy AS SELECT 1 AS x ORDER BY x DESC;
CREATE OR REPLACE VIEW patient_name_current AS
SELECT DISTINCT ON (patient_id) patient_id, value
FROM patient_name
ORDER BY patient_id,
         value COLLATE \"C\" DESC;
";
    let got = winner_order_by(canonical).expect("winner ORDER BY present");
    assert_eq!(got, "ORDER BY patient_id, value COLLATE \"C\" DESC");
    assert!(
        !got.contains("x DESC"),
        "must not return the decoy view's ORDER BY"
    );

    // Same clause, reflowed onto one line with different spacing — normalization makes it equal.
    let reflowed = canonical.replace(
        "ORDER BY patient_id,\n         value COLLATE \"C\" DESC;",
        "ORDER BY patient_id,   value   COLLATE \"C\"   DESC;",
    );
    assert_eq!(
        winner_order_by(&reflowed).unwrap(),
        got,
        "a cosmetic reflow must NOT read as drift",
    );

    // Dropping the COLLATE "C" pin is exactly the #69 regression — it MUST change the result.
    let de_collated = canonical.replace("value COLLATE \"C\" DESC", "value DESC");
    assert_ne!(
        winner_order_by(&de_collated).unwrap(),
        got,
        "a dropped COLLATE \"C\" pin must read as drift",
    );

    assert_eq!(winner_order_by("-- no view here").as_deref(), None);
}

/// The guard (#159): db/012 and db/025 define `patient_name_current`'s winner ORDER BY twice, and
/// db/025 (loading last) is live. This asserts the two clauses are identical, so any future edit
/// to one that misses the other fails the build instead of silently diverging the display winner.
#[test]
fn db012_and_db025_winner_order_in_lockstep() {
    let a = winner_order_by(DB012).expect("db/012 defines patient_name_current + its ORDER BY");
    let b = winner_order_by(DB025).expect("db/025 re-defines patient_name_current + its ORDER BY");
    assert_eq!(
        a, b,
        "patient_name_current winner ORDER BY has DRIFTED between db/012 and db/025.\n\
         db/025's copy is the live one; a mismatch silently re-introduces the cross-node\n\
         projection divergence closed by #69/ADR-0045. Re-sync both clauses (keep every\n\
         COLLATE \"C\" pin), or update this guard if the winner rule genuinely changed.\n\
         db/012: {a}\n\
         db/025: {b}",
    );
}
