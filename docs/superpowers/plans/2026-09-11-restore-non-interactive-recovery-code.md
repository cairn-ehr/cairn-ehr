# The restore's recovery code gets a non-interactive path — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give `cairn-node restore` a way to read the OLD node's recovery code from a file, so a
disaster-recovery drill can be scripted and the restore CLI surface becomes testable at all.

**Architecture:** One optional flag, `--old-recovery-code-file <PATH>`, resolved in the restore arm's
existing step-0 pre-flight block before anything is minted. The pure read-and-validate logic lives in a
new library module so its tests are real integration tests rather than `#[cfg(test)]` blocks inside a
binary crate nothing can import. The existing injected-`ask` seam in `unseal_local_state_with_retries`
carries the supplied code with no new machinery. A separate `IsTerminal`-keyed warning reports, without
claiming to prevent, the freshly-minted recovery code that a non-interactive restore writes to stderr.

**Tech Stack:** Rust (`cairn-node`), clap 4 derive, `thiserror`, `zeroize::Zeroizing`, `std::io::IsTerminal`.
Python 3 for `scripts/measure_dr_restore.py`. PostgreSQL 18 for the DB-gated tests. **No new dependency
is added by this plan.**

**Spec:** [`docs/superpowers/specs/2026-09-11-restore-non-interactive-recovery-code-design.md`](../specs/2026-09-11-restore-non-interactive-recovery-code-design.md)

## Global Constraints

- **AGPL-3.0.** Every dependency must be AGPL-3.0-compatible. This plan adds none. `IsTerminal` is std.
- **TDD, without exception.** The failing test first, run it, watch it fail for the stated reason, then
  the minimal code. This is the §9 safety-critical surface: a defect here costs a clinic its record.
- **Inline documentation for a junior developer.** Every non-trivial function carries *why it exists and
  how it fits*, not a restatement of the next line. House rule 3, and the §9 reviewer-legibility rule.
- **Files stay under 500 lines where feasible.** `main.rs` is 6405, `restore.rs` 601,
  `restore/clinical.rs` 620. None of them may absorb this work; that is why Task 1 creates a module.
- **Never hard-code cryptographic material in tests, and never give a non-cryptographic value a
  cryptographic name** (house rule 6). Recovery codes in fixtures are **derived at runtime**. The words
  `salt`, `nonce`, `iv` are reserved for real constructions. `crates/cairn-node/tests/crypto_sink_names_are_genuine.rs`
  enforces the second half.
- **No migration, no `SCHEMA_GENERATION` bump, no wire-format change.** If a task seems to need one,
  stop: the design is wrong, not the schema.
- **DB-gated tests** read `CAIRN_TEST_PG` and take `db::test_serial_guard(&base)`. A DB-free
  `cargo test` needs `CAIRN_ALLOW_DB_SKIP=1` since #450.
- **The gate is the FULL workspace.** `cargo test --workspace`, not `-p cairn-node`. A per-crate run
  misses cross-crate call sites; that is #503's lesson and why `cairn-sync/tests/clinical_pull.rs` exists.
  Never pipe it through `tail` — that masks cargo's exit code.
- **Commit messages** end with `Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>`.
  Use the `fix(#572):` / `test(#570):` form: the parenthesis breaks GitHub's closing-keyword adjacency,
  which is what stopped seven issues being closed by sentences that disclaimed closing them.

---

## File Structure

**Created:**

| Path | Responsibility |
|---|---|
| `crates/cairn-node/src/restore/recovery_code.rs` | Pure: read and validate a supplied recovery code, decide the retry budget, word the exposure warning. No I/O beyond one file read, no DB, no tty. |
| `crates/cairn-node/tests/recovery_code_file.rs` | The pure module's behaviour, driven from outside the crate. |
| `crates/cairn-node/tests/restore_cli_surface.rs` | #570: the shipped `restore` command driven as a subprocess — the non-zero exit, warning reachability, and the non-interactive restore that proves #572 closed. |
| `docs/spec/decisions/0069-the-restore-takes-its-recovery-code-from-a-file.md` | The ADR. |

**Modified:**

| Path | Change |
|---|---|
| `crates/cairn-node/src/restore.rs` | One `pub mod recovery_code;` declaration with its rationale. |
| `crates/cairn-node/src/main.rs` | The clap arg; the step-0 read; the `ask` wiring; the step-4 warning; `RECOVERY_CODE_ATTEMPTS` re-homed. |
| `crates/cairn-node/tests/restore_needs_nothing_about_the_dead_node.rs` | The module doc's "deliberately NOT pinned" paragraph is now false; add the secret-half pin. |
| `crates/cairn-node/src/localstate.rs` | Nothing functional. Task 7 adds tests only. |
| `scripts/measure_dr_restore.py` | Drop the pseudo-terminal. |
| `docs/spec/index.md`, `docs/spec/decisions/README.md`, `mkdocs.yml` | Spec version bump, ADR index row, nav entry. |
| `docs/HANDOVER.md`, `docs/ROADMAP.md` | Session record, and both pruned. |

---

## Task 1: The pure recovery-code module

**Files:**
- Create: `crates/cairn-node/src/restore/recovery_code.rs`
- Create: `crates/cairn-node/tests/recovery_code_file.rs`
- Modify: `crates/cairn-node/src/restore.rs` (add the `pub mod` declaration beside `pub mod clinical;` at line 26)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces, all `pub` from `cairn_node::restore::recovery_code`:
  - `const RECOVERY_CODE_ATTEMPTS: usize` (value `3`)
  - `enum RecoveryCodeError { Unreadable { path: PathBuf, source: std::io::Error }, Blank { path: PathBuf } }`
  - `fn read_recovery_code_file(path: &Path) -> Result<Zeroizing<String>, RecoveryCodeError>`
  - `fn recovery_code_attempts(supplied: bool) -> usize`
  - `fn minted_code_exposure_warning() -> String`

- [ ] **Step 1: Write the failing test**

Create `crates/cairn-node/tests/recovery_code_file.rs`:

```rust
//! #572 — the OLD node's recovery code can reach `restore` from a file.
//!
//! These are the pure halves of the decision: what a supplied code file may contain, how many
//! times the unseal loop may ask given where the code came from, and the wording of the
//! warning that reports a freshly-minted code landing somewhere no human is reading.
//!
//! They live in the LIBRARY rather than in `main.rs`, so an integration test can import them.
//! `Cmd` is defined in a binary crate, which is why the `--help` surface has to be driven as a
//! subprocess; pure functions have no such excuse.

use cairn_node::restore::recovery_code::{
    minted_code_exposure_warning, read_recovery_code_file, recovery_code_attempts,
    RecoveryCodeError, RECOVERY_CODE_ATTEMPTS,
};

/// Write `contents` to a scratch file and hand back the directory guard with the path.
///
/// The guard is returned alongside the path deliberately: `TempDir` deletes its tree on drop,
/// so a helper that returned the path alone would hand back a path to a directory that had
/// already been removed.
fn code_file(contents: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old-recovery-code");
    std::fs::write(&path, contents).unwrap();
    (dir, path)
}

/// A recovery code, derived at runtime rather than written as a literal (house rule 6a).
///
/// The shape mirrors `generate_recovery_code`'s output closely enough for the tests that
/// matter — grouped uppercase alphanumerics — without importing the generator, because these
/// tests are about the FILE, not about the code's alphabet.
fn a_recovery_code() -> String {
    let alphabet: Vec<char> = ('A'..='Z').chain('2'..='7').collect();
    (0..32)
        .map(|i| alphabet[(i * 7 + 3) % alphabet.len()])
        .collect()
}

#[test]
fn a_file_holding_a_code_yields_that_code() {
    let expected = a_recovery_code();
    let (_dir, path) = code_file(&expected);
    let got = read_recovery_code_file(&path).unwrap();
    assert_eq!(&*got, &expected);
}

/// `printf '%s\n' "$CODE" > file` is how anyone would write one of these, and
/// `normalize_recovery_code` strips the newline before the unwrap anyway. Refusing it would be
/// a trap with no upside.
#[test]
fn a_trailing_newline_is_tolerated() {
    let expected = a_recovery_code();
    let (_dir, path) = code_file(&format!("{expected}\n"));
    let got = read_recovery_code_file(&path).unwrap();
    assert_eq!(&*got, &expected);
}

/// NOT cosmetic. `normalize_recovery_code` strips all spacing and case before the unwrap, so a
/// file holding only spaces normalizes to the empty string and would attempt an unseal under an
/// effectively empty secret. `establish-local-state-key` already guards exactly this.
#[test]
fn a_whitespace_only_file_is_refused_and_says_why() {
    let (_dir, path) = code_file("   \n\t ");
    let err = read_recovery_code_file(&path).unwrap_err();
    assert!(
        matches!(err, RecoveryCodeError::Blank { .. }),
        "a blank file must be its own error, not an I/O one: {err:?}"
    );
    let text = err.to_string();
    assert!(
        text.contains("blank") || text.contains("no recovery code"),
        "the message must name the real problem: {text}"
    );
}

/// A missing path is the drill author's most likely mistake, and it must be
/// DISTINGUISHABLE from a blank file: one means "fix your script", the other means "the
/// secret you saved is not there".
#[test]
fn a_missing_path_is_refused_distinguishably() {
    let dir = tempfile::tempdir().unwrap();
    let err = read_recovery_code_file(&dir.path().join("absent")).unwrap_err();
    assert!(
        matches!(err, RecoveryCodeError::Unreadable { .. }),
        "a missing file must not be reported as a blank one: {err:?}"
    );
    assert!(
        err.to_string().contains("absent"),
        "the message must name the path the operator typed: {err}"
    );
}

/// Re-reading a file cannot change its contents, so a budget of three would print
/// "2 attempt(s) left" about a file. The prompt keeps its retries because a human can
/// genuinely type a different thing the second time.
#[test]
fn a_supplied_code_is_asked_once_and_a_prompt_keeps_its_retries() {
    assert_eq!(recovery_code_attempts(true), 1);
    assert_eq!(recovery_code_attempts(false), RECOVERY_CODE_ATTEMPTS);
    assert!(RECOVERY_CODE_ATTEMPTS > 1, "the prompt must have retries at all");
}

/// The warning REPORTS an exposure; it must not imply it prevented one. A future reader who
/// takes it as a guarantee would stop looking for the real fix.
#[test]
fn the_exposure_warning_reports_rather_than_reassures() {
    let text = minted_code_exposure_warning();
    assert!(text.contains("stderr"), "it must name the stream: {text}");
    assert!(
        text.to_lowercase().contains("recovery code"),
        "it must name what leaked: {text}"
    );
    for reassurance in ["prevented", "suppressed", "safe", "withheld"] {
        assert!(
            !text.to_lowercase().contains(reassurance),
            "the warning must not claim to have prevented anything ({reassurance:?}): {text}"
        );
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p cairn-node --test recovery_code_file 2>&1 | tail -30`
Expected: FAIL to compile, `unresolved import cairn_node::restore::recovery_code`.

- [ ] **Step 3: Write the minimal implementation**

Create `crates/cairn-node/src/restore/recovery_code.rs`:

```rust
//! #572 — how the OLD node's recovery code reaches a restore when no human is at the terminal.
//!
//! # What problem this solves
//!
//! `restore` needs two secrets and used to treat them unalike. The passphrase for the NEW
//! sealed key has `--passphrase` and `CAIRN_KEY_PASSPHRASE`. The OLD node's recovery code —
//! which unseals the local-state export, and is therefore the thing that returns the dead
//! node's custody — had no flag and no environment variable at all. It was read through
//! `rpassword::prompt_password`, which opens `/dev/tty` and fails on any non-tty.
//!
//! A piped code did not merely get ignored: the read errored, the export never opened, and the
//! restore finished having recovered ZERO PATIENTS while exiting non-zero. So a clinic could
//! not rehearse its disaster recovery, and the one CLI surface whose correctness matters most
//! could not be tested at all (#570 is the same wall).
//!
//! # Why a FILE, and not a flag value or an environment variable
//!
//! The two secrets are not alike. The passphrase is invented at restore time and protects a key
//! that has not existed for ten seconds. The recovery code is the single RETAINED off-node
//! artifact, and together with the medium sitting beside it, it yields the clinic's whole
//! clinical record in the clear. Consistency with `CAIRN_KEY_PASSPHRASE` is not a strong enough
//! reason to give both the same exposure.
//!
//! A path keeps the secret off the process table (`ps auxww` shows the path, never the code),
//! out of shell history, and out of the environment — so not in `/proc/<pid>/environ`, not
//! inherited by child processes, and not in a crash dump. It also composes for free: a tmpfs
//! path, a named pipe and `/dev/stdin` all work with no extra code here.
//!
//! # Why these functions are PURE and live in the library
//!
//! `Cmd` is defined in a binary crate, so integration tests cannot import it — which is why the
//! `--help` surface has to be driven as a subprocess. Pure functions have no such excuse, and
//! the validation below is exactly the kind that must be provable without a database or a tty.
//!
//! See `docs/spec/decisions/0069-the-restore-takes-its-recovery-code-from-a-file.md`.

use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

/// How many times the unseal loop may re-ask a HUMAN for the old recovery code.
///
/// Lives here rather than in `main.rs` so that [`recovery_code_attempts`] — the function that
/// decides between this and one — can be tested without spawning a binary. The budget exists
/// because the prompt lands after `finalize_identity` has fenced the restore door: before the
/// retries, a single mistyped character cost the node its custody key outright.
pub const RECOVERY_CODE_ATTEMPTS: usize = 3;

/// Why an operator-supplied recovery-code file could not be used.
///
/// The two variants are deliberately distinct, because they send the operator to different
/// places. `Unreadable` means "fix your script or your mount". `Blank` means "the file you
/// saved your only off-node secret into has nothing in it", which is a far worse morning.
#[derive(thiserror::Error, Debug)]
pub enum RecoveryCodeError {
    #[error("could not read the recovery-code file at {path} ({source})")]
    Unreadable {
        path: PathBuf,
        source: std::io::Error,
    },
    /// Not merely cosmetic — see [`read_recovery_code_file`] for why a blank file is dangerous
    /// rather than just useless.
    #[error(
        "the recovery-code file at {path} is blank (no recovery code in it). Refusing to \
         continue: `normalize_recovery_code` strips all spacing before the unwrap, so a file \
         of whitespace would attempt to open the export under an EMPTY secret and report the \
         resulting failure as a wrong code."
    )]
    Blank { path: PathBuf },
}

/// Read the OLD node's recovery code out of a file the operator named.
///
/// **The trailing newline is tolerated on purpose.** `printf '%s\n' "$CODE" > file` is how
/// anyone would write one of these, and `cairn_keystore::seal::normalize_recovery_code` strips
/// spacing and case before the unwrap regardless. Refusing it would be a trap with no upside.
///
/// **A whitespace-only file is refused on purpose, and this is the load-bearing half.** That
/// same normalization turns `"   "` into the empty string, so such a file would sail into an
/// unseal under an effectively empty secret and come back as `None` — which is bit-for-bit
/// the answer a WRONG code gives. The operator would then be told their code was wrong, or
/// that their export might be damaged, and would go hunting for a code they had in fact saved
/// correctly. `establish-local-state-key` already guards exactly this input for exactly this
/// reason.
///
/// The result is `Zeroizing<String>` so the secret is wiped from the heap on drop (issue #46),
/// matching every other secret this binary handles.
pub fn read_recovery_code_file(path: &Path) -> Result<Zeroizing<String>, RecoveryCodeError> {
    let raw = Zeroizing::new(std::fs::read_to_string(path).map_err(|source| {
        RecoveryCodeError::Unreadable {
            path: path.to_path_buf(),
            source,
        }
    })?);
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(RecoveryCodeError::Blank {
            path: path.to_path_buf(),
        });
    }
    Ok(Zeroizing::new(trimmed.to_string()))
}

/// How many times the unseal loop may ask, given where the code comes from.
///
/// A SUPPLIED code is asked once: re-reading the same file cannot change the answer, and a
/// budget of three would print "2 attempt(s) left" about a file, which tells an operator
/// nothing and invites them to wait for a prompt that will never come.
///
/// A PROMPTED code keeps its full budget, because a human can genuinely type a different
/// thing the second time — which is the entire reason the budget exists.
pub fn recovery_code_attempts(supplied: bool) -> usize {
    if supplied {
        1
    } else {
        RECOVERY_CODE_ATTEMPTS
    }
}

/// The warning printed when a freshly-minted recovery code lands on a stream no human is reading.
///
/// **This REPORTS an exposure. It does not prevent one, and the wording must never suggest it
/// does** — a reader who takes it as a guarantee stops looking for the real fix.
///
/// The exposure is older and wider than the flag this module adds. A medium with no local-state
/// export sibling never reaches the recovery-code prompt at all, so a sealed restore of one has
/// always been able to run unattended and print a fresh code to stderr. That is why the caller
/// keys this on whether stderr is a terminal rather than on which flags were passed: the honest
/// question is "will a human see this code?", and it covers both paths.
pub fn minted_code_exposure_warning() -> String {
    "WARNING: the recovery code above was written to stderr, and stderr is not a terminal. \
     Whatever captured this stream — a log file, a cron mail, a CI artifact — now holds the \
     only off-node secret that recovers this node's signing key. Treat that capture as secret, \
     or re-run this restore attended."
        .to_string()
}
```

Then add to `crates/cairn-node/src/restore.rs`, immediately after the `pub mod clinical;`
declaration at line 26:

```rust
/// How the OLD node's recovery code reaches a restore that has no terminal (#572). Kept in its
/// own file for the same two reasons `clinical` is: `restore.rs` is already at the crate's size
/// cap (house rule 4), and these are pure functions whose whole value is being testable without
/// a database, a tty or a spawned binary.
pub mod recovery_code;
```

- [ ] **Step 3a: Check the crypto-sink-name guard**

Run: `cargo test -p cairn-node --test crypto_sink_names_are_genuine 2>&1 | tail -20`
Expected: PASS. Nothing here is named `salt`, `nonce` or `iv`, so no `ALLOWED` entry is owed.
If it fails, a name in the new module collided with a CodeQL sink — rename the binding, do not
widen `ALLOWED`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p cairn-node --test recovery_code_file 2>&1 | tail -20`
Expected: PASS, 7 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-node/src/restore/recovery_code.rs \
        crates/cairn-node/src/restore.rs \
        crates/cairn-node/tests/recovery_code_file.rs
git commit -m "feat(#572): a recovery code can come from a file, and a blank one is refused

...body per the design's 3.1 and 5.2...

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: The flag, the step-0 read, and the retry wiring

**Files:**
- Modify: `crates/cairn-node/src/main.rs` — the `Cmd::Restore` clap variant (around line 1426), the
  step-0 pre-flight block (around line 2886), `apply_local_state_export` (line 1087) and its sole
  caller (line 3155), and the two `RECOVERY_CODE_ATTEMPTS` uses (lines 1116, 1127).
- Modify: `crates/cairn-node/tests/restore_needs_nothing_about_the_dead_node.rs`

**Interfaces:**
- Consumes: `cairn_node::restore::recovery_code::{read_recovery_code_file, recovery_code_attempts, RECOVERY_CODE_ATTEMPTS}` from Task 1.
- Produces: the `--old-recovery-code-file` surface that Tasks 4–6 drive, and
  `apply_local_state_export`'s new fifth parameter `supplied_code: Option<&Zeroizing<String>>`,
  inserted **before** `new_secrets`.

- [ ] **Step 1: Write the failing test**

In `crates/cairn-node/tests/restore_needs_nothing_about_the_dead_node.rs`, replace the module
doc's final paragraph (the one beginning "**The secret half is deliberately NOT pinned here**")
with:

```rust
//! **The secret half is pinned now, and it could not be before (#572).** `restore` needs two
//! secrets. The passphrase for the NEW key is invented at restore time, so it is not
//! *retained* and the budget's "one secret" was never about it; it has both `--passphrase` and
//! `CAIRN_KEY_PASSPHRASE`. The OLD node's recovery code IS the retained one, and until #572 it
//! had no flag and no environment variable at all — it was read through `rpassword`, which
//! fails on any non-tty. Pinning the clause then would have pinned the defect. It now has
//! `--old-recovery-code-file`, and the two properties that matter are asserted below: the flag
//! EXISTS, and it is OPTIONAL, so a solo clinic still supplies nothing but `--conn` and
//! `--from`.
```

Then add these two tests to the same file:

```rust
/// #572: the retained secret has a non-interactive path at all. Before this existed, a DR
/// drill could not be scripted and this very CLI surface could not be tested.
#[test]
fn the_retained_secret_has_a_non_interactive_path() {
    let help = restore_help();
    assert!(
        documented_flags(&help).iter().any(|f| f == "--old-recovery-code-file"),
        "restore must offer a non-interactive path for the OLD node's recovery code; \
         documented flags were {:?}",
        documented_flags(&help)
    );
}

/// And it must stay OPTIONAL. A required flag here would break the clause this whole file
/// exists to defend: the operator whose disk just died supplies the new database and the
/// medium, and nothing else.
#[test]
fn the_recovery_code_file_is_optional() {
    let help = restore_help();
    assert!(
        !required_flags(&help).iter().any(|f| f == "--old-recovery-code-file"),
        "the recovery-code file must be optional — required flags were {:?}",
        required_flags(&help)
    );
    assert_eq!(
        unexpected_required(&help),
        Vec::<String>::new(),
        "restore may demand only {PERMITTED_REQUIRED:?}"
    );
}
```

> If the existing file spells the help-fetching helper differently from `restore_help()`, use
> the name it already has. Read the file's existing tests before writing these two; do not add
> a second helper that duplicates one.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p cairn-node --test restore_needs_nothing_about_the_dead_node 2>&1 | tail -30`
Expected: `the_retained_secret_has_a_non_interactive_path` FAILS because the flag is not
documented. `the_recovery_code_file_is_optional` PASSES vacuously — that is expected and fine,
it becomes meaningful once the flag exists.

- [ ] **Step 3: Add the flag**

In `main.rs`, inside the `Restore { .. }` variant, after `passphrase`:

```rust
        /// Read the OLD node's recovery code from this file instead of prompting for it,
        /// so a disaster-recovery drill can be scripted (#572). The file's contents are the
        /// code; a trailing newline is fine. A path rather than a flag value or an env var
        /// deliberately: this is the one RETAINED off-node secret, and a path keeps it off
        /// the process table, out of shell history and out of the environment. `/dev/stdin`
        /// and named pipes work. NOT to be confused with the NEW code this command mints
        /// and prints — hence "old".
        #[arg(long)]
        old_recovery_code_file: Option<PathBuf>,
```

Add `old_recovery_code_file,` to the `Cmd::Restore { .. }` destructuring at line 2877.

- [ ] **Step 4: Read it in the step-0 pre-flight block**

In the `Cmd::Restore` arm, inside the numbered comment block `// 0. PRE-FLIGHT, before a single
byte is minted or written.`, after the `export_bytes` match and before step 1's medium read:

```rust
            //    #572. Read a supplied recovery code HERE, in the pre-flight, for the same
            //    reason the two checks above live here: by the time the unseal runs,
            //    `finalize_identity` has fenced the restore door and there is no free second
            //    attempt. A drill script pointed at a path that does not exist should cost
            //    nothing; before this, the equivalent mistake cost an identity and a database.
            let supplied_code = match &old_recovery_code_file {
                Some(path) => Some(cairn_node::restore::recovery_code::read_recovery_code_file(
                    path,
                )?),
                None => None,
            };
            //    A supplied code with NO export beside the medium is inert. Warn rather than
            //    fail: a drill script pointed at the wrong medium would otherwise pass while
            //    exercising none of the path it exists to exercise — a green run that proves
            //    nothing is worse than a red one.
            if supplied_code.is_some() && export_bytes.is_none() {
                eprintln!(
                    "WARNING: --old-recovery-code-file was given, but no local-state export \
                     sits beside {}. Nothing will be unsealed and no custody will be \
                     inherited. If this is a drill, it is not exercising the path you think \
                     it is.",
                    from.display()
                );
            }
```

- [ ] **Step 5: Thread it to the unseal loop**

Change `apply_local_state_export`'s signature (line 1087) to insert the parameter before
`new_secrets`:

```rust
async fn apply_local_state_export(
    db: &tokio_postgres::Client,
    bytes: &[u8],
    export_path: &std::path::Path,
    unwrap_path: &std::path::Path,
    supplied_code: Option<&Zeroizing<String>>,
    new_secrets: Option<&(Zeroizing<String>, Zeroizing<String>)>,
) -> anyhow::Result<Option<cairn_node::localstate::AppliedLocalState>> {
```

Replace the `eprintln!` + `unseal_local_state_with_retries` block (lines 1114–1132) with:

```rust
    // #572: the code may already be in hand, read from a file during the pre-flight. The
    // ASK is what differs; the loop, the degradation and every message below are shared, so
    // a scripted restore and an attended one cannot drift apart.
    let attempts = cairn_node::restore::recovery_code::recovery_code_attempts(
        supplied_code.is_some(),
    );
    match supplied_code {
        Some(_) => eprintln!(
            "Local-state export found. Using the OLD node's recovery code from the file given."
        ),
        None => eprintln!(
            "Local-state export found. Enter the OLD node's recovery code to unseal it:"
        ),
    }
    // Bounded re-prompt, because this prompt is past the point of no return — see
    // `unseal_local_state_with_retries`. A wrong code still degrades the same way in the end
    // (warn + skip): a bad guess at the OPTIONAL local-state must not kill an otherwise
    // complete restore. A SUPPLIED code gets one attempt, since re-reading a file cannot
    // change the answer.
    let plaintext = unseal_local_state_with_retries(&sealed, attempts, |attempt| {
        match supplied_code {
            Some(code) => Ok(code.clone()),
            None => Ok(Zeroizing::new(rpassword::prompt_password(
                if attempt == 1 {
                    "old recovery code: "
                } else {
                    "old recovery code (try again): "
                },
            )?)),
        }
    })?;
    let Some(plaintext) = plaintext else {
        warn_no_custody_key_installed(
            &unsealing_failed_cause(export_path, attempts),
            // ... existing remedy text, unchanged ...
        );
        return Ok(None);
    };
```

> Note `unsealing_failed_cause(export_path, attempts)` now takes the RESOLVED count, not the
> constant. A message saying "did not open after 3 attempts" when the file was read once is a
> message that lies, which is the exact defect class 2d's review round spent a round on.

Update the sole caller (line 3155) to pass `supplied_code.as_ref(),` before `new_secrets.as_ref(),`.

Update the two remaining `RECOVERY_CODE_ATTEMPTS` references in `main.rs` to import from the new
module and delete the local `const RECOVERY_CODE_ATTEMPTS: usize = 3;` at line 963.

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p cairn-node --test restore_needs_nothing_about_the_dead_node 2>&1 | tail -20`
Expected: PASS, including the two new tests.

Run: `cargo build -p cairn-node 2>&1 | tail -20`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add crates/cairn-node/src/main.rs \
        crates/cairn-node/tests/restore_needs_nothing_about_the_dead_node.rs
git commit -m "feat(#572): restore takes the old recovery code from a file

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: The exposure warning

**Files:**
- Modify: `crates/cairn-node/src/main.rs` — step 4, immediately after `print_recovery_code(&code);` (line 3096).

**Interfaces:**
- Consumes: `minted_code_exposure_warning()` from Task 1.
- Produces: a stderr line that Task 5 asserts on.

- [ ] **Step 1: Write the failing test**

The pure wording is already pinned by Task 1. What is unpinned is *reachability*, and that is a
CLI concern — it is asserted in Task 5, where a real subprocess has a non-terminal stderr. Add
the assertion there rather than writing a second unit test that cannot see the call site. This
step is therefore: **confirm Task 5's step 1 includes the assertion**, and write no test here.

- [ ] **Step 2: Implement**

Add `use std::io::IsTerminal;` to `main.rs`'s imports, and after `print_recovery_code(&code);`:

```rust
                // #572 / ADR-0069. A restore can now run with no human at the terminal, and
                // the code just printed is the only off-node way to recover this node's key.
                // Keyed on the STREAM rather than on which flags were passed, because that is
                // the honest question — and because the exposure is older and wider than the
                // flag: a medium with no local-state export never reaches the recovery-code
                // prompt at all, so a sealed restore of one has always been able to run
                // unattended and print a code into a log. This REPORTS that; it does not
                // prevent it. The fix is tracked separately.
                if !std::io::stderr().is_terminal() {
                    eprintln!("{}", cairn_node::restore::recovery_code::minted_code_exposure_warning());
                }
```

- [ ] **Step 3: Verify nothing existing breaks**

Run: `CAIRN_TEST_PG="$(scripts/pg-target.sh)" cargo test -p cairn-node --test restore_torn_medium_cli 2>&1 | tail -20`
Expected: PASS. That test uses `--insecure-plaintext`, so no code is minted and the warning does
not fire. If it fails, an assertion somewhere is stricter than `contains` and needs reading.

- [ ] **Step 4: Commit**

```bash
git add crates/cairn-node/src/main.rs
git commit -m "feat(#572): say so when a minted recovery code lands on a stream no human reads

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: The headline CLI test — a scripted restore brings the record back

**Files:**
- Create: `crates/cairn-node/tests/restore_cli_surface.rs`

**Interfaces:**
- Consumes: the flag from Task 2, the warning from Task 3.
- Produces: the fixture helpers Tasks 5 and 6 extend (`cairn_node()`, `drilled_medium()`).

**This is the test that proves #572 is closed.** Everything else in this plan is machinery.

- [ ] **Step 1: Write the failing test**

Create `crates/cairn-node/tests/restore_cli_surface.rs` with a module doc covering: why this
drives the binary (the `main.rs`-only orchestration that library tests cannot reach), why it
reads a body in CLEAR rather than counting rows (the double-wrap of 2d design §2.1 — rows
present, well-formed, right length, counts agreeing, and every chart unopenable), and that it
runs with **no pseudo-terminal**, which is the whole point.

The test body, in order:

1. Gate on `CAIRN_TEST_PG`; take `db::test_serial_guard`.
2. Provision a node: `keystore::generate_plaintext`, then `establish_lsk(op, code)` +
   `serialize_sidecar` + `atomic_write` to `lsk_sidecar_path_for(&key)` — the idiom
   `cli_localstate.rs::write_existing_escrow` already uses. **Derive `code` at runtime**
   (house rule 6a); do not write it as a literal.
3. Author one born-sealed clinical event with real custody, mirroring
   `restore_reads_the_clinical_plane.rs::author_sealed_clinical_event`. Keep the twin text in a
   variable — it is what step 8 asserts on.
4. Run `backup --to <medium> --passphrase <op>` as a subprocess. Assert it exits 0 and that the
   `CAIRNL1` sibling exists at `localstate::localstate_path_for(&medium)`.
5. Wipe to a fresh DR machine (`wipe_to_a_fresh_dr_machine`'s shape).
6. Write `code` to a scratch file.
7. Run, **with no pty**:
   `restore --from <medium> --old-recovery-code-file <codefile> --insecure-plaintext`
8. Assert, in this order and with the failure messages spelling out what each one means:
   - `out.status.success()` — checked FIRST and strictly broader than any summary line,
     because `restore` prints its whole report and only THEN fails.
   - the clinical summary line reports records applied, and `applied > 0`.
   - **a sealed body OPENS**: query the restored `event_clear.twin` for the authored event and
     assert it equals the twin text from step 3. This is the assertion a row count cannot make.

- [ ] **Step 2: Run it to verify it fails**

Run: `CAIRN_TEST_PG="$(scripts/pg-target.sh)" cargo test -p cairn-node --test restore_cli_surface 2>&1 | tail -40`
Expected: FAIL. Before Tasks 1–3 it fails on the unknown flag; after them it should pass. If it
passes on the FIRST run, the test is not exercising what it claims — check that the export
sibling was actually written in step 4 and that step 5 really wiped custody.

- [ ] **Step 3: Make it pass**

No production code should be needed. If it is, the design is wrong somewhere and that is a
finding worth stopping for, not patching around.

- [ ] **Step 4: Commit**

```bash
git add crates/cairn-node/tests/restore_cli_surface.rs
git commit -m "test(#572): a scripted restore brings the record back, and a body opens

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: #570 items 1 and 2 — the non-zero exit and warning reachability

**Files:**
- Modify: `crates/cairn-node/tests/restore_cli_surface.rs`

**Interfaces:**
- Consumes: Task 4's fixture helpers.
- Produces: nothing later tasks depend on.

- [ ] **Step 1: Write the failing tests**

Add to `restore_cli_surface.rs`:

1. **`an_incomplete_clinical_restore_exits_non_zero`.** Build a medium carrying a clinical
   record the restore must refuse, run it, and assert **both** that the exit is non-zero and
   that the clinical summary line still printed. `main.rs`'s comment says "a script must see
   that" and nothing asserts it. The two halves together are the point: the summary prints and
   the process still fails.
2. **`the_minted_code_exposure_warning_is_reachable`.** A restore **without**
   `--insecure-plaintext` (so a sealed key is minted) with `--passphrase` supplied and
   `--old-recovery-code-file`, driven as a subprocess so stderr is a pipe. Assert the warning
   text reaches stderr. This is the only test that can prove Task 3's branch is reachable: the
   existing text-grepping guards stay green if the block is gated behind `if false`.
3. **`a_wrong_code_in_a_file_degrades_like_a_wrong_typed_one`.** Warn, skip local-state, the
   restore still stands, exit non-zero. Assert the message names both causes — a wrong code and
   a damaged export — because `unsealing_failed_cause` cannot tell them apart and says so
   (principle 4). Assert also that it says **one** attempt, not three.

- [ ] **Step 2: Run them to verify they fail**

Run: `CAIRN_TEST_PG="$(scripts/pg-target.sh)" cargo test -p cairn-node --test restore_cli_surface 2>&1 | tail -40`
Expected: the three new tests FAIL for stated reasons; Task 4's passes.

- [ ] **Step 3: Fix what they find**

Any production change here is a **finding**, not a chore. Record it in the commit body and, if it
is out of scope, open an issue (house rule 5).

- [ ] **Step 4: Verify and commit**

```bash
git add crates/cairn-node/tests/restore_cli_surface.rs
git commit -m "test(#570): the restore CLI's exit status and its warnings are reachable

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

## Task 6: #570 item 3 — the registry encoder round-trip

**Files:**
- Modify: `crates/cairn-node/src/localstate.rs` (tests module only, around line 1405)

**Interfaces:**
- Consumes: `actor_registry_rows_to_json` (line 594), `ActorRegistryRow` (line 559).
- Produces: nothing.

- [ ] **Step 1: Write the failing test**

`actor_registry_rows_to_json` has no direct content assertion. The SQL mirror in
`db/tests/052_restore_doors_test.sql` proves the enroll-then-revoke ordering with hand-written
JSON that bypasses the encoder entirely, so `recorded_at` travels
`TIMESTAMPTZ → ::text → String → JSON → ::TIMESTAMPTZ` with nothing asserting it at any point.
A precision or timezone bug there re-authorises a revoked clinician through the door built to
restore the registry.

Add a test that builds an `ActorRegistryRow` with **every** optional field populated —
`op = "supersede"` with a `superseded_by`, and a non-null `pinned` carrying JSON source with an
embedded quote (ADR-0029's agent-actor determinant, and the case the `serde_json`-not-`format!`
comment exists for) — encodes it, parses the JSON back, and asserts field by field:

- `actor_id` and `superseded_by` are **hex**, because the door decodes them through
  `cairn_decode_hex_or_raise`;
- `pinned` is a JSON **string holding JSON source**, not a nested object;
- `recorded_at` survives **byte for byte**, including sub-second precision and offset;
- an absent optional is **omitted**, not `null`.

Also add the negative: a row whose `pinned` contains a quote produces parseable JSON.

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p cairn-node --lib localstate 2>&1 | tail -30`
Expected: FAIL if any of the above is wrong. **If every assertion passes first time, that is a
real result** — the encoder is correct and was merely unpinned. Say so in the commit body rather
than inventing a failure.

- [ ] **Step 3: Fix anything it finds, then commit**

```bash
git add crates/cairn-node/src/localstate.rs
git commit -m "test(#570): the registry encoder is pinned through the Rust path

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

## Task 7: The rig drops its pseudo-terminal

**Files:**
- Modify: `scripts/measure_dr_restore.py`

- [ ] **Step 1: Replace `restore_under_pty` with a plain subprocess**

Write the recovery code to a scratch file and pass `--old-recovery-code-file`. Delete the `pty`
import, the prompt-matching logic and the three-attempt answering loop — every one of them exists
only to work around #572, and the review round that hardened them (matching `"old recovery code"`
rather than `"recovery code"`, answering up to three tries) is now moot.

Rewrite the "# The pseudo-terminal, and why it is here" section as the record of a **closed** gap,
naming ADR-0069. Keep the history: it explains why the rig is shaped as it is.

**Keep the exit-status check and the incomplete-restore refusal.** Neither had anything to do with
the pty. A restore that applies nothing is fast, and a rig that timed it would write a flattering
wrong number into a dated file that outlives the session.

- [ ] **Step 2: Run the rig's own suite**

Run: `uv run --with pytest pytest scripts/ -q 2>&1 | tail -20`

> Use `uv`, never `venv`/`pip`. If the rig's tests live elsewhere, find them first —
> `grep -rl measure_dr_restore` — and run those.

Expected: PASS. **If a mutant survives the change, that is a finding**: the suite was mutation-tested
during #512 precisely so a transposed column could not hide, and a change to how the rig invokes the
binary must not blind it.

- [ ] **Step 3: Prove it end to end at a small size**

Run: `python3 scripts/measure_dr_restore.py --sizes 100`
Expected: it completes with no pty and reports a plausible time and a non-zero applied count.

> **Do NOT re-run the published curve and do NOT touch
> `crates/cairn-node/results/2026-09-10-macos-m3max.md`.** This slice changes how a secret arrives,
> not what a restore costs. Restating a number this slice did not change is how a results file drifts
> from what it measured.

- [ ] **Step 4: Commit**

```bash
git add scripts/measure_dr_restore.py
git commit -m "fix(#572): the measurement rig no longer needs a pseudo-terminal

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

## Task 8: ADR-0069, the spec bump, the filed issue, and the tracking documents

**Files:**
- Create: `docs/spec/decisions/0069-the-restore-takes-its-recovery-code-from-a-file.md`
- Modify: `docs/spec/decisions/README.md`, `docs/spec/index.md`, `mkdocs.yml`, `docs/HANDOVER.md`, `docs/ROADMAP.md`

- [ ] **Step 1: Write ADR-0069**

Follow the house ADR structure (read ADR-0068 for the current shape). It must carry:

1. **The decision.** `--old-recovery-code-file`, a path and not a flag value or an env var, with
   §3.1's asymmetry table as the reasoning. The name says "old" because this command mints and
   prints a new code, and the #512 rig's own type-ahead bug is the evidence that the ambiguity
   bites.
2. **What was rejected and why**, including the refusal this design started with. ADRs are the
   home of *why*, and a future session will otherwise re-propose it.
3. **The correction to #527/#562's triage note**, stated as a correction. *"No cron-run command
   reaches `print_recovery_code`"* was **already false** before this slice: a medium with no
   local-state export sibling never reaches the prompt, so a sealed restore of one has always
   run unattended. A reader who finds that sentence beside this slice's date must not conclude
   this slice broke it.
4. **What this does NOT fix**, in as many words: the minted code still goes to stderr on both
   paths, and the warning reports rather than prevents.

- [ ] **Step 2: Bump the spec version and wire the nav**

`docs/spec/index.md` v0.70 → v0.71. Add the ADR-0069 row to `docs/spec/decisions/README.md`.
**Add the ADR to `mkdocs.yml`'s nav** — nav omissions fail the strict build since #573, and
that guard exists because ADR-0067 shipped un-navigated.

Run: `uv run --with-requirements docs/requirements.txt -- mkdocs build --strict 2>&1 | tail -20`
Expected: clean. Always the **pinned** requirements file, never an ad-hoc `--with`.

- [ ] **Step 3: File the follow-up issue**

Title: *the freshly-minted recovery code still goes to stderr, on both restore paths*.

Body must carry: the two reachable paths (this slice's flag, and the older federation-only
medium with no export sibling); that the warning reports rather than prevents; the two candidate
fixes (`--new-recovery-code-file <PATH>`, or refusing to mint a sealed key when nothing can show
its code to a human); and that #527/#562's triage note is corrected by ADR-0069 rather than
broken by it.

- [ ] **Step 4: Update HANDOVER and ROADMAP, and PRUNE both**

HANDOVER is **851 lines** and ROADMAP **575**; the guideline is 500. Condense, do not merely
append. **Never drop an open issue number while condensing** — the PR #271 review finding. The
2d and #512 ⇒ NEXT blocks are the ones to compress: their slices are behind us and their *why*
lives in ADR-0067 and ADR-0068.

Serving the reader beats hitting the number: condense redundancy, verify no issue number is
lost, then stop grinding at the count.

- [ ] **Step 5: The full gate**

Run: `CAIRN_TEST_PG="$(scripts/pg-target.sh)" cargo test --workspace 2>&1 | tail -40`

> Never pipe through `tail` alone in a way that discards the exit code — capture it. A healthy
> full local gate is roughly **two hours** (132 binaries, ~1449 tests); start it in the
> background and do the docs pass while it runs. A killed binary exits 101 with **zero**
> `test result: FAILED` lines, which is not the same as a failure.

Also run: `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --check`.
And `cargo doc --workspace --no-deps` with `RUSTDOCFLAGS=-D warnings`, because
`run-db-gated-tests.sh` does **not** run it and CI fails two jobs on a broken intra-doc link.

- [ ] **Step 6: Commit and open the PR**

```bash
git add docs/
git commit -m "docs(#572,#570): ADR-0069, the spec bump, and the tracking documents

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
git push -u origin feat/572-non-interactive-recovery-code
```

Open the PR linking #572 and #570. **Every session ends in at least a draft PR** — if any task
above is unfinished, open it as a **draft** whose body says what is done, what is not, and what
blocks it.

---

## Self-review notes

**Spec coverage.** Design §3 → Tasks 1–2. §3.1 (why a file) → Task 1's module doc and Task 8's
ADR. §3.2 (why "old") → Task 2's flag doc and Task 8. §4 (the warning and what it does not fix)
→ Tasks 1, 3, 5, and Task 8's steps 1 and 3. §5.1 (a new module) → Task 1. §5.2 (the three pure
functions) → Task 1. §5.3 (wiring) → Tasks 2 and 3. §5.4 (what does not change) → Task 3's step 3
and Task 8's step 5. §6 tests 1–6 → Task 1. Test 7 → Task 2. Tests 8–11 → Tasks 4 and 5. Test 12
→ Task 6. Test 13 → Task 7. §7 (what is still broken) → Task 8's steps 1 and 3.

**Type consistency.** `read_recovery_code_file(&Path) -> Result<Zeroizing<String>, RecoveryCodeError>`,
`recovery_code_attempts(bool) -> usize` and `minted_code_exposure_warning() -> String` are spelled
identically in Task 1's interface block, its test, its implementation, and Tasks 2 and 3's call
sites. `apply_local_state_export`'s new parameter is `supplied_code: Option<&Zeroizing<String>>`,
inserted **before** `new_secrets`, in both the signature and the sole call site.

**Known risk, flagged rather than assumed away.** Task 4 is the largest single step and its
fixture is the whole cost: provisioning a node with a local-state escrow, authoring a born-sealed
clinical event, running a real `backup`, and wiping to a fresh machine. If it proves too slow to
build inside one session, Tasks 5 and 6 are the ones to defer — they close #570, which is
worthwhile but is not what makes a clinic able to rehearse. **Task 4 itself is not deferrable:
without it, nothing proves #572 is actually closed.**

---

## Paper-parity benchmark (§1.2)

**Inherited unchanged from [#512](https://github.com/cairn-ehr/cairn-ehr/issues/512), and not
re-derived here.** House rule 7 permits filing an `M > N` defect, never arguing one away, and
redefining the baseline inside the slice that touches the ceremony is how a falsifiable benchmark
stops being falsifiable.

**Paper counterpart:** the off-site duplicate chart — the practice that copies its records, keeps
the copy in another building, and carries the box back after a fire. This slice also serves its
neglected twin: the practice that periodically *checks the box is still readable* without waiting
for a fire.

**Steps:** paper *N* = **2** (fetch the box; shelve it) → architecture-forced *M* = **3** (attach
the medium; run `cairn-node restore`; supply the old node's recovery code, which is a **second,
separately-prompted secret** asked for after the node plane is already applied) → UI bundling
target *K* = **2**. `M > N` still stands, **#512 stays open, and this slice does not close it.**

**This slice adds no human act.** In the attended ceremony the flag is absent and every act is
what it was. In a drill it *replaces* one human act with a file read, so the count moves down
rather than up. The excess act is unchanged in identity from what the #512 session established:
ADR-0068 deleted the provenance confirmation that DR slice 1's plan blamed, and the recovery-code
prompt is the real third act.

**Time + cognitive load:** **not re-measured, deliberately.** The measured figure is 116.7 s
against a 600 s budget, linear at 1.17 ms/event with no bend
(`crates/cairn-node/results/2026-09-10-macos-m3max.md`). Reading a secret from a file instead of
from a terminal does not touch the per-event cost that figure is about, and restating a number
this slice did not change is how a results file drifts from what it measured. Task 7 runs the rig
at one small size to prove the pty is genuinely gone; it does **not** rewrite the dated results
file.

**What this slice does move, and it is the point.** #512's budget says a restore completes "within
10 minutes **unattended** after the operator's last keystroke". Before this slice that word could
not be taken at face value: there was no way to reach the last keystroke without a human at a
terminal. After it, a drill genuinely runs unattended, which is the first time the budget's own
wording is operationally true. **If a future measurement falls outside the budget, that is the
finding — file an issue, never adjust the budget.**
