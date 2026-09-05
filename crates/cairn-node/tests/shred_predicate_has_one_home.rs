//! Every place that DECIDES whether a shredded body's key may TRAVEL, named.
//!
//! Not style: this is the wire-level half of the crypto-shred guarantee, and before db/051
//! it had two spellings in two crates. A third would be silent — every caller keeps
//! working, and only the one that drifts stops filtering.
//!
//! # What counts as a mention — narrower than "the string appears somewhere"
//!
//! The first cut of this guard matched ANY non-comment line naming `erasure_shred_log`.
//! That caught the two genuine callers it was written to catch, but it ALSO caught every
//! `TRUNCATE erasure_shred_log` fixture reset, every `INSERT`/`DELETE`/`GRANT`/`REVOKE`
//! bookkeeping line, and every `SELECT count(*) FROM erasure_shred_log WHERE …` test
//! assertion — 23 files, 12 of them medication test files carrying the IDENTICAL
//! copy-pasted "TRUNCATE the table between tests" reason (itself the mirror-list pattern
//! db/051 exists to end, #182/#404/#441). New medication test files land routinely in
//! this slice-by-slice build, each with its own such line; a reviewer facing a 13th
//! near-identical precedent would reasonably rubber-stamp it in, and the guard's real
//! signal — a genuinely new way of deciding whether a key travels — would drown.
//!
//! So the check below matches only a FILTERING construct: the two idioms every real
//! definition or caller of the travel predicate actually uses to turn `erasure_shred_log`
//! into an exclusion over ANOTHER table's rows.
//!
//! (a) `JOIN erasure_shred_log` (`LEFT JOIN`, plain `JOIN`, …) — the table is joined
//!     against something else to decide something about ITS rows (cairn-sync's serve
//!     door, before Task 3; `dr_clinical_guarantee_gap.rs` stages the same shape to
//!     verify the guarantee).
//! (b) `NOT EXISTS (SELECT … erasure_shred_log …)` — an existence subquery deciding
//!     whether to admit or create a row elsewhere (db/005, db/020, and db/051's own
//!     definition). Genuine sites split this across two lines as often as one — db/051
//!     itself writes `WHERE NOT EXISTS (` on one line and the `SELECT … FROM
//!     erasure_shred_log` on the next — so the check looks at a CODE-LINE window (the
//!     mention line plus the nearest non-comment line before it) rather than a single
//!     line. Requiring `SELECT` in that window is what tells a real filter apart from
//!     `CREATE TABLE IF NOT EXISTS erasure_shred_log` (db/037's table declaration, which
//!     also contains the literal substring "NOT EXISTS" but reads no row and decides
//!     nothing about any other table).
//!
//! A bare `TRUNCATE`/`INSERT`/`DELETE`/`GRANT`/`REVOKE`/`CREATE TABLE` mention, or a
//! `SELECT … FROM erasure_shred_log` with no `JOIN` (a plain row-count assertion), does
//! NOT match: those read or write the ledger itself, they never turn it into a filter
//! over something else.
//!
//! # `tests/` stays IN SCOPE, deliberately
//!
//! Some of this crate's other source-guards pass `"tests"` in `skip_dirs` (see
//! `source_walk_shared.rs`) because their subject is production shape, not test content.
//! This guard does not, on purpose: what used to drown it in `tests/` was the mechanical
//! fixture noise above, and the tightened matcher already silences that on its own — a
//! new medication test's `TRUNCATE … erasure_shred_log CASCADE` will never trip it again.
//! What is left in `tests/` after tightening is rare and exactly the shape worth a human
//! eyeball every time: `dr_clinical_guarantee_gap.rs` stages a genuine `JOIN
//! erasure_shred_log` to verify the export's guarantee, which is precisely what a rival,
//! silently-drifting reimplementation of the travel filter would also look like if
//! someone wrote one in a test instead of production code. Excluding `tests/` would
//! blind the guard to that case, so it stays scanned like everything else.
//!
//! NAME, NEVER COUNT (the house rule a count cannot satisfy: it cannot separate "one site
//! moved" from "one site added and one deleted"). The allow-list below is the inventory of
//! every legitimate filtering mention, each with the reason it is not a second definition
//! of the travel predicate. When this fails, the panic message says which of two things
//! happened: a new CALLER (select from db/051 instead) or a genuinely new decision site
//! (name it here, with its reason, in the same commit).

#[path = "common/sources.rs"]
mod sources;

use std::path::Path;

/// (repo-relative file, why this mention is not a rival definition).
///
/// LEARNED, not guessed: this is the file list `cargo test -p cairn-node --test
/// shred_predicate_has_one_home` actually printed under `Found:` once the matcher below
/// was tightened to filtering constructs only (see the module doc for why). Order
/// matters: `found` is sorted, `ALLOWED` is compared unsorted, so this list is kept in
/// the same alphabetical order the scan produces.
const ALLOWED: &[(&str, &str)] = &[
    (
        "crates/cairn-node/tests/dr_clinical_guarantee_gap.rs",
        "the ADR-0066/#500 guarantee-gap suite stages a real `JOIN erasure_shred_log` to \
         verify the export's behaviour (a shredded body's DEK never reaches LocalState) \
         — a consumer of the guarantee that exercises the same join shape, never a \
         second definition of it",
    ),
    (
        "crates/cairn-node/tests/shred_predicate_has_one_home.rs",
        "this guard's own source: `JOIN erasure_shred_log` appears literally inside its \
         OWN matching code, and `NOT EXISTS`/`SELECT`/`erasure_shred_log` co-occur inside \
         its allow-list reason strings (this very file, describing db/005 and db/020) — \
         naming the idiom is not deciding anything about a row",
    ),
    (
        "db/005_submit.sql",
        "the LOCAL submit door's own anti-resurrection check (same shape as db/020's \
         remote one): NOT EXISTS(SELECT … erasure_shred_log …) decides whether to CREATE \
         a custody row for an already-shredded target on first write, not whether an \
         existing key travels",
    ),
    (
        "db/020_apply_remote_event.sql",
        "the remote apply door's anti-resurrection check: NOT EXISTS(SELECT … \
         erasure_shred_log …) decides whether to CREATE a custody row for an \
         already-shredded target, not whether an existing key travels",
    ),
    (
        "db/051_clinical_capture_source.sql",
        "THE definition: event_custody_surviving is the one filter every caller inherits",
    ),
];

#[test]
fn the_travel_filter_has_one_definition_and_every_other_mention_is_named() {
    let root = sources::repo_root();
    let roots = vec![root.join("db"), root.join("crates")];
    let mut found: Vec<String> = Vec::new();
    for path in sources::source_files(&roots, &["target"], &["sql", "rs"]) {
        let text = sources::read_source(&path);
        if mentions_a_filtering_construct(&text) {
            let rel = path.strip_prefix(&root).unwrap_or(Path::new("")).display();
            found.push(rel.to_string());
        }
    }
    found.sort();
    found.dedup();
    let allowed: Vec<String> = ALLOWED.iter().map(|(f, _)| (*f).to_string()).collect();
    if found != allowed {
        // Split the mismatch into the two situations a maintainer can actually be in,
        // rather than making them diff two `#[derive(Debug)]` dumps by eye — that split
        // IS the fix a prior review asked for (a bare assert_eq! only says "these
        // differ", not what to do about it).
        let added: Vec<&String> = found.iter().filter(|f| !allowed.contains(f)).collect();
        let removed: Vec<&String> = allowed.iter().filter(|f| !found.contains(f)).collect();
        panic!(
            "the inventory of files deciding anything about erasure_shred_log's travel \
             filter has changed.\n\n\
             Found:   {found:#?}\n\
             Allowed: {allowed:#?}\n\n\
             NEWLY FOUND, not on the allow-list — {added:#?}\n\
             For EACH one, decide which situation you are in:\n  \
             - It is a CALLER (it wants to know whether a body survived shredding): \
             delete the new filter and select from `event_custody_surviving` / \
             `cairn_clinical_page` (db/051) instead. Do not add it here.\n  \
             - It is a genuinely NEW decision site (it decides something else about the \
             shred log — recording one, refusing to create a row, granting a privilege): \
             add it to ALLOWED above WITH the reason, in this same commit.\n\n\
             ON THE ALLOW-LIST BUT NOT FOUND, now stale — {removed:#?}\n\
             The filtering mention that entry described is gone (moved, deleted, or no \
             longer a JOIN/NOT EXISTS shape) — delete its ALLOWED entry."
        );
    }
}

/// Does `text` contain a line that turns `erasure_shred_log` into a FILTER over some
/// other table's rows — as opposed to merely reading or writing the ledger itself?
///
/// See the module doc for the full "why these two idioms, why not just the substring"
/// argument. In short: real definitions/callers of the travel predicate always either
/// JOIN the table against another one, or wrap it in a `NOT EXISTS(SELECT …)` existence
/// check; a `TRUNCATE`/`INSERT`/`DELETE`/`GRANT`/`REVOKE`/`CREATE TABLE` line, or a bare
/// `SELECT … FROM erasure_shred_log` with no join, is bookkeeping on the ledger, not a
/// decision about anything else.
fn mentions_a_filtering_construct(text: &str) -> bool {
    // Non-comment lines only, trimmed, in file order. Kept as a `Vec` (not just an
    // iterator) because idiom (b) below needs to look at the PREVIOUS entry in this same
    // filtered sequence — comments in between must not count as "the line before".
    let code_lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with("--") && !l.starts_with("//"))
        .collect();

    code_lines.iter().enumerate().any(|(i, line)| {
        if !line.contains("erasure_shred_log") {
            return false;
        }
        // (a) A join directly against the table. Every SQL join flavour keeps the
        // keyword and the table name on the SAME line in this codebase (checked against
        // every site this guard has ever found), so no window is needed here.
        if line.contains("JOIN erasure_shred_log") {
            return true;
        }
        // (b) NOT EXISTS(SELECT … erasure_shred_log …), which db/051 itself splits
        // across two lines (`WHERE NOT EXISTS (` then `SELECT 1 FROM
        // erasure_shred_log …` on the next). Look at this line plus the ONE non-comment
        // line before it. Requiring `SELECT` in that window is what excludes `CREATE
        // TABLE IF NOT EXISTS erasure_shred_log` — that phrase contains "NOT EXISTS" too,
        // but declares the table rather than querying it, and no `SELECT` sits nearby.
        let window = match i.checked_sub(1) {
            Some(prev) => format!("{} {line}", code_lines[prev]),
            None => (*line).to_string(),
        };
        window.contains("NOT EXISTS") && window.contains("SELECT")
    })
}
