//! #572 — how the OLD node's recovery code reaches a restore when no human is at the terminal.
//!
//! # What problem this solves
//!
//! `restore` needs two secrets and used to treat them unalike. The passphrase for the NEW sealed
//! key has `--passphrase` and `CAIRN_KEY_PASSPHRASE`. The OLD node's recovery code — which
//! unseals the local-state export, and is therefore the thing that returns the dead node's
//! custody — had no flag and no environment variable at all. It was read through
//! `rpassword::prompt_password`, which opens `/dev/tty` and fails on any non-tty.
//!
//! A piped code did not merely get ignored: the read errored, the export never opened, and the
//! restore finished having recovered **zero patients** while exiting non-zero — #500's own
//! signature, arriving inside the mechanism built to prevent it. So a clinic could not rehearse
//! its disaster recovery, and the one CLI surface whose correctness matters most could not be
//! tested at all (#570 is the same wall).
//!
//! # Why a FILE, and not a flag value or an environment variable
//!
//! The two secrets are not alike. The passphrase is invented at restore time and protects a key
//! that has not existed for ten seconds. The recovery code is the single **retained** off-node
//! artifact, and together with the medium sitting beside it, it yields the clinic's whole
//! clinical record in the clear. Consistency with `CAIRN_KEY_PASSPHRASE` is not a strong enough
//! reason to give both the same exposure.
//!
//! A path keeps the secret off the **process table** (`ps auxww` shows the path, never the code),
//! out of **shell history**, and out of the **environment** — so not in `/proc/<pid>/environ`,
//! not inherited by child processes, and not in a crash dump. It also composes for free: a tmpfs
//! path, a named pipe and `/dev/stdin` all work with no extra code here.
//!
//! # Why these functions are PURE and live in the library
//!
//! `Cmd` is defined in a binary crate, so integration tests cannot import it — which is why the
//! `--help` surface has to be driven as a spawned subprocess. Pure functions have no such
//! excuse, and the validation below is exactly the kind that must be provable without a
//! database, a tty or a spawned binary. Pinned by `tests/recovery_code_file.rs`.
//!
//! See `docs/spec/decisions/0069-the-restore-takes-its-recovery-code-from-a-file.md`.

use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

/// How many times the unseal loop may re-ask a HUMAN for the old recovery code.
///
/// Lives here rather than in `main.rs` so that [`recovery_code_attempts`] — the function that
/// chooses between this and one — can be tested without spawning a binary.
///
/// **Why a budget exists at all.** This prompt lands *after* `finalize_identity` has fenced the
/// restore door: `local_node` is written and a second restore into the same database is refused.
/// Before the retries, a single mistyped character therefore cost the node its custody key
/// outright, with the only remaining option — restore again from the same medium into a
/// different fresh database, accepting a second superseding identity — stated nowhere.
pub const RECOVERY_CODE_ATTEMPTS: usize = 3;

/// Why an operator-supplied recovery-code file could not be used.
///
/// The two variants are deliberately distinct, because they send the operator to different
/// places. [`Self::Unreadable`] means *"fix your script, your permissions or your mount"*.
/// [`Self::Blank`] means *"the file you saved your only off-node secret into has nothing in
/// it"*, which is a far worse morning and must not be reported as an I/O problem.
#[derive(thiserror::Error, Debug)]
pub enum RecoveryCodeError {
    /// The path could not be read: missing, permissions, a mount that went away.
    #[error("could not read the recovery-code file at {path} ({source})")]
    Unreadable {
        path: PathBuf,
        source: std::io::Error,
    },
    /// The file exists but holds no code. See [`read_recovery_code_file`] for why this is
    /// dangerous rather than merely useless.
    #[error(
        "the recovery-code file at {path} is blank (no recovery code in it). Refusing to \
         continue: `normalize_recovery_code` strips all spacing before the unwrap, so a file of \
         whitespace would attempt to open the export under an EMPTY secret and report the \
         resulting failure as a wrong code — sending you hunting for a code you saved correctly."
    )]
    Blank { path: PathBuf },
}

/// Read the OLD node's recovery code out of a file the operator named.
///
/// **The trailing newline is tolerated on purpose.** `printf '%s\n' "$CODE" > file` is how anyone
/// would write one of these, and [`cairn_keystore::seal::normalize_recovery_code`] strips spacing
/// and case before the unwrap regardless. Refusing it would be a trap with no upside, on the one
/// ceremony that has no second attempt.
///
/// **A whitespace-only file is refused on purpose, and this is the load-bearing half.** That same
/// normalization turns `"   "` into the empty string, so such a file would sail into an unseal
/// under an effectively empty secret and come back as `None` — which is bit-for-bit the answer a
/// WRONG code gives. The operator would then be told their code was wrong, or that their export
/// might be damaged, and would go hunting for a code they had in fact saved correctly.
/// `establish-local-state-key` already guards exactly this input for exactly this reason.
///
/// The result is [`Zeroizing`] so the secret is wiped from the heap on drop (issue #46), matching
/// every other secret this binary handles. The raw read is wrapped too: the untrimmed copy is
/// just as much key material as the trimmed one.
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
/// A SUPPLIED code is asked **once**: re-reading the same file cannot change the answer, and a
/// budget of three would print *"2 attempt(s) left"* about a file — which tells an operator
/// nothing and invites them to wait for a prompt that will never come.
///
/// A PROMPTED code keeps its full budget, because a human can genuinely type a different thing
/// the second time. That is the entire reason [`RECOVERY_CODE_ATTEMPTS`] exists.
///
/// Split out as a named function rather than inlined at the call site so the count that reaches
/// the failure message is the same one that drove the loop: a message saying *"did not open
/// after 3 attempts"* when the file was read once is a message that lies.
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
/// **The exposure is older and wider than the flag this module adds**, which is why the caller
/// keys it on whether stderr is a terminal rather than on which flags were passed. A medium with
/// no local-state export sibling never reaches the recovery-code prompt at all, so a sealed
/// restore of one has always been able to run unattended and print a fresh code into a log. The
/// honest question is *"will a human see this code?"*, and it covers both paths.
///
/// This corrects, rather than breaks, #527/#562's triage note that *"no cron-run command reaches
/// `print_recovery_code`"*: that note was already false before this slice existed.
pub fn minted_code_exposure_warning() -> String {
    "WARNING: the recovery code above was written to stderr, and stderr is not a terminal. \
     Whatever captured this stream — a log file, a cron mail, a CI artifact — now holds the only \
     off-node secret that recovers this node's signing key. Treat that capture as secret, or \
     re-run this restore attended."
        .to_string()
}
