//! Writing `state/status.json`, the installer's only word to the window (which reads it with
//! `InstallerStatus::from_json`, cairn-release `status.rs`). Mode 0644: the window runs as
//! another user and must be able to read it; only root writes it.

use cairn_release::{InstallerState, InstallerStatus, Refusal, Who};
use std::io;
use std::path::Path;

/// The status file's name inside the state folder.
pub const STATUS: &str = "status.json";

/// What the installer held when it wrote the status: its last installed release and the
/// newest trust version it has accepted (0: none).
#[derive(Clone, Copy, Debug)]
pub struct Held {
    pub installed: u64,
    pub pinned: u64,
}

/// Nothing tried yet.
pub fn idle(held: Held) -> InstallerStatus {
    InstallerStatus {
        state: InstallerState::Idle,
        installed_release: held.installed,
        pinned_root_version: held.pinned,
        sentence: None,
        refused_release: None,
    }
}

/// `release` is now installed, under trust version `pinned`.
pub fn installed(release: u64, pinned: u64) -> InstallerStatus {
    InstallerStatus {
        state: InstallerState::Installed,
        installed_release: release,
        pinned_root_version: pinned,
        sentence: None,
        refused_release: None,
    }
}

/// A refusal, said as the terminal. `refused_release` is the release refused for good (the
/// window then never fetches it again), or `None` for every refusal but the two reached after
/// the release verified (`decide::refused_for_good`, ruling R23).
pub fn refused(held: Held, refusal: &Refusal, refused_release: Option<u64>) -> InstallerStatus {
    InstallerStatus {
        state: InstallerState::Refused,
        installed_release: held.installed,
        pinned_root_version: held.pinned,
        sentence: Some(refusal.sentence(Who::Terminal)),
        refused_release,
    }
}

/// Write the status file atomically (a window polling it never reads half a file).
pub fn write(state_dir: &Path, status: &InstallerStatus) -> io::Result<()> {
    cairn_keystore::fsio::atomic_write(&state_dir.join(STATUS), &status.to_json(), Some(0o644))
}
