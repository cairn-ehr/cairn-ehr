//! This tree's DB-gated suites cannot go silently green (#442's rule, in proportion).
//!
//! Every DB-gated test in `cairn-gui-live` opens with `let Some(cs) = common::cs() else {
//! return; };` — the right default on a laptop with no PostgreSQL, and the wrong one
//! unattended, because a skipped test prints `ok`. The run that proved the funnel writes a
//! truthful attestation and the run that returned on line 1 are byte-identical in exactly the
//! output a PR description quotes as evidence.
//!
//! So: with no `CAIRN_TEST_PG`, this test FAILS, unless the run declares that it knows —
//! `CAIRN_ALLOW_DB_SKIP=1`. CI's `gui` job sets that, because it deliberately has no database
//! (the suites run in the `test` job, which has PostgreSQL and the `cairn_pgx` extension); a
//! developer who simply forgot the export does not, and gets told.
//!
//! This is much smaller than the root tree's `common/db_gate.rs`, and the difference is
//! honest rather than lazy. That one DERIVES the variable list from the test sources, because
//! three variables spread across a hundred suites is a list that rots — the 2026-08-19 lesson
//! that a guard defined over the list it guards is not a guard. Here there is ONE variable and
//! two suites, both in this crate, both visible in one directory listing. If this crate ever
//! grows a second gate variable, derive the list; until then a derived list would be
//! ceremony.
mod common;

/// The declaration that a skip is intended. Same name the root tree uses, so a contributor
/// exports it once (`CONTRIBUTING.md`) and both trees are satisfied.
const OPT_OUT: &str = "CAIRN_ALLOW_DB_SKIP";

#[test]
fn the_db_gated_suites_in_this_crate_actually_ran() {
    if common::cs().is_some() {
        return;
    }
    let declared = std::env::var(OPT_OUT)
        .ok()
        .is_some_and(|v| common::is_affirmative(&v));
    assert!(
        declared,
        "CAIRN_TEST_PG is unset, so every DB-gated test in cairn-gui-live returned on its \
         first line and printed `ok`. That is fine on a machine with no PostgreSQL — say so \
         with `export {OPT_OUT}=1`. It is not fine unnoticed: this crate's whole subject is \
         what a registration actually writes to a real record."
    );
}

/// The opt-out must not be satisfiable by accident. A bare `env::var(..).is_ok()` would let
/// `CAIRN_ALLOW_DB_SKIP=false` disable the gate, which is how a guard becomes decoration.
#[test]
fn only_an_affirmative_value_opts_out() {
    for yes in ["1", "true", "TRUE", " yes ", "on"] {
        assert!(common::is_affirmative(yes), "{yes:?} must read as yes");
    }
    for no in ["", "0", "false", "no", "off", "please", "maybe"] {
        assert!(!common::is_affirmative(no), "{no:?} must NOT read as yes");
    }
}
