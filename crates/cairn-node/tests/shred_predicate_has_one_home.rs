//! Every place that DECIDES whether a shredded body's key may TRAVEL, named.
//!
//! Not style: this is the wire-level half of the crypto-shred guarantee, and before db/051
//! it had two spellings in two crates. A third would be silent — every caller keeps
//! working, and only the one that drifts stops filtering.
//!
//! # Two failed shapes before this one, and what each one taught
//!
//! **Cut 1** matched ANY non-comment line naming `erasure_shred_log`. That caught the two
//! real callers it was written for, but also 21 files of `TRUNCATE`/`INSERT`/`GRANT`
//! bookkeeping and row-count assertions — 12 of them identical copy-pasted "fixture
//! cleanup" reasons for medication test files added routinely in this build. A reviewer
//! facing a 13th near-identical precedent would reasonably rubber-stamp it in.
//!
//! **Cut 2** tried to fix that by enumerating the FILTERING idioms instead: `JOIN
//! erasure_shred_log`, or `NOT EXISTS`+`SELECT` in a one-line lookback window. That
//! shrank the list to 5 — but enumerating idioms is unbounded, and a scoped re-review
//! found two realistic rival spellings that slipped through undetected: `NOT IN (SELECT
//! target_event_id FROM erasure_shred_log)` (as common an anti-join idiom as `NOT
//! EXISTS`), and a bare `EXISTS (SELECT … erasure_shred_log …)` driving an inverted
//! branch (same decision, phrased without the leading `NOT`). A narrowed guard that
//! misses a real rival is worse than the loose one it replaced — it looks green while
//! proving nothing.
//!
//! # Cut 3 (this one): invert it
//!
//! Enumerating every way a filter CAN be spelled is unbounded — SQL always has another
//! idiom. Enumerating the ways the table is touched WITHOUT deciding anything is small
//! and closed: `TRUNCATE` it, `INSERT INTO` it, `DELETE FROM` it, `GRANT`/`REVOKE` on it,
//! declare it (`CREATE TABLE`/`CREATE INDEX`/`COMMENT ON`), or read it plainly (a `SELECT`
//! that names no OTHER table). Call that closed set BOOKKEEPING. Everything else that
//! mentions the table — a `SELECT` that also reaches a second table by ANY means
//! (`NOT EXISTS`, `EXISTS`, `IN`, `NOT IN`, a `JOIN` of any flavour, a CTE, an idiom
//! nobody has written yet), or a shape this guard does not recognize AT ALL — is a
//! candidate definition and must be named on the allow-list with its reason. An
//! unrecognized shape fails CLOSED (counts as a candidate), which is the correct
//! direction for a guard on a safety predicate: the failure mode of over-flagging is one
//! more line in a human-reviewed allow-list; the failure mode of under-flagging is a
//! silent rival that never gets caught.
//!
//! ## Statements, not lines
//!
//! A single definition can spread `NOT EXISTS (` and the `SELECT … erasure_shred_log`
//! it wraps across two, three, or more physical lines — db/051's own definition does
//! exactly this. Cut 2's one-line lookback window was fragile to that (it happened to
//! reach one line back, which covered db/051, but a three-line split would have sailed
//! through unnoticed — the re-review named this explicitly). So this cut does not use a
//! line window at all: it strips comments, then splits the WHOLE file on `;` into
//! statements. `;` is what actually ends a SQL statement — including one written inside a
//! plpgsql function body, which is still one `;` per internal statement regardless of the
//! `$$ … $$` wrapper around the whole function — and, for the queries this repo embeds as
//! Rust string literals, `;` is what ends the enclosing Rust expression, so a statement
//! spread across any number of line breaks lands in one chunk either way. The BOOKKEEPING
//! test then looks at which recognized command starts EARLIEST in that chunk (not which
//! one sits closest to the mention — `GRANT SELECT ON …, erasure_shred_log, …` has the
//! word "SELECT" sitting right next to the table, but `GRANT` governs the statement and
//! must win), and, for a `SELECT`, `DELETE FROM`, or `INSERT INTO` statement, counts how
//! many `FROM`/`JOIN` table-introductions it contains.
//!
//! ## A gap in this very design, caught by review before it shipped
//!
//! The first version of this cut gave EVERY non-`SELECT` command the unconditional
//! bookkeeping pass — including `DELETE FROM` and `INSERT INTO`. That is right for
//! `INSERT INTO erasure_shred_log … SELECT … FROM event_log …` (db/037's rebuild) and for
//! `GRANT`/`REVOKE` (privilege lists), but it is WRONG for `DELETE FROM event_dek WHERE
//! event_id IN (SELECT target_event_id FROM erasure_shred_log)` — purging custody rows
//! keyed on shred-log membership is a real decision about whether a key survives, arguably
//! more direct than the `SELECT`-filter shape this guard was built for, and the leading
//! `DELETE FROM` would have hidden it wholesale. So `DELETE FROM` and `INSERT INTO` get
//! the SAME `FROM`/`JOIN`-count test as `SELECT`; only `TRUNCATE`, `GRANT`, `REVOKE`,
//! `CREATE …`, and `COMMENT ON` keep the unconditional shortcut, because none of those can
//! carry a subquery that reads the shred log to decide about another row.
//!
//! NAME, NEVER COUNT (the house rule a count cannot satisfy: it cannot separate "one site
//! moved" from "one site added and one deleted"). The allow-list below is the inventory of
//! every legitimate mention, each with the reason it is not a second definition of the
//! travel predicate. When this fails, the panic message says which of two things
//! happened: a new CALLER (select from db/051 instead) or a genuinely new decision site
//! (name it here, with its reason, in the same commit).

#[path = "common/sources.rs"]
mod sources;

use std::path::Path;

/// (repo-relative file, why this mention is not a rival definition).
///
/// LEARNED, not guessed: this is the file list `cargo test -p cairn-node --test
/// shred_predicate_has_one_home` actually printed under `Found:` once the matcher was
/// inverted to "closed bookkeeping list, everything else is a candidate" (see the module
/// doc). Order matters: `found` is sorted, `ALLOWED` is compared unsorted, so this list is
/// kept in the same alphabetical order the scan produces.
const ALLOWED: &[(&str, &str)] = &[
    (
        "crates/cairn-node/src/restore/clinical.rs",
        "the restore's custody post-condition, and it is the db/005 / db/020 family rather \
         than a caller: NOT EXISTS(SELECT … erasure_shred_log …) decides whether the ABSENCE \
         of a custody row is a defect or the anti-resurrection rule working correctly. db/020 \
         has two LENIENT arms that admit a sealed record WITHOUT custody and return OK, so a \
         restore cannot read the door's return as proof the record came back; it asks the \
         database instead. A shredded target legitimately has no custody — penning it would \
         hold a record whose key was destroyed on purpose — so this decides about CREATION, \
         never about whether an existing key travels. `event_custody_surviving` cannot answer \
         it: that view is empty for BOTH the defect and the shred, and telling those two \
         apart is the entire question",
    ),
    (
        "crates/cairn-node/tests/dr_clinical_guarantee_gap.rs",
        "the ADR-0066/#500 guarantee-gap suite: stages a real `event_dek d JOIN \
         erasure_shred_log s` to verify the export's behaviour, and its assertion \
         messages narrate the guarantee in prose (no recognized SQL command on those \
         lines, so the guard fails closed on them too) — a consumer of the guarantee \
         throughout, never a second definition of it",
    ),
    (
        "crates/cairn-node/tests/seal_apply.rs",
        "the shred-execution test: one assertion message names `erasure_shred_log` in \
         plain English (\"still holds exactly one row\") with no SQL command on that \
         line, so the guard fails closed on it — it is narrating a row-count check on \
         cairn_execute_shred's own bookkeeping, not deciding whether a key travels",
    ),
    (
        "crates/cairn-node/tests/shred_predicate_has_one_home.rs",
        "this guard's own source: the allow-list reasons, the panic message and the \
         RIVAL_SPELLINGS/GENUINE_BOOKKEEPING injection set are DATA (not comments) that \
         name `SELECT`/`FROM`/`JOIN`/`USING`/`erasure_shred_log` together, and the \
         matching code itself contains those same keywords as string literals — \
         describing an idiom, and feeding it to the matcher, is not deciding anything \
         about a row",
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
        if mentions_a_candidate_definition(&text) {
            let rel = path.strip_prefix(&root).unwrap_or(Path::new("")).display();
            found.push(rel.to_string());
        }
    }
    found.sort();
    found.dedup();
    let allowed: Vec<String> = ALLOWED.iter().map(|(f, _)| (*f).to_string()).collect();
    if found != allowed {
        // Split the mismatch into the two situations a maintainer can actually be in,
        // rather than making them diff two `#[derive(Debug)]` dumps by eye.
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
             shred log — recording one, refusing to create a row, granting a privilege \
             — or a plain assertion message naming the table with no SQL nearby): add it \
             to ALLOWED above WITH the reason, in this same commit.\n\n\
             ON THE ALLOW-LIST BUT NOT FOUND, now stale — {removed:#?}\n\
             The mention that entry described is gone (moved, deleted, or now provably \
             bookkeeping) — delete its ALLOWED entry."
        );
    }
}

/// Does `text` contain a STATEMENT that mentions `erasure_shred_log` and is NOT
/// bookkeeping (see [`is_bookkeeping`])? This is the inverted check: everything that
/// isn't provably harmless bookkeeping counts, rather than trying to enumerate every
/// harmful shape.
fn mentions_a_candidate_definition(text: &str) -> bool {
    // Non-comment lines only, joined back into one blob so `;` splitting can cross line
    // breaks freely — a comment line's `;` (there are none in this codebase, but nothing
    // should rely on that) or its prose must never fracture a real statement.
    let code_text: String = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with("--") && !l.starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");

    // `;` is the statement terminator in both the SQL this codebase writes (top-level
    // AND inside a plpgsql function body — the `$$ … $$` wrapper does not stop an inner
    // statement from ending in its own `;`) and, for an embedded query string, in the
    // Rust expression that carries it. Splitting here — rather than a fixed-size line
    // window — is what makes the check robust to a statement spread over any number of
    // physical lines, which a prior cut of this guard got wrong (db/051's own two-line
    // `NOT EXISTS (` / `SELECT …` split was the one case it happened to still catch; a
    // three-line split would not have been).
    code_text
        .split(';')
        .any(|stmt| stmt.contains("erasure_shred_log") && !is_bookkeeping(stmt))
}

/// The commands to look for, checked by which of these starts EARLIEST in the statement
/// — not by proximity to the `erasure_shred_log` mention, which is what would let `GRANT
/// SELECT ON …, erasure_shred_log, …` fool a nearest-keyword check into reading the
/// privilege name "SELECT" as a query verb. Not all of these are unconditionally
/// bookkeeping: `SELECT`, `DELETE FROM`, and `INSERT INTO` are bookkeeping ONLY when the
/// statement names no other table (see [`is_bookkeeping`]) — a `DELETE`/`INSERT` can
/// carry a subquery that reads the shred log to decide about a DIFFERENT row, exactly
/// like a `SELECT` can, so the leading verb alone cannot clear them.
const RECOGNIZED_COMMANDS: &[&str] = &[
    "TRUNCATE",
    "INSERT INTO",
    "DELETE FROM",
    "GRANT",
    "REVOKE",
    "CREATE TABLE",
    "CREATE INDEX",
    "COMMENT ON",
    "SELECT",
];

/// Is `stmt` (a `;`-delimited chunk already known to mention `erasure_shred_log`)
/// BOOKKEEPING — reading or writing the ledger itself — rather than a FILTER deciding
/// something about a DIFFERENT row?
///
/// Two steps:
/// 1. Find which [`RECOGNIZED_COMMANDS`] entry starts EARLIEST in the statement. That is
///    the statement's real command, even when a later, unrelated occurrence of one of
///    the other keywords sits closer to the `erasure_shred_log` mention —
///    `GRANT SELECT ON event_dek, erasure_shred_log, …` has `GRANT` first and stays
///    bookkeeping despite `SELECT` sitting immediately before the table name. Finding no
///    recognized command at all (a plain-English assertion message that happens to name
///    the table, say) is NOT bookkeeping — an unrecognized shape fails CLOSED.
/// 2. `TRUNCATE`, `GRANT`, `REVOKE`, `CREATE …`, and `COMMENT ON` are bookkeeping
///    UNCONDITIONALLY once they win step 1 — `TRUNCATE …, erasure_shred_log, …` and
///    `REVOKE ALL ON …, erasure_shred_log, … FROM cairn_agent` each name several tables
///    and are still pure bookkeeping; none of these five can carry a subquery that reads
///    the shred log to decide about a different row, so the command alone is enough.
///
///    `SELECT`, `DELETE FROM`, and `INSERT INTO` get NO such free pass — a review caught
///    the first version of this guard giving `DELETE FROM`/`INSERT INTO` the
///    unconditional pass too, which hid `DELETE FROM event_dek WHERE event_id IN (SELECT
///    target_event_id FROM erasure_shred_log)` (purging custody keyed on shred-log
///    membership — a real decision) behind the leading `DELETE FROM`. All three of these
///    commands are bookkeeping ONLY when `erasure_shred_log` is the sole table the
///    statement touches: at most one `FROM`/`JOIN` table-introduction in the whole
///    statement (db/037's `INSERT INTO erasure_shred_log … SELECT … FROM event_log …`
///    rebuild stays bookkeeping under this same test — its one `FROM` names `event_log`,
///    not a second read of `erasure_shred_log`). Two or more means the statement combines
///    `erasure_shred_log` with something else — the definition of "decides something
///    about another row" — regardless of which relational operator wires them together
///    (`NOT EXISTS`, `EXISTS`, `IN`, `NOT IN`, any `JOIN` flavour, or an idiom nobody has
///    written yet).
fn is_bookkeeping(stmt: &str) -> bool {
    let command = RECOGNIZED_COMMANDS
        .iter()
        .filter_map(|kw| stmt.find(kw).map(|idx| (idx, *kw)))
        .min_by_key(|(idx, _)| *idx)
        .map(|(_, kw)| kw);
    match command {
        None => false, // fail closed: no recognized SQL shape at all
        // These three can carry a subquery reading erasure_shred_log to decide about a
        // DIFFERENT row, exactly like a SELECT can — the leading verb alone cannot clear
        // them; only "touches no other table" does.
        Some("SELECT") | Some("DELETE FROM") | Some("INSERT INTO") => {
            table_introduction_count(stmt) <= 1
        }
        Some(_) => true, // TRUNCATE / GRANT / REVOKE / CREATE … / COMMENT ON
    }
}

/// How many `FROM`/`JOIN`/`USING` table-introductions appear in a statement. Every SQL
/// join flavour (`LEFT JOIN`, `INNER JOIN`, a bare `JOIN`, …) puts the table name directly
/// after the word `JOIN`, so counting the bare keyword is sufficient without also having
/// to enumerate join flavours.
///
/// `USING` earns its place for one specific reason (#500 slice 2c final review, Minor 6):
/// Postgres' `DELETE … USING` is a THIRD way to introduce a table, and it is the natural
/// spelling of the very statement this guard exists to catch —
/// `DELETE FROM event_dek USING erasure_shred_log s WHERE d.event_id = s.target_event_id`
/// purges custody keyed on shred-log membership, a real decision about whether a key
/// survives. Without `USING` counted, that statement offers exactly ONE `FROM`
/// (`DELETE FROM`), clears the `<= 1` bookkeeping test, and passes as harmless ledger
/// maintenance — the same under-flagging the module doc calls the worse failure direction.
/// The `JOIN … USING (col)` form also matches, which merely double-counts a join that was
/// already caught; over-counting can only ever move a statement INTO the human-reviewed
/// allow-list, never out of it.
fn table_introduction_count(stmt: &str) -> usize {
    stmt.matches("FROM").count() + stmt.matches("JOIN").count() + stmt.matches("USING").count()
}

/// Rival spellings of the travel filter, each of which MUST be seen as a candidate
/// definition rather than as bookkeeping.
///
/// WHY AN INJECTION SET AT ALL. The scan test above is a whole-repo inventory: it passes
/// when `found == ALLOWED`, which is exactly as true of a guard that detects everything as
/// of one that detects nothing new. Narrowing `is_bookkeeping` by accident — the failure
/// that killed cuts 1 and 2 of this guard (see the module doc) — would leave that test
/// green and silent. So the shapes the guard claims to catch are written down and fed
/// through it directly. They are synthetic strings, never SQL this repo runs.
const RIVAL_SPELLINGS: &[(&str, &str)] = &[
    (
        "DELETE FROM event_dek d USING erasure_shred_log s WHERE d.event_id = s.target_event_id",
        "Postgres' DELETE … USING join — the spelling Minor 6 found slipping through on a \
         single FROM",
    ),
    (
        "DELETE FROM event_dek WHERE event_id IN (SELECT target_event_id FROM erasure_shred_log)",
        "the anti-join purge the module doc's 'gap caught by review' section names",
    ),
    (
        "SELECT d.dek_wrapped FROM event_dek d WHERE NOT EXISTS (SELECT 1 FROM \
         erasure_shred_log s WHERE s.target_event_id = d.event_id)",
        "db/051's own NOT EXISTS shape — the definition, restated elsewhere",
    ),
    (
        "SELECT d.dek_wrapped FROM event_dek d LEFT JOIN erasure_shred_log s ON \
         s.target_event_id = d.event_id WHERE s.target_event_id IS NULL",
        "cairn-sync's historical LEFT JOIN shape",
    ),
    (
        "SELECT dek_wrapped FROM event_dek WHERE event_id NOT IN (SELECT target_event_id \
         FROM erasure_shred_log)",
        "NOT IN — as common an anti-join idiom as NOT EXISTS, and cut 2 missed it",
    ),
    (
        "SELECT dek_wrapped FROM event_dek d WHERE EXISTS (SELECT 1 FROM erasure_shred_log \
         s WHERE s.target_event_id = d.event_id)",
        "a bare EXISTS driving an inverted branch — the same decision without a leading NOT",
    ),
];

/// Statements that touch the ledger and decide NOTHING about another row. Kept beside the
/// rivals so a future tightening cannot quietly make the guard flag everything and call
/// that safety: a guard that fails on `TRUNCATE` teaches maintainers to edit the
/// allow-list reflexively, which is how a real rival gets waved through.
const GENUINE_BOOKKEEPING: &[(&str, &str)] = &[
    (
        "TRUNCATE event_log, erasure_shred_log, event_dek CASCADE",
        "fixture cleanup naming several tables",
    ),
    (
        "REVOKE ALL ON event_dek, erasure_shred_log FROM cairn_agent",
        "a privilege list; GRANT/REVOKE wins over the word SELECT sitting nearby",
    ),
    (
        "INSERT INTO erasure_shred_log (target_event_id) SELECT event_id FROM event_log \
         WHERE payload IS NULL",
        "db/037's rebuild: its one FROM names event_log, not a second read of the ledger",
    ),
    (
        "SELECT count(*) FROM erasure_shred_log",
        "a plain row-count read of the ledger itself",
    ),
];

#[test]
fn the_guard_actually_catches_every_rival_spelling_it_claims_to() {
    for (sql, why) in RIVAL_SPELLINGS {
        assert!(
            mentions_a_candidate_definition(sql),
            "this guard must see `{sql}` as a candidate definition ({why}) — it decides \
             whether a shredded body's key travels, and a spelling the guard cannot see is \
             a silent second home for the predicate"
        );
    }
    for (sql, why) in GENUINE_BOOKKEEPING {
        assert!(
            !mentions_a_candidate_definition(sql),
            "this guard must NOT flag `{sql}` ({why}) — over-flagging pure bookkeeping is \
             how the allow-list becomes a rubber stamp, which is how a real rival gets in"
        );
    }
}
