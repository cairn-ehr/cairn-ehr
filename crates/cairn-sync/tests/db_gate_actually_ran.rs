//! `cairn-sync`'s binding of the shared DB-gate guard (#481).
//!
//! # The hole this closes
//!
//! #450 made a database-free `cargo test` fail closed, and it worked — in `cairn-node`, whose
//! integration test carried it. A test binary only runs when its own crate is tested, so the
//! guard never bound a per-crate run of this one. Measured before this file existed:
//!
//! ```text
//! $ cargo test -p cairn-sync        # CAIRN_TEST_PG unset, no opt-out declared
//! test result: ok. 101 passed; 0 failed
//! $ echo $?
//! 0
//! ```
//!
//! Every DB-gated test in this crate self-skips with an `eprintln!` and a bare `return`, and
//! the run reports green. That mattered immediately rather than in principle: PR #478 landed
//! the behavioural halves of #471 and #475 here — `a_requeue_interrupted_mid_loop_still_-
//! reports_what_it_released`, the only test that exercises a real mid-loop interruption, and
//! `a_failed_migration_names_the_migration_and_the_cause`, which is the whole of #475's
//! acceptance criterion. The two issues with the thinnest coverage sat in the one crate whose
//! per-crate runs were unguarded, and a developer iterating with `cargo test -p cairn-sync`
//! got a confident green over both.
//!
//! CI was not exposed — `scripts/run-db-gated-tests.sh` runs a *workspace* `cargo test`, so
//! `cairn-node`'s binding fires there — but it would have become a CI hole the moment anyone
//! added a per-crate job.
//!
//! # Why an include and not a copy
//!
//! The guard is one file, pulled into both crates. A second copy would need a drift test
//! keeping the two in lockstep; #452 records what this repo learned when a source walk was
//! copied three times, and two copies carried defects the third had already fixed. Sharing
//! removes the question instead of answering it — a fix to the parser or to the fail-closed
//! polarity lands in both binaries in the same commit, unavoidably.
//!
//! The argument, the parser, the polarity and the fixtures are all in the shared module's
//! header. Read that, not this.
#[path = "../../cairn-node/tests/common/db_gate.rs"]
mod db_gate;
