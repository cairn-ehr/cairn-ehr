//! #467 stays fixed: no DB error in this crate may reach an operator as its own `Display`.
//!
//! # Why a source scan rather than more behavioural tests
//!
//! PR #472 rerouted every `tokio_postgres::Error` rendering in
//! `crates/cairn-node/src/db.rs` through `db_diagnosis::legible_db_error`, and two of them
//! are pinned behaviourally by `tests/db_diagnosis.rs` (the migration door and the connect
//! door). The rest are the same one-line composition, and writing that many more
//! near-identical DB-gated tests would buy little. (This paragraph used to hardcode the
//! counts, which this sweep's own new site immediately made wrong — PR #478 review, I9.)
//!
//! What that leaves unguarded is not any individual site but the **class**: the NEXT
//! site, written six months from now by someone who has never read #467, as the
//! `anyhow!("…: {e}")` that every other Rust codebase writes without thinking. It would be
//! correct-looking, would pass every test in the tree, and would put `db error` back in
//! front of an operator. The CI line that filed #467 —
//! `loading 031_medication: db error` — was exactly that shape.
//!
//! So this guard asserts a property of the FILE, which is the only thing that can catch a
//! site nobody has written yet.
//!
//! # What counts as an offender
//!
//! Any interpolation of a bare binding named `e`/`err`/`error` inside a string in a guarded
//! file — the `{e}` / `{err}` / `{error}` shapes. The legitimate form is `{}` fed by
//! `legible_db_error(&e)`, which is why the check is on the INTERPOLATION and not on the
//! word.
//!
//! **Comment lines are skipped**, and that is not a loophole: a comment renders nothing to
//! an operator, and the three files here explain the defect by NAMING the shape that caused
//! it. A guard that punished the most precise available description of the bug it protects
//! against would push every future writer toward vaguer prose — which is the opposite of
//! what #467 needs. (The skip is deliberately narrow: only a line whose first non-blank
//! characters are `//`. A trailing comment after code is still scanned, which errs in the
//! safe direction. The residual hole is a line INSIDE a multi-line string literal that
//! happens to begin with `//`; no such line exists in this crate, and SQL — the only
//! multi-line literal here — comments with `--`.)
//!
//! # Scope, stated so coverage is not confused with aspiration
//!
//! Three files in `cairn-node`: `db.rs` (the file #467 was filed against, and the one that
//! fails first on a fresh node), `safety.rs` (#473 — the clinical write path) and `sync.rs`
//! (#474 — the daemon loop). They are the three files those issues were filed against, and
//! **not** every production file in this crate that talks to the database: **28** files
//! under `crates/cairn-node/src/` execute SQL. `auto_apply.rs:304`/`:324` are live
//! offenders today — `db error` on the §5.7 identity auto-apply ceremony — and roughly 24
//! further raw `?` sites elsewhere name no operation. All of it is tracked as **#477**.
//!
//! An earlier draft of this paragraph claimed the three files WERE the whole set. That
//! claim was false, contradicted by this repo's own HANDOVER, and worse than the honest
//! gap it replaced: a reader who believes the crate is covered never widens the guard
//! (PR #478 review, finding 3). Add a file to `GUARDED` when its sites are fixed — each
//! one converted is a durable ratchet.
//!
//! `cairn-sync`'s `main.rs` is deliberately NOT here. It carries the twin renderer and its
//! own loops were fixed alongside these (#475, #471), but it is one 9,000-line file mixing
//! production code with its own test modules, and dozens of its `{e}`-shaped sites render
//! errors that are not database errors at all — hex decoding, serde, I/O, and `ApplyError`,
//! which is legible by construction. A name-based scan over that file would be mostly false
//! positives, and a guard whose failures are usually noise is one people learn to silence.
//! Splitting `main.rs` is separate work (#402's shape); until then the crate's DB-error
//! legibility rests on its own tests rather than on a scan.
//!
//! # When a site here IS a false positive
//!
//! The predicate is name-based, because a source scan cannot type-check. A binding that
//! genuinely does not hold a database error — an `io::Error` from `accept`, a serde failure
//! — is resolved by NAMING it (`accept_err`), never by suppressing the check. The rename is
//! only acceptable when the new name says what the value IS: that leaves the source more
//! informative than `e` was, which is what makes it a fix rather than a dodge.
//!
//! **A rename is not, by itself, proof.** The first version of this widening renamed five
//! bindings in `sync.rs`, and two of them (`session_err`, `pull_err`) genuinely DID hold
//! database errors on some branches — so the guard reported green over two live instances
//! of the defect it had just been widened to catch (PR #478 review, findings 1, 2 and 9).
//! Both now render through `db_diagnosis::operator_chain`, which walks the whole `anyhow`
//! chain. Before renaming a binding, establish that every branch reaching it is
//! non-database; if any branch is, render the chain instead.
//!
//! The predicate cannot see two further shapes, stated so they are not mistaken for
//! coverage: the positional `format!("…: {}", e)` form, and any renamed binding at all.
//! That is why `sync.rs`'s `LocalDbFault` discipline is pinned by COUNT below rather than
//! by the interpolation scan.

#[path = "common/sources.rs"]
mod sources;

/// The files whose DB errors must stay legible. See the module doc for which files these
/// are and why `cairn-sync`'s `main.rs` is not among them.
const GUARDED: &[&str] = &[
    "crates/cairn-node/src/db.rs",
    "crates/cairn-node/src/safety.rs",
    "crates/cairn-node/src/sync.rs",
];

/// Bindings that, interpolated raw, render a `tokio_postgres::Error` as its useless kind.
const RAW_ERROR_BINDINGS: &[&str] = &["e", "err", "error"];

/// Is this line a comment, and therefore incapable of rendering anything to an operator?
///
/// Only a line whose first non-blank characters are `//` — which covers `//`, `///` and
/// `//!`. A trailing comment after real code is deliberately NOT excluded: that line still
/// contains code, and erring toward scanning it is the safe direction for a guard.
///
/// This exists because the widening pass found the guard's only three "offenders" were
/// comments EXPLAINING the defect, quoting the exact shape that caused it. See the module
/// doc for why naming the shape is worth protecting.
fn is_a_comment_line(line: &str) -> bool {
    line.trim_start().starts_with("//")
}

/// Does this line interpolate one of the raw error bindings into a string?
///
/// Deliberately simple and slightly over-eager on CODE: it is a guard, and a false positive
/// costs one `legible_db_error` call or one rename that names the error's kind, while a
/// false negative costs an operator their diagnosis. Pure, so the judgement is testable
/// without touching disk.
fn interpolates_a_raw_error(line: &str) -> bool {
    if is_a_comment_line(line) {
        return false;
    }
    RAW_ERROR_BINDINGS
        .iter()
        .any(|b| line.contains(&format!("{{{b}}}")))
}

/// Every `sync.rs` postgres call that PROPAGATES is wrapped in `LocalDbFault`.
///
/// Twelve is not a magic number — it is the count of postgres calls in `sync.rs` that
/// return their error to a caller. (The thirteenth, `SELECT apply_remote_node_event`, is
/// matched inline and never propagates, so it has no wrapper to lose.) Bump it
/// deliberately when a query is added or removed, exactly as `twin_registry.rs` and
/// `db/tests/034` are bumped.
const SYNC_LOCAL_DB_FAULT_SITES: usize = 12;

/// The `LocalDbFault` discipline in `sync.rs`, pinned by count (PR #478 review, finding 8).
///
/// # Why a count, when the file already states the rule in a doc comment
///
/// Reverting any one `map_err(|e| LocalDbFault::new(…))` to `.context(…)` compiles, leaves
/// the interpolation scan above green (no `{e}` appears), and leaves
/// `tests/pull_failure_class.rs` green (it builds its own `LocalDbFault` and never asserts
/// that a production site produces one). Yet it reinstates BOTH defects at once: the line
/// loses its SQLSTATE, and — because `pull_failure_class` walks the chain looking for a
/// `tokio_postgres::Error` that `.context()` leaves in place but `anyhow!` does not — the
/// discipline that keeps the chain intact stops being verifiable at all. That is issue
/// #474 item 3's machinery, unpinned.
///
/// A count is crude, and it is the only thing that catches site 13 written six months from
/// now — the same argument the interpolation scan above makes for itself.
#[test]
fn every_propagating_postgres_call_in_sync_is_wrapped_in_a_local_db_fault() {
    let root = sources::repo_root();
    let text = std::fs::read_to_string(root.join("crates/cairn-node/src/sync.rs"))
        .expect("sync.rs is in the tree");
    let found = text.matches("LocalDbFault::new(").count();

    assert_eq!(
        found, SYNC_LOCAL_DB_FAULT_SITES,
        "sync.rs has {found} `LocalDbFault::new(` sites, expected \
         {SYNC_LOCAL_DB_FAULT_SITES}. If you ADDED a postgres call, wrap it and bump the \
         constant. If this DROPPED, a call was reverted to `.context()` — which loses the \
         SQLSTATE and unpins the chain the partition classifier reads (#474 item 3)."
    );
    assert!(
        !text.contains(".context(\"checkpointing"),
        "the cursor checkpoint is the canonical #474 item 3 site: it must never be a \
         `.context()`, whatever the count says"
    );
}

#[test]
fn no_db_error_in_a_guarded_file_reaches_an_operator_as_its_kind() {
    let root = sources::repo_root();
    let mut offenders: Vec<String> = Vec::new();

    for rel in GUARDED {
        let path = root.join(rel);
        let text = sources::read_source(&path);
        for (n, line) in text.lines().enumerate() {
            if interpolates_a_raw_error(line) {
                offenders.push(format!("{rel}:{} — {}", n + 1, line.trim()));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "#467: a database failure must never reach an operator as `tokio_postgres::Error`'s \
         own Display — that is the literal string \"db error\" for a server-side failure, and \
         a bare kind name (\"error connecting to server\") for everything else. Wrap the \
         error in `db_diagnosis::legible_db_error(&e)` and interpolate THAT.\n\n{}",
        offenders.join("\n")
    );
}

/// The guard's own predicate, pinned — a guard whose judgement is wrong is worse than no
/// guard, because it reports the same green.
#[test]
fn the_predicate_catches_the_shape_that_filed_the_issue_and_spares_the_fix() {
    // The exact line from `db.rs` as it stood when #467 was filed.
    assert!(interpolates_a_raw_error(
        r#"            .map_err(|e| anyhow::anyhow!("loading {name}: {e}"))"#
    ));
    assert!(interpolates_a_raw_error(r#"eprintln!("failed: {err}")"#));
    assert!(interpolates_a_raw_error(r#"format!("{error}")"#));

    // The fix, which must not trip it.
    assert!(!interpolates_a_raw_error(
        r#"            .map_err(|e| anyhow::anyhow!("loading {name}: {}", legible_db_error(&e)))"#
    ));
    // A name that merely CONTAINS a guarded binding is not one.
    assert!(!interpolates_a_raw_error(
        r#"format!("{event}: {errors_seen}")"#
    ));

    // A comment cannot render anything to an operator, and all three files here explain
    // the defect by naming its shape. Every comment form must be spared.
    assert!(!interpolates_a_raw_error(
        r#"    // and `{e}` printed `db error` in its place."#
    ));
    assert!(!interpolates_a_raw_error(
        r#"/// flags `{e}` in this file, and its predicate is name-based."#
    ));
    assert!(!interpolates_a_raw_error(r#"//! `anyhow!("…: {e}")`"#));

    // …but a trailing comment does not launder the code beside it.
    assert!(interpolates_a_raw_error(
        r#"eprintln!("failed: {e}"); // TODO: make legible"#
    ));
}
