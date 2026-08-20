//! #442 — the DB-gated suite cannot go silently green.
//!
//! # The hole
//!
//! Every DB-gated test in this crate opens with some form of `let Some(base) = cs() else {
//! return };`, so a machine without PostgreSQL can still run `cargo test`. That is the right
//! default locally and the wrong one in CI, because a *skipped* test prints `ok`. Nothing
//! distinguishes "the in-DB floor suite proved its invariants" from "the same tests returned on
//! line 1" — the two runs produce byte-identical output — and that total is the number that
//! ends up quoted in a PR description as evidence.
//!
//! `CAIRN_TEST_PG` and its two siblings are set **together** in exactly one place: a step-level
//! `env:` block in `.github/workflows/rust.yml`. (`CAIRN_TEST_PG` alone also appears in the
//! matcher's own step and in `scripts/run-db-gated-tests.sh`; the trio appears once.) A typo in
//! one of those keys, a step split, or a job copied without its `env:` would skip the entire
//! DB-gated suite at once, and the run would be greener than usual rather than red. That is the
//! failure mode this file removes: after it, the same typo is a named, failing test.
//!
//! # Why the variable list is derived rather than written down
//!
//! The 2026-08-19 review lesson (recorded on #387) was that a guard defined over the list it
//! guards is not a guard. A hardcoded `["CAIRN_TEST_PG", "CAIRN_TEST_PG2", "CAIRN_TEST_PG3"]`
//! would assert that three names this file chose are set, and would be blind to the fourth the
//! moment a future multi-node suite starts reading it — which is precisely when the coverage
//! claim stops being true.
//!
//! So the names come from the **test sources**, which are the independent authority on what
//! the suite actually reads: whatever `crates/` mentions is what CI must provide. A new gate
//! variable is covered the moment the first test reads it, with no edit here. The two sources
//! being compared — the source text and the process environment — are genuinely separate,
//! which is what the #387 lesson asks for.
//!
//! The cost of deriving from source text rather than from a list is that *prose* counts too: a
//! `CAIRN_TEST_…` name written in a doc comment anywhere under `crates/` becomes a name CI has
//! to satisfy. That is a deliberate trade (a mention is usually a read), but it is a live edge —
//! `docs/HANDOVER.md` and `docs/ROADMAP.md` both use a hypothetical `CAIRN_TEST_PG4` as a worked
//! example, and this repo's house style is long `//!` headers. Only `.md` files are outside the
//! scan; paraphrasing that sentence into a Rust header would red CI. #449 tracks narrowing the
//! scan to `env::var` call sites, which keeps the derivation and drops the prose sensitivity.
//!
//! # Scope, stated so coverage is not confused with aspiration
//!
//! It binds only when `$CI` is set, so a local `cargo test` on a laptop with no database is
//! unaffected — the skip behaviour is deliberate and stays. Note that `$CI` is itself an
//! unverified assumption of exactly the kind this file argues against, and it fails *open*:
//! nothing in this repo sets `CI`, it is inherited from the runner, and a scrubbed environment
//! would silently disable the guard. #450 carries the question of inverting it to an explicit
//! opt-out.
//!
//! It covers the Rust tests under `crates/`. `cairn-gui` is a separate workspace with its own
//! `cargo test` CI job (it reads no gate variable today), and the matcher's Python DB-gated
//! tests run in their own CI step with their own `env:` block — fifteen files that keep the
//! identical hole this one closes, tracked in #451.
//!
//! It also does not claim the substrate is *healthy* — only that CI declared one. A connection
//! string pointing at a dead cluster fails loudly in the tests themselves, which is the
//! behaviour we want and is a different property from this one.
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// This file's own repo-relative path, from `file!()`.
///
/// It is excluded from the scan below because it necessarily *writes* the variable names in
/// prose, so scanning it would let a documentation example become a requirement the environment
/// has to satisfy — a guard that grows new obligations from its own comments.
///
/// Matched as a path suffix rather than by basename: a second file with this name elsewhere
/// under `crates/` — a plausible outcome of the #327 `cs()` unification — would otherwise be
/// silently excluded too, taking any variable only it reads out of the checked set.
const THIS_FILE: &str = file!();

/// The floor on how many gate variables the scan must find, as a liveness check on the scan
/// itself rather than a definition of the set.
///
/// Without it, a scan that silently found nothing — a moved directory, a changed naming
/// convention — would pass for the same reason a correctly-configured CI run passes. Three is
/// what the suite reads today (`CAIRN_TEST_PG` plus the `PG2`/`PG3` the multi-node convergence
/// suites need).
///
/// A *fourth* variable needs no edit here; the floor still holds. What does need an edit is a
/// variable legitimately going away — a retired multi-node suite, or #327's `cs()` unification
/// collapsing two of them — and that is meant to be a conscious act rather than a silent
/// narrowing of coverage, which is why the failure message names both causes.
const GATE_VARS_TODAY: usize = 3;

fn crates_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .canonicalize()
        .expect("crates/ dir")
}

/// SCREAMING_SNAKE identifier characters. The run of these following the prefix is the name.
fn is_name_char(c: char) -> bool {
    c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'
}

/// Every `CAIRN_TEST_*` environment variable name any Rust source under `crates/` mentions.
///
/// A deliberately simple maximal-run scan rather than a parser: the names are SCREAMING_SNAKE
/// identifiers, so the run of `[A-Z0-9_]` following the prefix is the whole name. Trailing
/// underscores are trimmed so a prose mention like `CAIRN_TEST_PG_` cannot invent a variable.
///
/// The prefix must also *start* an identifier. Without that check `OLD_CAIRN_TEST_PG4` would
/// register a phantom `CAIRN_TEST_PG4` that no file reads and no CI job can set — the guard
/// inventing an obligation and then failing because it went unmet.
fn gate_vars_read_by_the_suite() -> (BTreeSet<String>, usize) {
    const PREFIX: &str = "CAIRN_TEST_";
    let mut found = BTreeSet::new();
    let mut self_exclusions = 0usize;
    let mut stack = vec![crates_dir()];

    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("readable dir") {
            let entry = entry.expect("dir entry");
            let path = entry.path();
            // `file_type()` does NOT follow symlinks, unlike `path.is_dir()`. A symlink to an
            // ancestor under crates/ would otherwise make this walk unbounded — a hang in a
            // required check, which reads as CI flakiness rather than as a defect.
            let file_type = entry.file_type().expect("dir entry file type");
            let name = entry.file_name().to_string_lossy().to_string();

            if file_type.is_dir() {
                // Build output only. Everything else under crates/ is in scope, including
                // `tests/` — which is where every reader of these variables actually lives.
                if name != "target" {
                    stack.push(path);
                }
            } else if file_type.is_file() && name.ends_with(".rs") {
                if is_this_file(&path) {
                    self_exclusions += 1;
                    continue;
                }
                // Loud, not silent: an unreadable or non-UTF-8 source would otherwise
                // contribute nothing and take every variable whose only evidence lived there
                // out of the checked set — a silent failure inside an anti-silence guard.
                let text = std::fs::read_to_string(&path)
                    .unwrap_or_else(|e| panic!("unreadable test source {}: {e}", path.display()));

                for (idx, _) in text.match_indices(PREFIX) {
                    if text[..idx].ends_with(is_name_char) {
                        continue;
                    }
                    let rest = &text[idx + PREFIX.len()..];
                    let end = rest.find(|c: char| !is_name_char(c)).unwrap_or(rest.len());
                    // `var_name`, not `name` — `name` is the FILE's name, three lines up.
                    let var_name = rest[..end].trim_end_matches('_');
                    if !var_name.is_empty() {
                        found.insert(format!("{PREFIX}{var_name}"));
                    }
                }
            }
        }
    }
    (found, self_exclusions)
}

/// Is `path` this very source file? Compared as a path suffix — see [`THIS_FILE`].
fn is_this_file(path: &Path) -> bool {
    path.ends_with(THIS_FILE)
}

/// Is this an unattended run that must not report a skip as a pass?
///
/// `CI` rather than `GITHUB_ACTIONS`: every CI system sets it, and the property being guarded
/// is not specific to GitHub. A future self-hosted or cron runner inherits the guard for free.
///
/// Empty, `false` and `0` are treated as *not* CI. Some tooling exports `CI=false`, and a
/// contributor with that set would otherwise get a red test on a laptop with no PostgreSQL —
/// the exact case the self-skip exists to permit.
fn running_under_ci() -> bool {
    match std::env::var("CI") {
        Ok(v) => {
            let v = v.trim().to_ascii_lowercase();
            !v.is_empty() && v != "false" && v != "0"
        }
        Err(_) => false,
    }
}

/// Is `name` set to something a connection string could be built from?
///
/// An *empty* value counts as missing. GitHub Actions resolves an undefined expression —
/// `CAIRN_TEST_PG3: ${{ env.TYPO }}` — to the empty string rather than to nothing, so the key
/// is present, `env::var` returns `Ok("")`, and a naive `is_err()` check passes while the suite
/// skips. That is the same species of typo the header names, arriving one layer lower down.
fn is_usefully_set(name: &str) -> bool {
    std::env::var(name).is_ok_and(|v| !v.trim().is_empty())
}

/// The guard: in CI, every gate variable the suite reads must actually be set.
///
/// Outside CI this is a no-op by design — the self-skip is what lets a contributor without
/// PostgreSQL run `cargo test` at all.
#[test]
fn the_db_gated_suite_actually_ran() {
    let (vars, self_exclusions) = gate_vars_read_by_the_suite();

    // The self-exclusion must actually have fired. `file!()` is compared as a path suffix, and
    // if that ever stops matching — a build that reports absolute paths, a moved crate — the
    // exclusion would silently do nothing AND THE TEST WOULD STILL PASS, because the names this
    // file writes in prose are the same three real variables the suite reads. The one name that
    // would leak is the hypothetical `CAIRN_TEST_PG4` in the header, which CI cannot set. Silent
    // either way, so it is asserted rather than assumed.
    assert_eq!(
        self_exclusions, 1,
        "expected to exclude exactly this file ({THIS_FILE}) from the scan, excluded \
         {self_exclusions} — the `file!()` suffix match has stopped identifying it, so its own \
         prose is now feeding the requirement list (#442)."
    );

    assert!(
        vars.len() >= GATE_VARS_TODAY,
        "the CAIRN_TEST_* scan found {} variable(s), fewer than the {GATE_VARS_TODAY} this \
         suite is known to read (#442). Either the scan has gone stale or is looking in the \
         wrong place — in which case it would now pass without checking anything — or a gate \
         variable was deliberately retired, in which case lower this floor in the same commit. \
         Found: {vars:?}",
        vars.len()
    );

    if !running_under_ci() {
        return;
    }

    let missing: Vec<&String> = vars.iter().filter(|v| !is_usefully_set(v)).collect();

    assert!(
        missing.is_empty(),
        "running under $CI, but these DB-gate variables are unset or empty: {missing:?}\n\nEvery \
         DB-gated test in this crate self-skips without them AND PRINTS `ok`, so the whole \
         in-DB floor suite would have reported success while proving nothing (#442). Set them \
         in the job's `env:` block — see the `cargo test (workspace, in-DB floor)` step in \
         .github/workflows/rust.yml — or unset $CI if this is not a CI run."
    );
}
