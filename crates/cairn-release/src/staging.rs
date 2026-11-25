//! The staging folder: the one contract between the window, which stages a release as an
//! unprivileged courier (`cairn-gui-tauri`'s `release_fetch.rs`), and the root-owned terminal
//! installer (`cairn-terminal-install`), which verifies and installs it. One definition here so
//! the two programs (often of different releases) can never disagree on a name or a limit.
//!
//! The window writes [`META`], then [`ARTIFACT`], then [`COMPLETE`] LAST (by rename); the
//! installer acts on [`COMPLETE`] alone and re-verifies every byte against its own pin.

/// The staged signed part of the release (a `ReleaseMeta`, canonical CBOR).
pub const META: &str = "meta.cbor";
/// The staged program's bytes.
pub const ARTIFACT: &str = "artifact.bin";
/// Written last, holding the release number (decimal): the signal that the rest is whole.
pub const COMPLETE: &str = "complete";

/// The largest program either side handles: the window never fetches a larger one, and the
/// installer refuses one (`Refusal::TooLarge`) before reading it into memory.
pub const MAX_ARTIFACT: u64 = 64 * 1024 * 1024;

/// The machine a program must be built for: what the node's offer door filters on, and what
/// the installer matches a manifest's `window` artifact against.
#[derive(Debug, Clone, PartialEq)]
pub struct Target {
    pub os: String,
    pub arch: String,
    pub platform: String,
}

/// `<ID>-<VERSION_ID>` from `/etc/os-release`'s text (values unquoted), e.g. `debian-12`.
/// Either key missing or empty is `None`. **Pure.**
pub fn platform_of(os_release: &str) -> Option<String> {
    let value = |key: &str| {
        os_release.lines().find_map(|line| {
            let v = line.trim().strip_prefix(key)?.strip_prefix('=')?;
            let v = v.trim().trim_matches(|c| c == '"' || c == '\'');
            (!v.is_empty()).then(|| v.to_string())
        })
    };
    Some(format!("{}-{}", value("ID")?, value("VERSION_ID")?))
}

/// This machine's target: the build's own os and arch, and `os_release`'s platform. No
/// os-release (not Linux, or unreadable) is no target. **Pure.**
pub fn this_target(os_release: Option<&str>) -> Option<Target> {
    Some(Target {
        os: std::env::consts::OS.into(),
        arch: std::env::consts::ARCH.into(),
        platform: platform_of(os_release?)?,
    })
}
