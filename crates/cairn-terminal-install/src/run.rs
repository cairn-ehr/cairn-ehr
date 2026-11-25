//! One installer run, start to finish: what systemd's path unit starts when the window marks
//! a staged release `complete`.
//!
//! Flow ([`install_staged`]):
//! 1. read this terminal's own state: its pin (`files::load_pin`, re-verified), its last
//!    installed release, and whether a fork incident froze it;
//! 2. no `complete` in the staging folder: the window may still be writing, so touch nothing
//!    (only write an idle status if there is none yet: the window fetches only once it exists);
//! 3. otherwise [`attempt`] the staged release (read hostile files, `decide`, check the bytes,
//!    `swap::install`), giving an [`Outcome`];
//! 4. [`record`] it: on success the new pin and release first, then ALWAYS `status.json`, then
//!    clear the staging folder (rulings R17/R19: status first, so a polling window never sees
//!    an empty folder beside a stale status).

use crate::decide::{decide, parse_complete, refused_for_good};
use crate::files::{self, MAX_COMPLETE, MAX_META};
use crate::status::{self, Held};
use crate::swap;
use cairn_keystore::fsio::atomic_write;
use cairn_release::staging::{Target, ARTIFACT, COMPLETE, MAX_ARTIFACT, META};
use cairn_release::{check_artifact, decode, encode, Manifest, Refusal, ReleaseMeta};
use cairn_release::{SignedRoot, VerifiedRoot};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Everything a run needs, resolved by `main.rs` from the command line and the machine.
pub struct Config {
    /// The staging folder the window writes (`incoming/`).
    pub staging: PathBuf,
    /// This terminal's own state folder (root-owned).
    pub state: PathBuf,
    /// The installed window program.
    pub program: PathBuf,
    /// The (uid, gid) the new program is test-run as; `None` keeps this user (tests only).
    pub run_as: Option<(u32, u32)>,
    /// The channel's first trust file, used only while `state/roots.cbor` does not exist.
    pub genesis_root: PathBuf,
    /// This machine: what the program must be built for.
    pub this: Target,
    /// How long each print flag may take.
    pub check_timeout: Duration,
}

/// What became of the staged release.
enum Outcome {
    Installed {
        release: u64,
        history: Vec<VerifiedRoot>,
    },
    /// Refused; `release` is the staged release's number when it could be read.
    Refused {
        refusal: Refusal,
        release: Option<u64>,
    },
    /// The installer could not do its own work (its state is damaged, a write failed): not
    /// the release's fault, so it is never refused for good.
    Failed(String),
}

/// Install whatever is staged. `true`: installed, or nothing was staged.
pub fn install_staged(cfg: &Config) -> bool {
    let pin = files::load_pin(&cfg.state.join("roots.cbor"), &cfg.genesis_root);
    let installed = files::read_number(&cfg.state.join("installed_release"));
    // Fail closed: an incident file that cannot be read still freezes (as version 0).
    let incident_file = cfg.state.join("incident");
    let incident = std::fs::symlink_metadata(&incident_file).is_ok().then(|| {
        files::read_number(&incident_file)
            .ok()
            .flatten()
            .unwrap_or(0)
    });
    let held = Held {
        installed: *installed
            .as_ref()
            .ok()
            .and_then(Option::as_ref)
            .unwrap_or(&0),
        pinned: pin.as_ref().ok().map_or(0, |h| newest(h)),
    };

    if !is_staged(&cfg.staging.join(COMPLETE)) {
        if !cfg.state.join(status::STATUS).exists() {
            return report(
                status::write(&cfg.state, &status::idle(held)),
                "write the status",
            );
        }
        return true;
    }
    let outcome = match (pin, installed) {
        (Err(e), _) => Outcome::Failed(format!("this terminal's trust state: {}", e.0)),
        (_, Err(e)) => Outcome::Failed(format!("this terminal's installed_release: {e}")),
        (Ok(history), Ok(installed)) => attempt(cfg, &history, installed.unwrap_or(0), incident),
    };
    record(cfg, held, incident, outcome)
}

/// Is anything at `complete`? A link or a FIFO there counts (and is refused as garbage).
fn is_staged(complete: &Path) -> bool {
    !matches!(std::fs::symlink_metadata(complete), Err(e) if e.kind() == std::io::ErrorKind::NotFound)
}

fn newest(history: &[VerifiedRoot]) -> u64 {
    history.last().map_or(0, |v| v.root().version)
}

/// Try the staged release. Every staged file is read hostile (`files::open_staged`).
fn attempt(
    cfg: &Config,
    history: &[VerifiedRoot],
    installed: u64,
    incident: Option<u64>,
) -> Outcome {
    let staged = |name: &str| cfg.staging.join(name);
    let damaged = |why: String| Outcome::Refused {
        refusal: Refusal::Malformed(why),
        release: None,
    };
    let complete = files::read_staged(&staged(COMPLETE), MAX_COMPLETE).ok();
    let Some(number) = complete.as_deref().and_then(parse_complete) else {
        return damaged("`complete` is not a release number".into());
    };
    let meta = files::read_staged(&staged(META), MAX_META)
        .map_err(|e| Refusal::Malformed(e.to_string()))
        .and_then(|bytes| decode::<ReleaseMeta>(&bytes));
    let meta = match meta {
        Ok(meta) => meta,
        Err(refusal) => return damaged(refusal.to_string()),
    };
    // The number the window staged must be the manifest's own; which number to refuse for
    // good is read only from the manifest (it is verified below).
    if decode::<Manifest>(&meta.manifest).map(|m| m.release_version) != Ok(number) {
        return damaged("`complete` does not name the staged release".into());
    }
    let refuse = |refusal| Outcome::Refused {
        refusal,
        release: Some(number),
    };
    let (file, len) = match files::open_staged(&staged(ARTIFACT)) {
        Ok(opened) => opened,
        Err(e) => return refuse(Refusal::Malformed(format!("the staged program: {e}"))),
    };
    let (history, manifest, index) =
        match decide(history, &meta, len, installed, &cfg.this, incident) {
            Ok(verdict) => verdict,
            Err(refusal) => return refuse(refusal),
        };
    let bytes = match files::read_capped(file, MAX_ARTIFACT) {
        Ok(bytes) => bytes,
        Err(e) => return refuse(Refusal::Malformed(format!("the staged program: {e}"))),
    };
    if let Err(refusal) = check_artifact(&manifest, index, &bytes) {
        return refuse(refusal);
    }
    let artifact = &manifest.artifacts[index];
    match swap::install(
        &cfg.program,
        &bytes,
        artifact,
        number,
        cfg.run_as,
        cfg.check_timeout,
    ) {
        Ok(true) => Outcome::Installed {
            release: number,
            history,
        },
        Ok(false) => refuse(Refusal::WontStart),
        Err(e) => Outcome::Failed(format!("putting the program in place: {e}")),
    }
}

/// Record `outcome`: the new pin and release (installed), the incident (a fresh fork), then
/// `status.json`, then clear staging. Each write is attempted whatever the others did, and a
/// failure is said on stderr. Returns `true` only for an install fully recorded.
fn record(cfg: &Config, held: Held, incident: Option<u64>, outcome: Outcome) -> bool {
    let state = &cfg.state;
    let mut ok = report(std::fs::create_dir_all(state), "create the state folder");
    let status = match outcome {
        Outcome::Installed { release, history } => {
            let roots: Vec<SignedRoot> = history.iter().map(|v| v.signed().clone()).collect();
            let wrote = atomic_write(&state.join("roots.cbor"), &encode(&roots), Some(0o644));
            ok &= report(wrote, "record the trust roots");
            let number = release.to_string();
            let wrote = atomic_write(
                &state.join("installed_release"),
                number.as_bytes(),
                Some(0o644),
            );
            ok &= report(wrote, "record the installed release");
            eprintln!("installed release {release}");
            status::installed(release, newest(&history))
        }
        Outcome::Refused { refusal, release } => {
            if let (None, Refusal::Fork { version }) = (incident, &refusal) {
                let wrote = atomic_write(
                    &state.join("incident"),
                    version.to_string().as_bytes(),
                    Some(0o644),
                );
                report(wrote, "record the fork incident");
            }
            eprintln!("refused: {refusal}");
            ok = false;
            let for_good = release.filter(|_| refused_for_good(&refusal));
            status::refused(held, &refusal, for_good)
        }
        Outcome::Failed(why) => {
            eprintln!("the installer could not finish: {why}");
            ok = false;
            // No sentence names the installer's own failure; this one is true (the program in
            // place keeps running) and says whom to tell. Never refused for good.
            status::refused(held, &Refusal::WontStart, None)
        }
    };
    ok &= report(status::write(state, &status), "write the status");
    ok & report(files::clear(&cfg.staging), "clear the staging folder")
}

/// Say a failed step on stderr (the journal); `true` if it succeeded.
fn report(result: std::io::Result<()>, what: &str) -> bool {
    if let Err(e) = &result {
        eprintln!("could not {what}: {e}");
    }
    result.is_ok()
}

#[cfg(test)]
#[path = "run_tests.rs"]
mod tests;
