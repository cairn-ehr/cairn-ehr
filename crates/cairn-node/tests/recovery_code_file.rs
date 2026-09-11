//! #572 — the OLD node's recovery code can reach `restore` from a file.
//!
//! These are the pure halves of the decision: what a supplied code file may contain, how many
//! times the unseal loop may ask given where the code came from, and the wording of the warning
//! that reports a freshly-minted code landing somewhere no human is reading.
//!
//! **Why they are in the LIBRARY rather than in `main.rs`.** `Cmd` is defined in a binary crate,
//! so an integration test cannot import it — which is why the `--help` surface has to be driven
//! as a spawned subprocess in `restore_needs_nothing_about_the_dead_node.rs`. Pure functions have
//! no such excuse, and validation whose whole value is being provable without a database, a tty
//! or a spawned binary belongs where it can actually be tested.
//!
//! See [`cairn_node::restore::recovery_code`] for the decision itself and
//! `docs/spec/decisions/0069-the-restore-takes-its-recovery-code-from-a-file.md` for its ADR.

use cairn_node::restore::recovery_code::{
    minted_code_exposure_warning, read_recovery_code_file, recovery_code_attempts,
    RecoveryCodeError, RECOVERY_CODE_ATTEMPTS,
};

/// Write `contents` to a scratch file and hand back the directory guard alongside the path.
///
/// **The guard is returned deliberately.** `TempDir` deletes its whole tree on drop, so a helper
/// that returned only the path would hand back a path into a directory that had already been
/// removed — a fixture that fails for a reason having nothing to do with the code under test.
fn code_file(contents: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old-recovery-code");
    std::fs::write(&path, contents).unwrap();
    (dir, path)
}

/// A recovery code, **derived at runtime rather than written as a literal** (house rule 6a).
///
/// A byte-array or string literal in a crypto context trips CodeQL's
/// `rust/hard-coded-cryptographic-value` as a recurring critical false positive that blocks the
/// scan until a human dismisses it (#146). Deriving keeps the fixture deterministic while
/// presenting no hard-coded value to the scanner.
///
/// The shape mirrors `generate_recovery_code`'s alphabet closely enough for the tests that
/// matter, without importing the generator: these tests are about the FILE, not about the code's
/// alphabet, and coupling them to the generator would make an unrelated alphabet change red.
fn a_recovery_code() -> String {
    let alphabet: Vec<char> = ('A'..='Z').chain('2'..='7').collect();
    (0..32usize)
        .map(|i| alphabet[(i * 7 + 3) % alphabet.len()])
        .collect()
}

#[test]
fn a_file_holding_a_code_yields_that_code() {
    let expected = a_recovery_code();
    let (_dir, path) = code_file(&expected);
    let got = read_recovery_code_file(&path).unwrap();
    assert_eq!(got.as_str(), expected.as_str());
}

/// `printf '%s\n' "$CODE" > file` is how anyone would write one of these, and
/// `normalize_recovery_code` strips the newline before the unwrap regardless. Refusing it would
/// be a trap with no upside, on the one ceremony that has no second attempt.
#[test]
fn a_trailing_newline_is_tolerated() {
    let expected = a_recovery_code();
    let (_dir, path) = code_file(&format!("{expected}\n"));
    let got = read_recovery_code_file(&path).unwrap();
    assert_eq!(got.as_str(), expected.as_str());
}

/// **Not cosmetic.** `normalize_recovery_code` strips all spacing and case before the unwrap, so
/// a file holding only whitespace normalizes to the empty string and would sail into an unseal
/// under an effectively EMPTY secret. That comes back as `None` — bit-for-bit the answer a WRONG
/// code gives — so the operator would be told their code was wrong, or their export possibly
/// damaged, and would go hunting for a code they had in fact saved correctly.
/// `establish-local-state-key` already guards this exact input for this exact reason.
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

/// A missing path is the drill author's most likely mistake, and it must be DISTINGUISHABLE from
/// a blank file: one means "fix your script", the other means "the file you saved your only
/// off-node secret into has nothing in it", which is a far worse morning.
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
/// "2 attempt(s) left" about a file — telling an operator nothing and inviting them to wait for
/// a prompt that will never come. The prompt keeps its retries because a human can genuinely
/// type a different thing the second time, which is the entire reason the budget exists.
#[test]
fn a_supplied_code_is_asked_once_and_a_prompt_keeps_its_retries() {
    assert_eq!(recovery_code_attempts(true), 1);
    assert_eq!(recovery_code_attempts(false), RECOVERY_CODE_ATTEMPTS);
    // A `const` block, because the value IS a constant and `clippy::assertions_on_constants`
    // is right to say so: this now fails the BUILD rather than a test run, which is the
    // stronger place for it. The guard matters because setting the budget to 1 would silently
    // collapse the prompt path into the file path — and the budget exists precisely because
    // this prompt lands after `finalize_identity` has fenced the restore door.
    const {
        assert!(
            RECOVERY_CODE_ATTEMPTS > 1,
            "the prompt must have retries at all"
        )
    };
}

/// The warning REPORTS an exposure. It must not imply it prevented one: a future reader who
/// takes it as a guarantee stops looking for the real fix, and the exposure is older and wider
/// than the flag this slice adds.
#[test]
fn the_exposure_warning_reports_rather_than_reassures() {
    let text = minted_code_exposure_warning();
    assert!(text.contains("stderr"), "it must name the stream: {text}");
    let lowered = text.to_lowercase();
    assert!(
        lowered.contains("recovery code"),
        "it must name what leaked: {text}"
    );
    for reassurance in ["prevented", "suppressed", "withheld", "is safe"] {
        assert!(
            !lowered.contains(reassurance),
            "the warning must not claim to have prevented anything ({reassurance:?}): {text}"
        );
    }
}
