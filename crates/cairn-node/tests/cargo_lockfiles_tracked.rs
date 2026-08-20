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
//! and nothing reported which one had happened. Two of the repo's three cargo trees were
//! pinned; the unpinned one was the most safety-critical of them.
//!
//! # Why a guard rather than vigilance
//!
//! It hid for months behind two lines of `.gitignore` nobody had a reason to open, and the
//! symptom when it finally bit was a required check failing in 45 seconds on a docs-only
//! commit — which reads as CI flakiness, not as an unpinned build. This repo also has its own
//! history of "pinned in two places, drifted apart" defects (#182, #189, #404), and a fourth
//! cargo tree will eventually be added by someone who is not thinking about any of this.
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
//! because nobody has a reason to `cd` there.
//!
//! # What this guard does NOT claim
//!
//! It does not check that a lockfile is *current* (`cargo update` drift is `--locked`'s job,
//! and CI passes it), nor that its contents are sound (that is cargo-deny's job, `deny.toml`).
//! It checks the one property whose absence is silent: that a build in CI resolves from a
//! pinned set rather than from whatever crates.io serves that minute.
//!
//! Both tests here are pure source/VCS inspection — no database, so they run in every
//! `cargo test`, not only the DB-gated slice.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The repository root — two levels up from `crates/cairn-node`.
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

/// Run a program in the repo root and return `(exit ok, stdout, stderr)`.
///
/// Kept as one small helper rather than three call-site copies so that "which directory did
/// this run in" has a single answer: a `git` invocation that silently ran somewhere else
/// would report about the wrong repository and still look green.
fn run(program: &str, args: &[&str]) -> (bool, String, String) {
    let out = Command::new(program)
        .args(args)
        .current_dir(repo_root())
        .output()
        .unwrap_or_else(|e| panic!("failed to run `{program} {}`: {e}", args.join(" ")));
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
        String::from_utf8_lossy(&out.stderr).trim().to_string(),
    )
}

/// Every `Cargo.toml` the repository **tracks**, as paths relative to the repo root.
///
/// Tracked rather than "found on disk" is deliberate and load-bearing twice over. It skips
/// build output and any local scratch checkout (`.claude/worktrees/*` holds several full
/// copies of this tree, each with its own manifests), and — the important half — it means
/// this guard reasons about the same file set a fresh CI clone sees, which is the set the
/// incident turned on.
fn tracked_manifests() -> Vec<PathBuf> {
    let (ok, stdout, stderr) = run("git", &["ls-files", "-z", "*Cargo.toml"]);
    assert!(ok, "git ls-files failed: {stderr}");
    stdout
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .collect()
}

/// The workspace root cargo resolves for `manifest`, or the error cargo reports.
///
/// `--offline` is passed because the answer is a pure function of the manifests on disk;
/// without it a future cargo could decide to touch the network and make this guard flaky for
/// a reason unrelated to what it tests.
fn workspace_root_for(manifest: &Path) -> Result<PathBuf, String> {
    let manifest = manifest.to_string_lossy().to_string();
    let (ok, stdout, stderr) = run(
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
    if ok {
        Ok(PathBuf::from(stdout))
    } else {
        Err(stderr)
    }
}

/// The repo-relative path of the `Cargo.lock` a tree root owns.
///
/// Both tests below need exactly this, and both then hand the result to `git`, which wants a
/// path relative to the repository — so getting it right once matters more than it looks: an
/// absolute path here would still work for `ls-files --error-unmatch` but would silently
/// change what `check-ignore` matches against.
fn lock_path_for(tree_root: &Path, repo: &Path) -> String {
    let lock = tree_root.with_file_name("Cargo.lock");
    lock.strip_prefix(repo)
        .unwrap_or(&lock)
        .to_string_lossy()
        .to_string()
}

/// Map each lockfile-owning tree to one manifest that resolved to it.
///
/// The manifest is carried alongside so a failure can name a path a reader recognises: for a
/// tree whose root manifest is not itself tracked, the root alone would be the only clue.
fn lockfile_owning_trees() -> BTreeMap<PathBuf, PathBuf> {
    let mut trees = BTreeMap::new();
    let mut unplaceable = Vec::new();

    for manifest in tracked_manifests() {
        match workspace_root_for(&manifest) {
            Ok(root) => {
                trees.entry(root).or_insert(manifest);
            }
            Err(err) => unplaceable.push(format!("  {}\n    {err}", manifest.display())),
        }
    }

    assert!(
        unplaceable.is_empty(),
        "cargo cannot place these tracked manifests in any workspace, so no cargo command \
         works in their directory (#446):\n{}\n\nFix: either add the package to the enclosing \
         workspace's `members`, or detach it with an empty `[workspace]` table in its own \
         manifest (the way poc/iced-ui-spike does).",
        unplaceable.join("\n")
    );

    trees
}

/// The load-bearing assertion: every tree that resolves its own dependency set does so from a
/// lockfile a fresh clone will have.
#[test]
fn every_cargo_tree_has_a_tracked_lockfile() {
    let root = repo_root();
    let mut offenders = Vec::new();

    for (tree, via) in lockfile_owning_trees() {
        let rel = lock_path_for(&tree, &root);

        // Tracked is the property that matters, not present: a gitignored lockfile is present
        // on every developer machine and absent in CI, which is exactly the state that hid
        // the cairn_pgx gap for months. `--error-unmatch` is git's own answer to "is this
        // path in the index", so this cannot disagree with what a clone would contain.
        let (tracked, _, _) = run("git", &["ls-files", "--error-unmatch", &rel]);
        if !tracked {
            offenders.push(format!(
                "  {rel}\n    (cargo tree reached via {})",
                via.display()
            ));
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
/// `--no-index` is what makes `git check-ignore` answer about the *rules* rather than about
/// the current index — without it, git skips tracked paths and this test would report that
/// nothing is ignored no matter what the `.gitignore` files say. That is the failure mode
/// worth stating outright: the polarity mistake here does not make the guard noisy, it makes
/// it vacuous.
#[test]
fn no_ignore_rule_hides_a_cargo_lock() {
    let root = repo_root();
    let mut offenders = Vec::new();

    for tree in lockfile_owning_trees().into_keys() {
        let rel = lock_path_for(&tree, &root);

        let (ignored, stdout, _) = run("git", &["check-ignore", "--no-index", "-v", &rel]);
        if ignored {
            offenders.push(format!("  {rel}\n    ignored by: {stdout}"));
        }
    }

    assert!(
        offenders.is_empty(),
        "an ignore rule matches these lockfiles (#446). They may be tracked today, but the \
         rule is a loaded gun: the next `git rm --cached`, or the next tree created by \
         copying this one, un-pins the build silently.\n{}\n\nFix: delete the rule. If a \
         lockfile is genuinely not wanted for a tree, that decision needs to be visible \
         here, not in a .gitignore nobody opens.",
        offenders.join("\n")
    );
}
