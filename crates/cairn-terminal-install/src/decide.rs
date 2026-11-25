//! The installer's verdict, PURE: given what this terminal holds (its verified pin, its last
//! installed release, whether a fork froze it) and what was staged (the decoded `ReleaseMeta`
//! and the artifact's length), may the staged program be installed, and which artifact is it?
//!
//! No I/O happens here, so every rule is a plain unit test. The caller (`run.rs`) reads the
//! staged files, calls [`decide`], then checks the artifact's bytes (`check_artifact`), proves
//! the program starts ([`answers_match`]) and swaps it in (`swap.rs`).
//!
//! Gate order (the plan's, binding): a fork incident refuses everything; a file over
//! `MAX_ARTIFACT` is too large (before a byte of it is read); the root chain and release
//! signatures verify against THIS terminal's pin (`verify_bundle`); the manifest names a
//! `window` program for this exact machine.

use cairn_release::staging::{Target, MAX_ARTIFACT};
use cairn_release::{verify_bundle, Artifact, Manifest, Refusal, ReleaseMeta, VerifiedRoot};

/// Decide on a staged release. `history` is this terminal's pin, re-verified from its own
/// state (empty when it has none: `NoChannel`); `incident` is the fork version that froze it,
/// if any. Returns the new root history (to persist only once installed), the verified
/// manifest, and the index of this terminal's program in it.
pub fn decide(
    history: &[VerifiedRoot],
    meta: &ReleaseMeta,
    artifact_len: u64,
    last_installed: u64,
    this: &Target,
    incident: Option<u64>,
) -> Result<(Vec<VerifiedRoot>, Manifest, usize), Refusal> {
    if let Some(version) = incident {
        return Err(Refusal::Fork { version });
    }
    if artifact_len > MAX_ARTIFACT {
        return Err(Refusal::TooLarge);
    }
    let (history, manifest) = verify_bundle(history, meta, last_installed)?;
    let index = manifest
        .artifacts
        .iter()
        .position(|a| is_program_for(a, this))
        .ok_or_else(|| Refusal::NoProgramForThisTerminal {
            os: this.os.clone(),
            arch: this.arch.clone(),
            platform: this.platform.clone(),
        })?;
    Ok((history, manifest, index))
}

/// Is `a` a window program built for exactly this machine? Every field must match: an
/// artifact that leaves one out is for no machine.
fn is_program_for(a: &Artifact, this: &Target) -> bool {
    a.kind == "window"
        && a.os.as_deref() == Some(this.os.as_str())
        && a.arch.as_deref() == Some(this.arch.as_str())
        && a.platform.as_deref() == Some(this.platform.as_str())
}

/// Did the new program, run unprivileged, print exactly what its signed manifest names? Its
/// `--print-schema-generation` output must be the artifact's `generation_line` and its
/// `--print-release-version` output the manifest's release number, each one line (a trailing
/// newline is allowed, nothing else). An artifact that names no generation line cannot be
/// checked, so it never matches. `None` output is a program that failed or timed out.
pub fn answers_match(
    artifact: &Artifact,
    release: u64,
    generation_out: Option<&str>,
    release_out: Option<&str>,
) -> bool {
    let one_line = |out: Option<&str>| out.map(|s| s.strip_suffix('\n').unwrap_or(s).to_string());
    let Some(want) = artifact.generation_line.as_deref() else {
        return false;
    };
    one_line(generation_out).as_deref() == Some(want)
        && one_line(release_out) == Some(release.to_string())
}

/// Ruling R18: is this refusal for good, so the window must never fetch that release again?
/// Every refusal is, except the two a corrupt transfer can cause (`Malformed`,
/// `ArtifactMismatch`): those the window fetches again.
pub fn refused_for_good(refusal: &Refusal) -> bool {
    !matches!(
        refusal,
        Refusal::Malformed(_) | Refusal::ArtifactMismatch { .. }
    )
}

/// The staged `complete` file's content: a decimal `u64` and nothing else (no sign, no
/// whitespace, no newline). The file is written by an unprivileged user, so anything else is
/// garbage. `u64::from_str` alone would accept a leading `+`.
pub fn parse_complete(bytes: &[u8]) -> Option<u64> {
    if bytes.is_empty() || !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(bytes).ok()?.parse().ok()
}

#[cfg(test)]
#[path = "decide_tests.rs"]
mod tests;
