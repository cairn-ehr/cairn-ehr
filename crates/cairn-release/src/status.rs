//! The terminal installer's `state/status.json` (#909 slice C2): what the root-owned installer
//! last did, written by it (Task 9) and read by the window (`release_fetch.rs`), which runs as
//! another user and so learns the terminal's pin and installed release only from this file.
//!
//! The window trusts it for nothing but courier work: which release to fetch, and what to say.
//! The installer still verifies every staged release against its own pin.
use serde::{Deserialize, Serialize};

/// One status file. JSON rather than CBOR so an operator can read it with `cat`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct InstallerStatus {
    /// `"idle"` (nothing tried yet), `"installed"` or `"refused"`.
    pub state: String,
    /// The release now installed (0: none yet).
    pub installed_release: u64,
    /// The trust version this terminal last accepted: the window fetches the roots after it.
    pub pinned_root_version: u64,
    /// The refusal's sentence (`Refusal::sentence`, `Who::Terminal`) when `state` is `"refused"`.
    pub sentence: Option<String>,
    /// The release refused for good: set ONLY for a refusal that fetching it again cannot fix
    /// (every refusal but `ArtifactMismatch` and `Malformed`), so the window never fetches that
    /// release again; `None` for one damaged in transit, which the window does fetch again. Absent
    /// in an older file, read as `None`.
    #[serde(default)]
    pub refused_release: Option<u64>,
}

impl InstallerStatus {
    /// Read a status file. Damaged, half-written or of another shape is `None`: no status.
    pub fn from_json(bytes: &[u8]) -> Option<Self> {
        serde_json::from_slice(bytes).ok()
    }

    /// The file's bytes.
    pub fn to_json(&self) -> Vec<u8> {
        // Serialising a struct of strings and numbers to a Vec cannot fail.
        serde_json::to_vec_pretty(self).expect("status JSON")
    }
}
