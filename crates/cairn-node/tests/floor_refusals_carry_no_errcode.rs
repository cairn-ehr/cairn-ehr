//! The in-DB floor's refusals must stay bare `RAISE EXCEPTION` — SQLSTATE `P0001` (#633).
//!
//! # What rests on this
//!
//! Three separate call sites classify a failed database call as *a deliberate refusal* versus
//! *an accident that befell the call*, and all three decide it by asking whether the SQLSTATE
//! is `P0001`:
//!
//! - `cairn_sync`'s `refusal_is_deliberate` — the sync pull loop, which quarantines a refused
//!   event rather than retrying it forever.
//! - `cairn_node::restore::clinical::refusal_is_deliberate` — the restore path.
//! - `cairn_gui_live::error::refusal_is_deliberate` — the funnel's ports, where the answer
//!   decides whether a clerk is offered a RETRY BUTTON (#648).
//!
//! That works only because every refusal in `db/*.sql` is raised bare. PL/pgSQL assigns
//! `P0001` to a bare `RAISE EXCEPTION`; adding `USING ERRCODE = …` replaces it with something
//! else, and the refusal instantly reads as an outage everywhere.
//!
//! # Why a test and not a comment
//!
//! The rule was, until now, stated **per door in prose**: `db/001_envelope.sql` says it above
//! `cairn_decode_hex_or_raise`, `db/048_sensitivity_stream.sql` says it for the clinical apply
//! door. Neither covers the rest of the tree, and nothing enforced either. Meanwhile
//! `cairn-sync`'s own doc asserts the general fact — *"no `USING ERRCODE` appears anywhere in
//! `db/`"* — as a premise for its routing.
//!
//! So the contract held by luck and vigilance. A well-meaning `USING ERRCODE = '22023'` on any
//! of db/045's ten registration refusals would turn a clerk's verdict into a retry button,
//! with nothing red anywhere: every existing test asserts the *message*, and the message would
//! be unchanged. This test is what makes the prose true of every file at once.
//!
//! # If this test fails
//!
//! Do not add an allow-list entry reflexively. The question to answer first is *which of the
//! three classifiers should now treat this new code as a verdict* — and that is
//! [#655](https://github.com/cairn-ehr/cairn-ehr/issues/655)'s subject (the `false` half of
//! `refusal_is_deliberate` is not one thing). A deliberate exception belongs in `ALLOWED`
//! below **with its reason**, next to the classifier change that understands it.
use std::fs;
use std::path::PathBuf;

#[path = "common/sources.rs"]
mod sources;
#[path = "common/sql_text.rs"]
mod sql_text;

/// `db/`, via the shared `repo_root()` so this test passes under `cargo test` from anywhere in
/// the workspace and cannot disagree with the other catalogue guards about where the tree is.
fn db_dir() -> PathBuf {
    sources::repo_root().join("db")
}

/// Deliberate exceptions, as (file name, reason). Empty today, and that is the finding: the
/// contract currently holds with no exceptions at all.
///
/// A row here is a promise that every `refusal_is_deliberate` caller has been taught about the
/// code in question. Adding one without doing that is the defect this file exists to prevent.
const ALLOWED: &[(&str, &str)] = &[];

/// Every `*.sql` directly under `db/`, as (file name, CODE with every SQL comment stripped).
///
/// Comment-stripping is load-bearing, not tidy: `db/001` and `db/048` both *discuss* `USING
/// ERRCODE` at length in order to forbid it, so a scan that read prose would fail on the very
/// files that state the rule — the guard would be unable to pass and would be deleted.
///
/// `sql_text::without_sql_comments` rather than a local line filter, which is what #633 asked
/// for and what the house already learned: *"a stripper written twice is the drift #608 was made
/// of."* It also handles what a line filter cannot — `/* ... */` blocks, which nest in Postgres,
/// and a trailing `--` after real code on the same line.
///
/// `read_dir` is deliberately non-recursive: `db/tests/` holds SQL mirrors that legitimately
/// inject faults with an explicit errcode — that is how they simulate an outage — and they are
/// not the floor.
fn migrations() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for entry in fs::read_dir(db_dir()).expect("read db/") {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("sql") {
            continue;
        }
        let sql = fs::read_to_string(&path).expect("read sql");
        out.push((
            path.file_name().unwrap().to_string_lossy().into_owned(),
            sql_text::without_sql_comments(&sql),
        ));
    }
    out.sort();
    out
}

/// No migration may set an explicit SQLSTATE on a raise. **Pure**, given the files.
#[test]
fn no_migration_overrides_the_refusal_sqlstate() {
    let mut offenders: Vec<String> = Vec::new();

    for (name, code) in migrations() {
        if ALLOWED.iter().any(|(allowed, _)| *allowed == name) {
            continue;
        }
        // Case-insensitive, and tolerant of the whitespace PL/pgSQL allows between the
        // keywords, so `using errcode=` and `USING   ERRCODE  =` are both caught. Matching the
        // two keywords rather than the whole `RAISE … USING ERRCODE` statement is deliberate:
        // the statement can span lines, and there is no legitimate other use of the pair.
        // Whitespace is squashed across the WHOLE file rather than per line, because `RAISE …
        // USING ERRCODE` may be split over two lines and a per-line scan would miss exactly the
        // formatting a reviewer is least likely to notice. The cost is that a line number cannot
        // be reported — `without_sql_comments` collapses comments to a single space, so offsets
        // no longer map to the source — so the file name is named and the reader greps.
        let squashed: String = code
            .to_ascii_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if squashed.contains("using errcode") {
            offenders.push(name);
        }
    }

    assert!(
        offenders.is_empty(),
        "these db/*.sql files set an explicit SQLSTATE on a raise: {offenders:?}\n\n\
         Every refusal in the floor must be a BARE `RAISE EXCEPTION` (P0001). Three \
         classifiers route on that — cairn-sync's pull loop, cairn-node's restore path, and \
         cairn-gui-live's funnel ports, where it decides whether a clerk is shown a retry \
         button for a verdict they can never retry past (#648). An explicit errcode here makes \
         that refusal read as an OUTAGE everywhere, silently, with its message unchanged.\n\n\
         If the override is intended, teach every classifier about the new code first (#655), \
         then add the file to `ALLOWED` with the reason."
    );
}

/// The guard must be able to SEE a violation. A scan that silently matched nothing — a wrong
/// `db/` path, an extension filter that excluded everything, comment-stripping that ate the
/// code — would pass the test above forever while enforcing nothing.
///
/// This is the lesson of 2026-08-19 applied in miniature: a guard defined over the list it
/// guards is not a guard. So: assert the corpus is non-trivial, and assert the predicate fires
/// on a line that genuinely contains the thing.
#[test]
fn the_scan_actually_reads_the_migrations_and_can_detect_a_violation() {
    let files = migrations();
    assert!(
        files.len() > 30,
        "only {} db/*.sql files found — the scan is looking in the wrong place, and the guard \
         above is passing vacuously",
        files.len()
    );
    assert!(
        files.iter().any(|(n, _)| n == "001_envelope.sql"),
        "db/001_envelope.sql must be in the corpus; got {:?}",
        files.iter().map(|(n, _)| n).collect::<Vec<_>>()
    );

    // Every migration must still have CODE after comment-stripping, or the strip is too greedy.
    for (name, code) in &files {
        assert!(
            !code.trim().is_empty(),
            "{name} has no code left after comment-stripping — the filter is eating real lines"
        );
    }

    // And the predicate itself, on the exact shapes PL/pgSQL accepts.
    for probe in [
        "RAISE EXCEPTION 'x' USING ERRCODE = '22023';",
        "raise exception 'x' using errcode='22023';",
        "  RAISE EXCEPTION 'x'  USING   ERRCODE  = '22023';",
    ] {
        let squashed: String = probe
            .to_ascii_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            squashed.contains("using errcode"),
            "the predicate must catch {probe:?}"
        );
    }

    // A bare raise — the shape the whole floor uses — must NOT match.
    let bare: String = "RAISE EXCEPTION 'a legible refusal';"
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        !bare.contains("using errcode"),
        "a bare RAISE EXCEPTION is the CORRECT shape and must not be flagged"
    );
}

/// The prose this test generalises must still be there.
///
/// If somebody deletes db/001's statement of the contract, the reason for this test disappears
/// from the place a reader of the SQL would look for it. Pinning the prose keeps the *why*
/// next to the code, which is the house rule; pinning it HERE rather than in db/001 is what
/// makes deleting it fail.
#[test]
fn the_per_door_prose_that_states_the_contract_is_still_present() {
    let envelope = fs::read_to_string(db_dir().join("001_envelope.sql")).expect("db/001");
    assert!(
        envelope.contains("USING ERRCODE"),
        "db/001_envelope.sql no longer discusses the USING ERRCODE contract — three crates \
         route on it (see this file's module doc), so the rule needs to stay stated where the \
         floor's own readers will find it"
    );
}
