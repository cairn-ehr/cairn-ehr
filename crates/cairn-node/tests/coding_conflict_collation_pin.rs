//! #295 — the `medication_group_coding_conflict` anchor comparison must stay
//! `COLLATE "C"`-pinned, in BOTH the select-list count and the `HAVING` count.
//!
//! THE HAZARD (the ADR-0045 class). The view flags a possible mis-reconciliation with
//! `HAVING count(DISTINCT <flattened anchor>) > 1`. Under a NON-DETERMINISTIC collation —
//! an ICU case-insensitive default, which a deployment is free to choose — two anchors
//! differing only in case compare EQUAL, the count collapses to 1, and the conflict row
//! never appears even though `anchors` would have listed both. The flag would then depend
//! on a node-LOCAL collation property: two honest nodes replaying the same events would
//! disagree about whether a group is in conflict, and the failure direction is the bad one
//! (a mis-reconciliation signal silently dropped). It is reachable in practice, because the
//! canonical-uuid pin guards only the STRICT door and the registry-derived tier is
//! deliberately lenient on remote apply (ADR-0051/0056) — a peer may hand us `0F8C…` for a
//! moiety we hold as `0f8c…`.
//!
//! WHY A SOURCE GUARD AND NOT A BEHAVIOURAL ONE. `medication_coding.rs`'s
//! `the_anchor_conflict_count_is_collation_pinned` proves the hazard is real (a scratch
//! non-deterministic ICU collation collapses the unpinned form to 1 while the pinned form
//! returns 2) and proves the live view flags case-differing anchors. But it CANNOT catch a
//! future unpinning: the test cluster's default collation is deterministic, so an unpinned
//! view passes it — verified by deliberately unpinning db/033 and watching it stay green.
//! Only a source-level pin closes the issue, and it needs no database, so it runs in every
//! `cargo test` and CI pass. Same idiom, same reasoning as `name_winner_order_drift.rs`.

/// The migration is `include_str!`-embedded at compile time (the same way `src/db.rs`
/// embeds it), so this guard reads the shipped text directly.
const DB033: &str = include_str!("../../../db/033_medication_reconciliation.sql");

const VIEW_HEADER: &str = "CREATE OR REPLACE VIEW medication_group_coding_conflict AS";

/// The executable body of the anchor-conflict view: from its `CREATE OR REPLACE VIEW`
/// header to the statement-terminating `;`. Scoped deliberately — the file defines several
/// other views, and a `count(DISTINCT …)` in one of them is not this invariant.
fn conflict_view_body() -> &'static str {
    let start = DB033
        .find(VIEW_HEADER)
        .expect("db/033 must define medication_group_coding_conflict");
    let rest = &DB033[start..];
    let end = rest
        .find(';')
        .expect("the view definition must be terminated");
    &rest[..=end]
}

#[test]
fn every_anchor_count_in_the_conflict_view_is_collation_pinned() {
    let body = conflict_view_body();

    // Both counts — the reported anchor_count and the HAVING that decides whether the row
    // exists at all. Two, not "at least one": pinning only the select-list count would
    // still let the HAVING silently swallow the conflict.
    let pinned = body
        .matches(r#"count(DISTINCT (mc.coding_system || '|' || mc.coding_code) COLLATE "C")"#)
        .count();
    assert_eq!(
        pinned, 2,
        "expected the select-list count AND the HAVING count to be COLLATE \"C\"-pinned; \
         found {pinned} pinned occurrence(s) in:\n{body}"
    );

    // And nothing else counts anchors unpinned — catches a future THIRD count, or a
    // reformatting that keeps two pinned forms while adding an unpinned one.
    let total = body.matches("count(DISTINCT").count();
    assert_eq!(
        total, pinned,
        "an unpinned count(DISTINCT …) appeared in the anchor-conflict view: under a \
         non-deterministic default collation it compares two case-differing anchors as \
         EQUAL and the mis-reconciliation signal is silently lost (#295, ADR-0045):\n{body}"
    );

    // The array_agg beside them must stay pinned too — it feeds the human-readable
    // `anchors` list, and an unpinned ORDER BY there makes the listing itself
    // collation-dependent (two honest nodes rendering the same conflict differently).
    assert!(
        body.contains(
            r#"array_agg(DISTINCT (mc.coding_system || '|' || mc.coding_code) COLLATE "C""#
        ),
        "the anchors array_agg must stay COLLATE \"C\"-pinned:\n{body}"
    );
}
