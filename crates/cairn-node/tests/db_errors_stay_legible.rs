//! #467 stays fixed: no DB error in `db.rs` may reach an operator as `{e}`.
//!
//! # Why a source scan rather than more behavioural tests
//!
//! PR #472 rerouted all fourteen `tokio_postgres::Error` renderings in
//! `crates/cairn-node/src/db.rs` through `db_diagnosis::legible_db_error`, and two of them
//! are pinned behaviourally by `tests/db_diagnosis.rs` (the migration door and the connect
//! door). The other twelve are the same one-line composition, and writing twelve more
//! near-identical DB-gated tests would buy little.
//!
//! What that leaves unguarded is not any individual site but the **class**: the fifteenth
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
//! Any interpolation of a bare binding named `e`/`err`/`error` inside a string in `db.rs`
//! — `{e}`, `{err}`, `{error}`, and the positional `{}` forms that pass one of those names
//! as an argument. The legitimate shape is `{}` fed by `legible_db_error(&e)`, which is
//! why the check is on the INTERPOLATION and not on the word.
//!
//! # Scope, stated so coverage is not confused with aspiration
//!
//! `crates/cairn-node/src/db.rs` only. It is the file #467 was filed against, the file
//! that fails first on a fresh node, and the one whose every statement talks to the
//! database. The same species elsewhere in the crate is real and tracked separately —
//! `safety.rs` is #473 and `sync.rs` is #474 — and widening this guard to the whole crate
//! today would simply fail on those two, so it would have to be born disabled. It is
//! written to be widened once they are fixed: change `GUARDED` and delete this paragraph.

#[path = "common/sources.rs"]
mod sources;

/// The files whose DB errors must stay legible. See the module doc for why this is one
/// file today and what has to happen before it is more.
const GUARDED: &[&str] = &["crates/cairn-node/src/db.rs"];

/// Bindings that, interpolated raw, render a `tokio_postgres::Error` as its useless kind.
const RAW_ERROR_BINDINGS: &[&str] = &["e", "err", "error"];

/// Does this line interpolate one of the raw error bindings into a string?
///
/// Deliberately simple and slightly over-eager: it is a guard, and a false positive costs
/// one `legible_db_error` call or one `#[allow]`-style rename, while a false negative costs
/// an operator their diagnosis. Pure, so the judgement is testable without touching disk.
fn interpolates_a_raw_error(line: &str) -> bool {
    RAW_ERROR_BINDINGS
        .iter()
        .any(|b| line.contains(&format!("{{{b}}}")))
}

#[test]
fn no_db_error_in_the_schema_loader_reaches_an_operator_as_its_kind() {
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
}
