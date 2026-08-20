//! #446 — every cargo tree in this repo builds from a lockfile the repo actually carries.
//!
//! # The incident this exists because of
//!
//! `extensions/cairn_pgx/Cargo.lock` was gitignored — cargo's default for a *library* crate,
//! inherited when the crates graduated to the top-level workspace. `cairn_pgx` is not a
//! library: it is a shared object loaded into a running PostgreSQL, and it is the artifact
//! that enforces the in-DB safety floor.
//!
//! The consequence was that `cargo pgrx install` re-resolved dependencies from scratch on
//! **every CI run**. On 2026-08-20 that resolve selected the compromised `arrayref` 0.3.10
//! and tried to fetch `proc-macro1`, a crate that typosquats `proc-macro2` and does not exist
//! on crates.io. A proc macro executes at **compile** time, inside the extension that enforces
//! the floor. It failed closed only because crates.io had already pulled the typosquat.
//!
//! It was also nondeterministic: the CI run twelve minutes earlier had passed, on a lockfile
//! restored by `Swatinem/rust-cache`. Cache hit meant pinned, cache miss meant fresh resolve,
//! and nothing reported which one had happened. Of the repo's **six** cargo trees, three were
//! pinned and three were not — and the most safety-critical of them was in the unpinned half.
//!
//! # Why a guard rather than vigilance
//!
//! It hid for months behind two lines of `.gitignore` nobody had a reason to open, and the
//! symptom when it finally bit was a required check failing in 45 seconds on a docs-only
//! commit — which reads as CI flakiness, not as an unpinned build. This repo also has its own
//! history of "pinned in two places, drifted apart" defects (#404 is the closest match: db/049's
//! thread arm diverged from db/048 and made a parameter inert), and a seventh cargo tree will
//! eventually be added by someone who is not thinking about any of this.
//!
//! # Why cargo is asked, rather than the manifests parsed
//!
//! Which manifest owns a lockfile is *cargo's* question, not ours: a manifest owns one when
//! it is a workspace root, or a package no workspace root claims. Reproducing that rule here
//! would mean re-implementing `members`/`exclude` resolution, including the globs and the
//! `[workspace]`-table-detaches-a-package case that `poc/iced-ui-spike` uses — a hand-rolled
//! copy that could drift from cargo's actual behaviour, which is precisely the class of defect
//! the #382 guard-design lesson warns about ("where a family has an authoritative list, read
//! the list"). `cargo locate-project --workspace` is that authoritative list, and it needs no
//! network.
//!
//! Asking cargo buys a second property worth naming, because it is not obvious: a manifest
//! cargo **cannot place at all** fails here too. `packaging/crates` was in exactly that state
//! until this guard was written — inside the root workspace directory, named in neither its
//! `members` nor its `exclude`, so every cargo command run in that directory errored with
//! "current package believes it's in a workspace when it's not". A crate published to
//! crates.io that no checkout could build is the kind of thing that stays true for years
//! because nobody has a reason to `cd` there. That property gets its own test rather than
//! riding inside the two lockfile assertions, so its failure headline names what actually
//! broke (#448 review).
//!
//! # What this guard does NOT claim
//!
//! It does not check that a lockfile's *contents* are sound — that is cargo-deny's job
//! (`deny.toml`), and #447 tracks the three trees cargo-deny does not yet cover.
//!
//! It does check the one property whose absence is silent: that a build in CI resolves from a
//! pinned set rather than from whatever crates.io serves that minute. Note that tracking a
//! lockfile is only half of that. A lockfile that has fallen behind its manifest is silently
//! re-resolved and rewritten mid-build, which is the same fresh resolve wearing a committed
//! file as a disguise — so the other half is `--locked`, which the CI workflow passes on every
//! cargo invocation against this repo's own trees. Both halves were added together in #448;
//! before it, the claim that CI passed `--locked` was written down here and was not true.
//!
//! All three tests here are pure source/VCS inspection — no database, so they run in every
//! `cargo test`, not only the DB-gated slice.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// How many lockfile-owning cargo trees the repo holds today (2026-08-21), as a floor rather
/// than an equality — the same liveness device `floor_execute_grants.rs` uses for its function
/// families, and for the same reason.
///
/// Without it, a scan that silently found *fewer* trees would pass for exactly the reason a
/// correct scan passes: no offenders. That is not hypothetical. `git ls-files` resolves its
/// pathspec against the process's working directory, so a `repo_root()` that ever pointed at a
/// subdirectory would quietly narrow the scan to the trees beneath it — plausibly to the one
/// root workspace, whose lockfile is tracked, leaving `extensions/cairn_pgx` (the tree the
/// whole incident was about) unexamined and the suite green.
///
/// The six: the root workspace, `cairn-gui`, `extensions/cairn_pgx`, `packaging/crates`,
/// `poc/iced-ui-spike`, `poc/pg-android-kit/extension`. Adding a tree raises this number; that
/// edit is the point, not an inconvenience.
const TREES_TODAY: usize = 6;

/// The captured outcome of one child process.
///
/// The exit **code** is kept rather than a bare success flag because the two git commands below
/// both use a three-way exit convention — 0 and 1 are answers, anything else is git failing to
/// answer at all — and collapsing that into a boolean is what made the ignore-rule test vacuous
/// before #448's review: `git check-ignore` exits 128 for a path it considers outside the
/// repository, which a `status.success()` test reads as the reassuring "no rule matched".
struct Run {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// Run a program in `dir` and capture its exit code and output.
///
/// `GIT_DIR`/`GIT_WORK_TREE` are removed from the child's environment because git honours them
/// *over* the working directory: with either one set, every `git` call below would report about
/// a different repository while still looking green. Passing a directory is not enough on its
/// own, which is what the previous version of this comment assumed.
fn run_in(dir: &Path, program: &str, args: &[&str]) -> Run {
    let out = Command::new(program)
        .args(args)
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .output()
        .unwrap_or_else(|e| panic!("failed to run `{program} {}`: {e}", args.join(" ")));
    Run {
        code: out.status.code(),
        stdout: String::from_utf8_lossy(&out.stdout).trim().to_string(),
        stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
    }
}

/// The repository root, as git itself reports it.
///
/// Asked rather than inferred positionally: the old `CARGO_MANIFEST_DIR/../..` walked up a fixed
/// number of levels and would silently point somewhere else the moment this crate moved, taking
/// the scan's scope with it. `--show-toplevel` is the authoritative answer, and it is also what
/// makes the `strip_prefix` below total — cargo and git then agree on the same physical root.
fn repo_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let out = run_in(&manifest_dir, "git", &["rev-parse", "--show-toplevel"]);
    assert_eq!(
        out.code,
        Some(0),
        "git rev-parse --show-toplevel failed in {}: {}",
        manifest_dir.display(),
        out.stderr
    );
    PathBuf::from(out.stdout)
}

/// Every `Cargo.toml` the repository **tracks**, as paths relative to the repo root.
///
/// Tracked rather than "found on disk" is deliberate and load-bearing twice over. It skips
/// build output and any local scratch checkout (`.claude/worktrees/*` holds several full
/// copies of this tree, each with its own manifests), and — the important half — it means
/// this guard reasons about the same file set a fresh CI clone sees, which is the set the
/// incident turned on.
///
/// The `file_name` filter is not redundant with the pathspec: git's `*` crosses `/` and does not
/// anchor at a separator, so `git ls-files '*Cargo.toml'` also matches a file literally named
/// `NotCargo.toml`. Handing one to cargo produces "the manifest-path must be a path to a
/// Cargo.toml file", which would surface as an *unplaceable manifest* — a failure whose stated
/// fix (join a workspace, or detach with `[workspace]`) cannot apply to a file that is not a
/// manifest at all.
fn tracked_manifests(repo: &Path) -> Vec<PathBuf> {
    let out = run_in(repo, "git", &["ls-files", "-z", "*Cargo.toml"]);
    assert_eq!(out.code, Some(0), "git ls-files failed: {}", out.stderr);
    out.stdout
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.file_name().is_some_and(|n| n == "Cargo.toml"))
        .collect()
}

/// The workspace root cargo resolves for `manifest`, or the error cargo reports.
///
/// `--offline` is passed because the answer is a pure function of the manifests on disk;
/// without it a future cargo could decide to touch the network and make this guard flaky for
/// a reason unrelated to what it tests.
///
/// Empty stdout is folded into the error arm rather than trusted. An empty path becomes
/// `"Cargo.lock"` after `with_file_name`, which resolves against the repo root — a real,
/// tracked, un-ignored lockfile — so a tree cargo failed to name would otherwise be reported
/// as clean, and would additionally collide with the genuine root entry in the map below.
fn workspace_root_for(repo: &Path, manifest: &Path) -> Result<PathBuf, String> {
    let manifest = manifest.to_string_lossy().to_string();
    let out = run_in(
        repo,
        env!("CARGO"),
        &[
            "locate-project",
            "--workspace",
            "--offline",
            "--message-format",
            "plain",
            "--manifest-path",
            &manifest,
        ],
    );
    match (out.code, out.stdout.is_empty()) {
        (Some(0), false) => Ok(PathBuf::from(out.stdout)),
        (Some(0), true) => Err("cargo exited 0 but named no workspace root".to_string()),
        _ => Err(out.stderr),
    }
}

/// The repo-relative path of the `Cargo.lock` a tree root owns.
///
/// `root_manifest` is cargo's answer from [`workspace_root_for`], so it is the path *to a
/// `Cargo.toml`* — not to a directory. The name matters: `with_file_name` on a directory would
/// replace that directory's own last component and produce a wrong-but-plausible path, and the
/// guard would then check a file nobody builds from.
///
/// The result is repo-relative because both callers hand it to `git`. An absolute path is not
/// the benign alternative an earlier version of this comment claimed — in-repo, git answers
/// identically for both forms; *out of* repo, git refuses with exit 128, which is precisely the
/// case [`git_ignore_rule_for`] must not mistake for "clean". `strip_prefix` is therefore an
/// assertion, not a best-effort: with `repo_root()` coming from `git rev-parse --show-toplevel`
/// there is no legitimate way for a tracked manifest's tree root to fall outside it, so a
/// failure here is a bug in this file and says so.
fn lock_path_for(root_manifest: &Path, repo: &Path) -> String {
    let lock = root_manifest.with_file_name("Cargo.lock");
    lock.strip_prefix(repo)
        .unwrap_or_else(|_| {
            panic!(
                "cargo named a tree root outside the repository: {} (repo root {})",
                lock.display(),
                repo.display()
            )
        })
        .to_string_lossy()
        .to_string()
}

/// Is `rel` in git's index? `Err` when git could not answer.
///
/// `--error-unmatch` exits 0 for tracked and 1 for untracked; anything else (a path git will not
/// evaluate, a corrupt index) is git failing rather than answering, and is reported as such so
/// the failure message does not tell a maintainer to `git add` a file that was never the problem.
fn git_tracks(repo: &Path, rel: &str) -> Result<bool, String> {
    let out = run_in(repo, "git", &["ls-files", "--error-unmatch", rel]);
    match out.code {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(out.stderr),
    }
}

/// The ignore rule matching `rel`, if any. `Err` when git could not answer.
///
/// A tracked file stays tracked even when an ignore rule matches it, so `--no-index` is what
/// makes this question about the *rules* rather than about the current index — without it git
/// skips tracked paths and the answer is "nothing is ignored" no matter what the `.gitignore`
/// files say. That is the failure mode worth stating outright: the polarity mistake here does
/// not make the guard noisy, it makes it vacuous.
///
/// Exit 128 is the same hazard one level down, and it *was* live until #448's review: git
/// refuses a path it considers outside the repository, and reading that refusal as "no rule
/// matched" turns the whole test into a pass that examined nothing.
///
/// `core.excludesFile=/dev/null` keeps the answer a property of the *repository*. Without it
/// git also consults the contributor's `~/.gitignore_global`, and a developer who ignores
/// `Cargo.lock` globally — an ordinary habit for someone who mostly writes libraries — would
/// get a red required check whose message tells them to delete a rule that is not in this repo.
/// `.git/info/exclude` is still in scope, deliberately: it is per-clone but it is local to this
/// repository, and a rule hidden there is exactly the kind this test should surface.
fn git_ignore_rule_for(repo: &Path, rel: &str) -> Result<Option<String>, String> {
    let out = run_in(
        repo,
        "git",
        &[
            "-c",
            "core.excludesFile=/dev/null",
            "check-ignore",
            "--no-index",
            "-v",
            rel,
        ],
    );
    match out.code {
        Some(0) => Ok(Some(out.stdout)),
        Some(1) => Ok(None),
        _ => Err(out.stderr),
    }
}

/// Map each lockfile-owning tree to one manifest that resolved to it, alongside the manifests
/// cargo could not place at all.
///
/// The manifest is carried alongside so a failure can name a path a reader recognises: for a
/// tree whose root manifest is not itself tracked, the root alone would be the only clue.
///
/// The unplaceable list is *returned* rather than asserted here, so that one shared helper does
/// not decide the verdict of three tests: before #448's review, a regressed `packaging/crates`
/// failed both lockfile tests under headlines about lockfiles, which is not what had broken.
fn survey_cargo_trees(repo: &Path) -> (BTreeMap<PathBuf, PathBuf>, Vec<String>) {
    let mut trees = BTreeMap::new();
    let mut unplaceable = Vec::new();

    for manifest in tracked_manifests(repo) {
        match workspace_root_for(repo, &manifest) {
            Ok(root) => {
                trees.entry(root).or_insert(manifest);
            }
            Err(err) => unplaceable.push(format!("  {}\n    {err}", manifest.display())),
        }
    }

    (trees, unplaceable)
}

/// The lockfile-owning trees, with the liveness floor applied.
///
/// Both lockfile tests go through here so neither can iterate a silently-narrowed set.
fn lockfile_owning_trees(repo: &Path) -> BTreeMap<PathBuf, PathBuf> {
    let (trees, _) = survey_cargo_trees(repo);
    assert!(
        trees.len() >= TREES_TODAY,
        "found {} lockfile-owning cargo tree(s) under {}, fewer than the {TREES_TODAY} this \
         repo is known to have — the scan has gone stale or is looking in the wrong place, and \
         would now pass without examining the trees it is meant to cover (#446). Found: {:?}",
        trees.len(),
        repo.display(),
        trees.keys().collect::<Vec<_>>()
    );
    trees
}

/// Every tracked manifest is one cargo can actually place in a workspace.
///
/// Its own test because its own defect: a package enclosed by a workspace directory but named in
/// neither `members` nor `exclude` makes *every* cargo command in that directory fail, which has
/// nothing to do with lockfiles and everything to do with whether the crate can be built at all.
#[test]
fn every_tracked_manifest_is_placeable_by_cargo() {
    let repo = repo_root();
    let (trees, unplaceable) = survey_cargo_trees(&repo);

    // Its own liveness floor, for the same reason the other two have one: an empty survey
    // yields an empty `unplaceable` list, which is indistinguishable from "every manifest is
    // fine". Counting both arms is the honest total — an unplaceable manifest is still one the
    // scan saw. There are 19 tracked manifests today, so TREES_TODAY is a loose but sufficient
    // floor: it cannot pass on nothing.
    let surveyed = trees.len() + unplaceable.len();
    assert!(
        surveyed >= TREES_TODAY,
        "surveyed only {surveyed} tracked manifest(s) under {} — the scan has gone stale or is \
         looking in the wrong place, and would now pass without examining anything (#446).",
        repo.display()
    );

    assert!(
        unplaceable.is_empty(),
        "cargo cannot place these tracked manifests in any workspace, so no cargo command \
         works in their directory (#446):\n{}\n\nFix: either add the package to the enclosing \
         workspace's `members`, or detach it with an empty `[workspace]` table in its own \
         manifest (the way poc/iced-ui-spike does).",
        unplaceable.join("\n")
    );
}

/// The load-bearing assertion: every tree that resolves its own dependency set does so from a
/// lockfile a fresh clone will have.
#[test]
fn every_cargo_tree_has_a_tracked_lockfile() {
    let repo = repo_root();
    let mut offenders = Vec::new();

    for (tree, via) in lockfile_owning_trees(&repo) {
        let rel = lock_path_for(&tree, &repo);

        // Tracked is the property that matters, not present: a gitignored lockfile is present
        // on every developer machine and absent in CI, which is exactly the state that hid
        // the cairn_pgx gap for months. `--error-unmatch` is git's own answer to "is this
        // path in the index", so this cannot disagree with what a clone would contain.
        match git_tracks(&repo, &rel) {
            Ok(true) => {}
            Ok(false) => offenders.push(format!(
                "  {rel}\n    (cargo tree reached via {})",
                via.display()
            )),
            Err(err) => panic!("git could not answer whether {rel} is tracked: {err}"),
        }
    }

    assert!(
        offenders.is_empty(),
        "these cargo trees have no lockfile tracked by git, so CI resolves their dependencies \
         fresh on every run (#446, #445):\n{}\n\nFix: `cargo generate-lockfile` in that \
         directory, `git add` the Cargo.lock, and remove whatever ignore rule was hiding it.",
        offenders.join("\n")
    );
}

/// The same defect at its source rather than at its symptom.
///
/// A tracked file stays tracked even when an ignore rule matches it, so the assertion above
/// passes today for a lockfile that a single `git rm --cached` would silently un-pin again.
#[test]
fn no_ignore_rule_hides_a_cargo_lock() {
    let repo = repo_root();
    let mut offenders = Vec::new();

    for tree in lockfile_owning_trees(&repo).into_keys() {
        let rel = lock_path_for(&tree, &repo);

        match git_ignore_rule_for(&repo, &rel) {
            Ok(None) => {}
            Ok(Some(rule)) => offenders.push(format!("  {rel}\n    ignored by: {rule}")),
            Err(err) => panic!("git could not answer whether {rel} is ignored: {err}"),
        }
    }

    assert!(
        offenders.is_empty(),
        "an ignore rule matches these lockfiles (#446). They may be tracked today, but the \
         rule is a loaded gun: the next `git rm --cached`, or the next tree created by \
         copying this one, un-pins the build silently.\n{}\n\nFix: delete the rule — it is in \
         a tracked .gitignore or in .git/info/exclude. If a lockfile is genuinely not wanted \
         for a tree, that decision needs to be visible here, not in a file nobody opens.",
        offenders.join("\n")
    );
}
