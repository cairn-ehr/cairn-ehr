//! #442 — the DB-gated suite cannot go silently green.
//!
//! # The hole
//!
//! Every DB-gated test in this crate opens with some form of `let Some(base) = cs() else {
//! return };`, so a machine without PostgreSQL can still run `cargo test`. That is the right
//! default locally and the wrong one unattended, because a *skipped* test prints `ok`. Nothing
//! distinguishes "the in-DB floor suite proved its invariants" from "the same tests returned on
//! line 1" — the two runs produce byte-identical output — and that total is the number that
//! ends up quoted in a PR description as evidence.
//!
//! `CAIRN_TEST_PG` and its two siblings are set **together** in exactly one place *in CI*: a
//! step-level `env:` block in `.github/workflows/rust.yml`. (`scripts/run-db-gated-tests.sh`
//! exports the trio too, for local runs; `CAIRN_TEST_PG` alone also appears in the matcher's
//! own step. The first cut of this sentence said the trio appeared once and listed the script
//! as a `CAIRN_TEST_PG`-alone site — wrong on both halves, #456 review.) A typo in one of those
//! keys, a step split, or a job copied without its `env:` would skip the entire DB-gated suite
//! at once, and the run would be greener than usual rather than red. That is the failure mode
//! this file removes: after it, the same typo is a named, failing test.
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
//! the suite actually reads. A new gate variable is covered the moment the first test reads it,
//! with no edit here. The two sources being compared — the source text and the process
//! environment — are genuinely separate, which is what the #387 lesson asks for.
//!
//! # The scan reads CODE, not prose (#449)
//!
//! The first cut matched the literal prefix `CAIRN_TEST_` anywhere in any `.rs` file under
//! `crates/`, which made *prose* a source of requirements: a name written in a doc comment
//! became a variable CI had to supply, and the resulting failure blamed the environment for a
//! variable nothing reads. This repo's own worked example — the sentence about "a future
//! `CAIRN_TEST_PG4`", which has lived in `HANDOVER.md` and now lives in this change's plan
//! record — was live bait: `.md` files are outside the scan, but this repo's house style is
//! long `//!` headers, and the first person to paraphrase that sentence into a Rust module
//! header would have reddened the required `test` job.
//!
//! A name now counts only when it sits inside an `env::var("…")` / `env::var_os("…")`
//! argument, and whole-line comments are dropped before the scan. Both narrowings keep the
//! derivation — the authority is still the source text, not a list this file maintains — while
//! making the sentence "whatever the suite READS is what CI must provide" literally true rather
//! than approximately true.
//!
//! Three honest limits. A call site quoted verbatim *inside* a trailing comment, after code on
//! the same line, is still read as a call site; the same is true inside a `/* … */` BLOCK
//! comment, which `without_comment_lines` does not strip (the first cut of this header claimed
//! the tree contained no Rust block comments — it contains two, `transport.rs` and
//! `medication/attestation.rs`, neither holding an `env::var` call, but the exemption never
//! rested on that being true); and a name assembled at runtime
//! (`env::var(&format!("CAIRN_TEST_PG{n}"))`) is invisible, because there is no literal to
//! read. The first two are deliberate acts rather than accidents of prose; the third would be a
//! new idiom, and the `GATE_VARS_TODAY` floor below is what notices coverage shrinking.
//!
//! # Polarity: it fails CLOSED (#450)
//!
//! It used to bind only when `$CI` was set. `CI` is set in **zero** places in this repo — it is
//! inherited from the runner — so a self-hosted runner started from a scrubbed environment, a
//! container run without `-e CI`, `env -i`, or a future cron/systemd invocation would each
//! silently disable it, and the DB-gated suite would then skip and print `ok`, which is #442.
//! The guard's own trigger was the one unverified assumption in a file whose entire argument is
//! that unverified assumptions are how a suite goes silently green, and it failed **open**
//! while every other floor in this repo fails closed.
//!
//! So it binds by default, and a run that means to skip the database tier says so:
//! `CAIRN_ALLOW_DB_SKIP=1`. A contributor with no PostgreSQL sets it once (`CONTRIBUTING.md`
//! says so, and the failure message repeats it); `matcher.yml`'s deliberately database-free
//! job declares it in its own `env:` block, at the site that means it. Nothing else in CI does,
//! so every other unattended run is bound whether or not anything sets `CI`.
//!
//! # Scope, stated so coverage is not confused with aspiration
//!
//! It covers the Rust tests under `crates/`. Three trees are outside it, and each is outside
//! for its own reason: `cairn-gui` is a separate workspace with its own `cargo test` CI job (it
//! reads no gate variable today); `extensions/cairn_pgx` is a separate pgrx build whose tests
//! run against the extension's own harness, not a connection string (it reads none either);
//! and the matcher's Python DB-gated tests get their own copy of this guard in
//! `matcher/tests/test_db_gate_actually_ran.py` (#451), sharing the same opt-out variable so
//! there is one rule and not two. A gate variable read only under `extensions/` would be
//! uncovered — stated here rather than left to be discovered (#456 review).
//!
//! It also does not claim the substrate is *healthy* — only that the environment declared one.
//! A connection string pointing at a dead cluster fails loudly in the tests themselves, which
//! is the behaviour we want and is a different property from this one.
#[path = "common/sources.rs"]
mod sources;

use sources::{read_source, repo_root, source_files};
use std::collections::BTreeSet;
use std::path::Path;

/// The prefix every DB-gate variable shares.
const PREFIX: &str = "CAIRN_TEST_";

/// The explicit opt-out. Set it to an affirmative value to allow a database-free run.
///
/// Deliberately NOT prefixed `CAIRN_TEST_`: it is not a gate variable, and prefixing it would
/// make the scan below demand that the opt-out itself be set — a guard that requires its own
/// escape hatch.
const OPT_OUT: &str = "CAIRN_ALLOW_DB_SKIP";

/// This file's own repo-relative path, from `file!()`.
///
/// Excluded from the scan because the fixture tests below contain literal `env::var("…")` call
/// sites inside raw strings — synthetic source, written to pin the parser, which the parser
/// would otherwise read as real call sites and turn into requirements CI cannot satisfy.
///
/// Matched as a path suffix rather than by basename: a second file with this name elsewhere
/// under `crates/` — a plausible outcome of the #327 `cs()` unification — would otherwise be
/// silently excluded too, taking any variable only it reads out of the checked set.
const THIS_FILE: &str = file!();

/// The floor on how many gate variables the scan must find, as a liveness check on the scan
/// itself rather than a definition of the set.
///
/// Without it, a scan that silently found nothing — a moved directory, a changed calling
/// idiom, a parser that stopped recognising the call shape — would pass for the same reason a
/// correctly-configured run passes. Three is what the suite reads today (`CAIRN_TEST_PG` plus
/// the `PG2`/`PG3` the multi-node convergence suites need).
///
/// A *fourth* variable needs no edit here; the floor still holds. What does need an edit is a
/// variable legitimately going away — a retired multi-node suite, or #327's `cs()` unification
/// collapsing two of them — and that is meant to be a conscious act rather than a silent
/// narrowing of coverage, which is why the failure message names both causes.
const GATE_VARS_TODAY: usize = 3;

/// Drop whole-line comments, keeping line structure so a call spanning two lines still parses.
///
/// Crude on purpose: a line whose first non-whitespace is `//` is prose (`//`, `///`, `//!` all
/// qualify), and this repo writes essentially all of its commentary that way. Block comments
/// are not stripped, and a trailing comment after code on the same line is left in place —
/// both are limits the header states.
fn without_comment_lines(text: &str) -> String {
    text.lines()
        .map(|l| {
            if l.trim_start().starts_with("//") {
                ""
            } else {
                l
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Every `CAIRN_TEST_*` name passed as a literal to `env::var` / `env::var_os` in `text`.
///
/// A hand-rolled scan rather than a parser, because the shape being matched is tiny and fixed:
/// the call path, an open paren, a plain double-quoted literal. Whitespace between the paren
/// and the literal is tolerated so a rustfmt-wrapped call still reads.
///
/// `env::var` matches `std::env::var` too (it is a suffix of it), which is the form every call
/// site in this tree uses. A near-miss like `env::variable_os(` is rejected: after `env::var`
/// the scanner requires either `(` or `_os(`, and `iable_os(` is neither.
fn gate_var_names_in(text: &str) -> BTreeSet<String> {
    const CALL: &str = "env::var";
    let scanned = without_comment_lines(text);
    let mut found = BTreeSet::new();

    for (idx, _) in scanned.match_indices(CALL) {
        let mut rest = &scanned[idx + CALL.len()..];
        // `env::var_os` is the same read through a different return type.
        rest = rest.strip_prefix("_os").unwrap_or(rest);
        let Some(rest) = rest.trim_start().strip_prefix('(') else {
            continue;
        };
        // A plain `"…"` literal. Raw/byte-string literals are not used for env names, and a
        // non-literal argument (a variable, a `format!`) has no name to read — see the header.
        let Some(rest) = rest.trim_start().strip_prefix('"') else {
            continue;
        };
        let Some(end) = rest.find('"') else { continue };
        let name = &rest[..end];
        if name.starts_with(PREFIX) && name.len() > PREFIX.len() {
            found.insert(name.to_string());
        }
    }
    found
}

/// Every `CAIRN_TEST_*` environment variable the Rust suite under `crates/` actually reads,
/// plus how many times this file excluded itself (asserted below — see [`THIS_FILE`]).
fn gate_vars_read_by_the_suite() -> (BTreeSet<String>, usize) {
    let mut found = BTreeSet::new();
    let mut self_exclusions = 0usize;

    // Build output only. Everything else under crates/ is in scope, including `tests/` —
    // which is where every reader of these variables actually lives.
    for path in source_files(&[repo_root().join("crates")], &["target"], &["rs"]) {
        if is_this_file(&path) {
            self_exclusions += 1;
            continue;
        }
        found.extend(gate_var_names_in(&read_source(&path)));
    }
    (found, self_exclusions)
}

/// Is `path` this very source file? Compared as a path suffix — see [`THIS_FILE`].
fn is_this_file(path: &Path) -> bool {
    path.ends_with(THIS_FILE)
}

/// Is `raw` — the opt-out's raw value — an affirmative?
///
/// **Only an explicit affirmative opts out.** This is the OPPOSITE default from the `$CI`
/// predicate it replaced, and the inversion is the whole point: there, an unrecognised value
/// was read as "yes, this is CI", which bound the guard — the safe direction. Here an
/// unrecognised value must NOT be read as permission, or `CAIRN_ALLOW_DB_SKIP=please` (or
/// `=false`, or `=0`) silently restores the fail-open behaviour #450 removed.
///
/// Split from the environment read below so the DECISION is drivable by a test. The PR #456
/// review found the split missing and the consequence exact: the fixture asserted over a
/// hand-written COPY of this `matches!`, so `db_skip_is_allowed() { true }` turned the whole
/// #442/#450 guard into a no-op — measured, 5 of 5 tests still green with no gate variable
/// set at all. A guard defined over a copy of itself is not a guard (#387), which is the
/// lesson this very file was written to apply.
fn is_affirmative(raw: &str) -> bool {
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

/// May this run skip the database tier? The environment read, and nothing else.
///
/// Everything decidable lives in [`is_affirmative`]; this is the one line that cannot be
/// exercised without `std::env::set_var`, which is process-wide and unsound to call while
/// these tests run in parallel. [`the_env_read_is_wired_to_the_decision`] covers the residue
/// differentially, without mutating anything.
fn db_skip_is_allowed() -> bool {
    is_affirmative(&std::env::var(OPT_OUT).unwrap_or_default())
}

/// Is `raw` a value a connection string could be built from?
///
/// An *empty* value counts as missing. GitHub Actions resolves an undefined expression —
/// `CAIRN_TEST_PG3: ${{ env.TYPO }}` — to the empty string rather than to nothing, so the key
/// is present, `env::var` returns `Ok("")`, and a naive `is_err()` check passes while the suite
/// skips. That is the same species of typo the header names, arriving one layer lower down.
///
/// Split from [`is_usefully_set`] for [`is_affirmative`]'s reason: before the split, that
/// defence — the only thing standing between a mistyped `env:` key and a silent skip — had no
/// fixture in either language, and relaxing it to a bare `is_ok()` passed everywhere.
fn value_is_useful(raw: Option<&str>) -> bool {
    raw.is_some_and(|v| !v.trim().is_empty())
}

/// Is `name` set to something a connection string could be built from? The environment read.
fn is_usefully_set(name: &str) -> bool {
    value_is_useful(std::env::var(name).ok().as_deref())
}

/// The guard: every gate variable the suite reads must actually be set, unless this run has
/// explicitly declared that it is skipping the database tier.
#[test]
fn the_db_gated_suite_actually_ran() {
    let (vars, self_exclusions) = gate_vars_read_by_the_suite();

    // The self-exclusion must actually have fired. `file!()` is compared as a path suffix, and
    // if that ever stops matching — a build that reports absolute paths, a moved crate — the
    // exclusion would silently do nothing AND THE TEST WOULD STILL PASS on a correctly
    // configured machine, because the fixtures below name variables no environment sets. It is
    // asserted rather than assumed.
    assert_eq!(
        self_exclusions, 1,
        "expected to exclude exactly this file ({THIS_FILE}) from the scan, excluded \
         {self_exclusions} — the `file!()` suffix match has stopped identifying it, so its own \
         fixture call sites are now feeding the requirement list (#442)."
    );

    assert!(
        vars.len() >= GATE_VARS_TODAY,
        "the CAIRN_TEST_* scan found {} variable(s), fewer than the {GATE_VARS_TODAY} this \
         suite is known to read (#442). Either the scan has gone stale — a moved directory, or \
         a calling idiom the `env::var(\"…\")` matcher no longer recognises — in which case it \
         would now pass without checking anything, or a gate variable was deliberately \
         retired, in which case lower this floor in the same commit. Found: {vars:?}",
        vars.len()
    );

    if db_skip_is_allowed() {
        return;
    }

    let missing: Vec<&String> = vars.iter().filter(|v| !is_usefully_set(v)).collect();

    assert!(
        missing.is_empty(),
        "these DB-gate variables are unset or empty: {missing:?}\n\nEvery DB-gated test in this \
         crate self-skips without them AND PRINTS `ok`, so the whole in-DB floor suite would \
         have reported success while proving nothing (#442).\n\n\
         · In CI: set them in the job's `env:` block — see the `cargo test (workspace, in-DB \
         floor)` step in .github/workflows/rust.yml.\n\
         · Locally with PostgreSQL 18 + cairn_pgx: `scripts/run-db-gated-tests.sh` bakes all \
         three in.\n\
         · Locally WITHOUT a database: export {OPT_OUT}=1 to declare that this run skips the \
         database tier (see CONTRIBUTING.md). The guard fails closed on purpose — an absent \
         opt-out is not permission (#450)."
    );
}

// ─── Fixture tests: the parser is pinned over synthetic source, not over the tree ──────────
//
// ANTI-VACUITY. The scan above runs against whatever `crates/` happens to contain, so on any
// given day it could pass while recognising very little. These pin the two properties #449 is
// about — prose is ignored, a real read is found — over strings written here, so they fail if
// the parser regresses regardless of what the real tree looks like.
//
// The raw strings below contain genuine `env::var("CAIRN_TEST_…")` call sites. That is exactly
// why THIS_FILE is excluded from the walk: without the exclusion these fixtures would become
// requirements the environment has to satisfy.

/// A name mentioned only in prose is NOT a requirement. This is #449 itself.
#[test]
fn prose_does_not_invent_a_gate_variable() {
    let doc_comment = r#"
//! Set CAIRN_TEST_PG4 before running the multi-node suites.
/// See CAIRN_TEST_PG5 for the fourth cluster.
// TODO: CAIRN_TEST_PG6 once the sweep lands.
"#;
    assert!(
        gate_var_names_in(doc_comment).is_empty(),
        "a name written in a comment must not become a variable CI has to supply (#449)"
    );

    // Prose that is not even in a comment — a failure message, a doc string — is likewise
    // inert, because it sits in no `env::var` argument.
    let message = r#"eprintln!("skipped: set CAIRN_TEST_PG7");"#;
    assert!(
        gate_var_names_in(message).is_empty(),
        "a bare name in a string literal is not a read"
    );

    // A COMMENTED-OUT CALL SITE — the only fixture here that actually pins
    // `without_comment_lines`. The three above contain no `env::var(` shape at all, so they
    // are rejected by the call-shape matcher and would still pass with the comment stripper
    // deleted outright: the PR #456 review measured exactly that, leaving half of #449
    // unpinned. This one fails without the stripper.
    let commented_out_call = r#"
    // let base = std::env::var("CAIRN_TEST_PG8").ok();
        /// Superseded: std::env::var_os("CAIRN_TEST_PG9") was read here until #327.
"#;
    assert!(
        gate_var_names_in(commented_out_call).is_empty(),
        "a call site that has been COMMENTED OUT is not a read — this is what the \
         whole-line comment stripper is for (#449)"
    );
}

/// A real read IS found, in every form the tree uses.
#[test]
fn a_real_read_is_found() {
    let calls = r#"
        let base = std::env::var("CAIRN_TEST_PG").ok();
        let (a, b) = (cs(), std::env::var("CAIRN_TEST_PG2").ok());
        let os = std::env::var_os("CAIRN_TEST_PG3");
        let wrapped = std::env::var(
            "CAIRN_TEST_PG9",
        );
    "#;
    let found = gate_var_names_in(calls);
    assert_eq!(
        found,
        [
            "CAIRN_TEST_PG",
            "CAIRN_TEST_PG2",
            "CAIRN_TEST_PG3",
            "CAIRN_TEST_PG9"
        ]
        .into_iter()
        .map(String::from)
        .collect::<BTreeSet<_>>(),
        "every literal env::var / env::var_os read must be picked up"
    );
}

/// Near-misses and non-gate variables contribute nothing.
#[test]
fn near_misses_are_rejected() {
    let cases = [
        // Not a gate variable — including this file's own opt-out, which must never become a
        // requirement.
        r#"std::env::var("CAIRN_ALLOW_DB_SKIP")"#,
        r#"std::env::var("CI")"#,
        // A different function whose name merely starts the same way.
        r#"sys_env::variable_os("CAIRN_TEST_PG4")"#,
        // The prefix alone names nothing.
        r#"std::env::var("CAIRN_TEST_")"#,
        // No literal to read: the name is assembled at runtime.
        r#"std::env::var(&format!("CAIRN_TEST_PG{n}"))"#,
        // A left-boundary near-miss: the argument is a DIFFERENT variable.
        r#"std::env::var("OLD_CAIRN_TEST_PG4")"#,
    ];
    for case in cases {
        assert!(
            gate_var_names_in(case).is_empty(),
            "must contribute no variable: {case}"
        );
    }
}

/// The opt-out recognises affirmatives only — an unrecognised value is not permission.
///
/// Drives the REAL [`is_affirmative`], which is the whole of the decision. The previous
/// version of this test re-implemented the `matches!` as a local closure and asserted over
/// the copy; the PR #456 review mutation-tested it and found the guard could be reduced to
/// `db_skip_is_allowed() { true }` — a total no-op — with every test still green.
#[test]
fn only_an_explicit_affirmative_opts_out() {
    for yes in ["1", "true", "TRUE", " yes ", "on"] {
        assert!(is_affirmative(yes), "{yes:?} must opt out");
    }
    // The whole point of #450: anything else binds the guard.
    for no in ["", "0", "false", "no", "off", "please", "maybe", "  "] {
        assert!(!is_affirmative(no), "{no:?} must NOT be read as permission");
    }
}

/// The environment read is wired to the decision — the residue [`is_affirmative`] cannot cover.
///
/// `set_var` is process-wide and unsound under parallel tests, so this asserts DIFFERENTIALLY
/// over whatever value this process was actually given: [`db_skip_is_allowed`] must agree with
/// [`is_affirmative`] applied to the raw `$CAIRN_ALLOW_DB_SKIP`. That catches every mutation
/// the pure fixture cannot — a constant return, a different variable name, an inverted sense —
/// in any run that did not opt out, which is every CI run (no Rust job sets it) and every local
/// run without it.
///
/// Honest about its own shape: in a run that DID opt out, both sides are `true` and this proves
/// little. That is the correct asymmetry — the case worth catching is a guard that waves a run
/// through, and that case is exactly the one this covers.
#[test]
fn the_env_read_is_wired_to_the_decision() {
    let raw = std::env::var(OPT_OUT).unwrap_or_default();
    assert_eq!(
        db_skip_is_allowed(),
        is_affirmative(&raw),
        "db_skip_is_allowed() must be nothing but is_affirmative() over ${OPT_OUT} (raw: \
         {raw:?}). If these disagree it has stopped reading that variable, or stopped \
         delegating — either way the #450 polarity is no longer what is_affirmative says it \
         is, and the whole guard can be waved through."
    );
}

/// Every crate that reads a gate variable is REACHED by the walk, and its read is PARSED there.
///
/// The fixtures above pin the parser over synthetic strings; the floor pins that the real scan
/// found at least three names. Neither notices a PARTIAL collapse of the walk: if
/// `crates/cairn-sync/` stopped being reached, `vars.len() >= 3` would still hold from
/// `crates/cairn-node/` alone, and a variable read only there would silently leave the
/// requirement set. Named per crate rather than counted, because a count cannot tell "found
/// both" from "found one twice" — the Python guard made that argument first (#451), and this
/// is the Rust side adopting it (#456 review).
///
/// The two files are named rather than derived on purpose: this is the one assertion in the
/// file that is allowed to know something concrete about the tree, and it is checked in the
/// direction that fails if the tree moves out from under it.
#[test]
fn every_crate_that_reads_a_gate_variable_is_reached() {
    let files = source_files(&[repo_root().join("crates")], &["target"], &["rs"]);
    for anchor in [
        "crates/cairn-node/tests/common/mod.rs",
        "crates/cairn-sync/src/main.rs",
    ] {
        let path = files
            .iter()
            .find(|p| p.ends_with(anchor))
            .unwrap_or_else(|| {
                panic!(
                    "the walk over crates/ never reached {anchor}, which reads a CAIRN_TEST_* \
                     variable — the scan has silently narrowed, and any variable read only \
                     under that crate has dropped out of the requirement set (#442)."
                )
            });
        let found = gate_var_names_in(&read_source(path));
        assert!(
            !found.is_empty(),
            "{anchor} reads a CAIRN_TEST_* variable and the parser must see it THERE. \
             Asserting only over the union lets one crate cover for another going silent."
        );
    }
}

/// An empty or whitespace-only value counts as MISSING (#456 review).
///
/// The GitHub-Actions defence [`value_is_useful`] describes had no fixture in either language:
/// relaxing it to a bare `is_ok()` passed everywhere, because CI sets these variables non-empty
/// and no test ever supplied an empty one.
#[test]
fn an_empty_value_counts_as_missing() {
    assert!(value_is_useful(Some("host=127.0.0.1 dbname=cairn_test")));
    // `CAIRN_TEST_PG3: ${{ env.TYPO }}` resolves to "" — the key is present and the read
    // succeeds, which is precisely how a mistyped env: block skips the tier while looking set.
    for empty in ["", "   ", "\t\n"] {
        assert!(
            !value_is_useful(Some(empty)),
            "{empty:?} must count as missing"
        );
    }
    assert!(!value_is_useful(None), "an unset variable is missing");
}
