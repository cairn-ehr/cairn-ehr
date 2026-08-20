//! #442 — the DB-gated suite cannot go silently green.
//!
//! # The hole
//!
//! Every DB-gated test in this crate opens with some form of `let Some(base) = cs() else {
//! return };`, so a machine without PostgreSQL can still run `cargo test`. That is the right
//! default locally and the wrong one in CI, because a *skipped* test prints `ok`. Nothing
//! distinguishes "395 tests proved the in-DB floor holds" from "395 tests returned on line 1",
//! and the total is the number that ends up quoted in a PR description as evidence.
//!
//! `CAIRN_TEST_PG` and its two siblings are set in exactly one place — a step-level `env:`
//! block in `.github/workflows/rust.yml`. A typo in one of those keys, a step split, or a job
//! copied without its `env:` would skip the entire DB-gated suite at once, and the run would
//! be greener than usual rather than red. That is the failure mode this file removes: after
//! it, the same typo is a named, failing test.
//!
//! # Why the variable list is derived rather than written down
//!
//! The 2026-08-19 review lesson (#387) was that a guard defined over the list it guards is not
//! a guard. A hardcoded `["CAIRN_TEST_PG", "CAIRN_TEST_PG2", "CAIRN_TEST_PG3"]` would assert
//! that three names this file chose are set, and would be blind to the fourth the moment a
//! future multi-node suite starts reading it — which is precisely when the coverage claim
//! stops being true.
//!
//! So the names come from the **test sources**, which are the independent authority on what
//! the suite actually reads: whatever `crates/` mentions is what CI must provide. A new gate
//! variable is covered the moment the first test reads it, with no edit here. The two sources
//! being compared — the source text and the process environment — are genuinely separate,
//! which is what the #387 lesson asks for.
//!
//! # Scope, stated so coverage is not confused with aspiration
//!
//! It binds only when `$CI` is set, so a local `cargo test` on a laptop with no database is
//! unaffected — the skip behaviour is deliberate and stays. It covers the **Rust** suite; the
//! matcher's Python DB-gated tests run in their own CI step with their own `env:` block and
//! are outside what a Rust test can see.
//!
//! It also does not claim the substrate is *healthy* — only that CI declared one. A connection
//! string pointing at a dead cluster fails loudly in the tests themselves, which is the
//! behaviour we want and is a different property from this one.
use std::collections::BTreeSet;
use std::path::PathBuf;

/// This file's own name. It is excluded from the scan below because it necessarily *writes*
/// the variable names in prose, so scanning it would let a documentation example become a
/// requirement the environment has to satisfy — a guard that grows new obligations from its
/// own comments.
const THIS_FILE: &str = "db_gate_actually_ran.rs";

/// The floor on how many gate variables the scan must find, as a liveness check on the scan
/// itself rather than a definition of the set.
///
/// Without it, a scan that silently found nothing — a moved directory, a changed naming
/// convention — would pass for the same reason a correctly-configured CI run passes. Three is
/// what the suite reads today (`CAIRN_TEST_PG` plus the `PG2`/`PG3` the multi-node convergence
/// suites need); the number only ever rises, so this never needs revising upward.
const GATE_VARS_TODAY: usize = 3;

fn crates_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .canonicalize()
        .expect("crates/ dir")
}

/// Every `CAIRN_TEST_*` environment variable name any Rust source under `crates/` mentions.
///
/// A deliberately simple maximal-run scan rather than a parser: the names are SCREAMING_SNAKE
/// identifiers, so the run of `[A-Z0-9_]` following the prefix is the whole name. Trailing
/// underscores are trimmed so a prose mention like `CAIRN_TEST_PG_` cannot invent a variable.
fn gate_vars_read_by_the_suite() -> BTreeSet<String> {
    const PREFIX: &str = "CAIRN_TEST_";
    let mut found = BTreeSet::new();
    let mut stack = vec![crates_dir()];

    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("readable dir") {
            let path = entry.expect("dir entry").path();
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            if path.is_dir() {
                // Build output only. Everything else under crates/ is in scope, including
                // `tests/` — which is where every reader of these variables actually lives.
                if name != "target" {
                    stack.push(path);
                }
            } else if name.ends_with(".rs") && name != THIS_FILE {
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                for (idx, _) in text.match_indices(PREFIX) {
                    let tail: String = text[idx + PREFIX.len()..]
                        .chars()
                        .take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == '_')
                        .collect();
                    let tail = tail.trim_end_matches('_');
                    if !tail.is_empty() {
                        found.insert(format!("{PREFIX}{tail}"));
                    }
                }
            }
        }
    }
    found
}

/// The guard: in CI, every gate variable the suite reads must actually be set.
///
/// Outside CI this is a no-op by design — the self-skip is what lets a contributor without
/// PostgreSQL run `cargo test` at all.
#[test]
fn the_db_gated_suite_actually_ran() {
    let vars = gate_vars_read_by_the_suite();

    assert!(
        vars.len() >= GATE_VARS_TODAY,
        "the CAIRN_TEST_* scan found {} variable(s), fewer than the {GATE_VARS_TODAY} this \
         suite is known to read — the scan has gone stale or is looking in the wrong place, \
         and would now pass without checking anything (#442). Found: {vars:?}",
        vars.len()
    );

    // `CI` rather than `GITHUB_ACTIONS`: every CI system sets it, and the property being
    // guarded ("an unattended run must not report a skip as a pass") is not specific to
    // GitHub. A future self-hosted or cron runner inherits the guard for free.
    if std::env::var("CI").is_err() {
        return;
    }

    let missing: Vec<&String> = vars.iter().filter(|v| std::env::var(v).is_err()).collect();

    assert!(
        missing.is_empty(),
        "running under $CI, but these DB-gate variables are unset: {missing:?}\n\nEvery \
         DB-gated test in this crate self-skips without them AND PRINTS `ok`, so the whole \
         in-DB floor suite would have reported success while proving nothing (#442). Set them \
         in the job's `env:` block — see the `cargo test (workspace, in-DB floor)` step in \
         .github/workflows/rust.yml — or unset $CI if this is not a CI run."
    );
}
