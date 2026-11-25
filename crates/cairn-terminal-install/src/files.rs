//! The installer's file I/O, apart from the swap: reading the STAGED files (written by the
//! unprivileged window, so every byte and every file type is hostile), clearing the staging
//! folder, and reading this terminal's own state (root-owned, trusted only after its roots are
//! re-verified).
//!
//! Hostile reads (`open_staged`): `O_NOFOLLOW` refuses a symlink (a link to a root-only file
//! would otherwise be read as root); `O_NONBLOCK` keeps a FIFO from hanging the open; only a
//! regular file is read; and a read never takes more than its cap, even if the file grows
//! while we read it. The installer verifies the bytes it read into memory and installs those
//! same bytes, so a file changed after the check changes nothing.

use cairn_release::staging::COMPLETE;
use cairn_release::{advance, decode, genesis, official_root, Refusal, SignedRoot, VerifiedRoot};
use std::fs::File;
use std::io::{self, Read};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

/// The largest staged meta (root chain + manifest): far above any real one (a few KB).
pub const MAX_META: u64 = 1024 * 1024;
/// The largest `complete` file: a u64 has at most 20 digits.
pub const MAX_COMPLETE: u64 = 20;

/// Open a staged file for reading, refusing symlinks and anything but a regular file.
/// Returns the file and its length at open time.
pub fn open_staged(path: &Path) -> io::Result<(File, u64)> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let meta = file.metadata()?; // fstat of what we opened, not of the path
    if !meta.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not a regular file",
        ));
    }
    Ok((file, meta.len()))
}

/// Read at most `cap` bytes of an opened staged file; more than `cap` is an error, never a
/// silent truncation.
pub fn read_capped(file: File, cap: u64) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    file.take(cap + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > cap {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "file too large"));
    }
    Ok(bytes)
}

/// [`open_staged`] then [`read_capped`].
pub fn read_staged(path: &Path, cap: u64) -> io::Result<Vec<u8>> {
    read_capped(open_staged(path)?.0, cap)
}

/// Remove everything inside the staging folder, keeping the folder (it is set up with the
/// window's group by packaging; the window cannot recreate it). `complete` goes FIRST, so a
/// folder the installer cannot fully remove (an attacker's deep tree) never leaves a release
/// marked staged; every other entry is then tried, and the first error is returned. A symlink
/// is removed as a link (its target is untouched), and `remove_dir_all` does not follow links
/// inside a directory, so nothing outside the folder is ever deleted.
pub fn clear(staging: &Path) -> io::Result<()> {
    let mut first = match std::fs::remove_file(staging.join(COMPLETE)) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    };
    for entry in std::fs::read_dir(staging)? {
        let removed = entry.and_then(|entry| {
            if entry.file_type()?.is_dir() {
                std::fs::remove_dir_all(entry.path())
            } else {
                std::fs::remove_file(entry.path())
            }
        });
        first = first.and(removed);
    }
    first
}

/// A small decimal state file (`installed_release`, `incident`): absent is `None`.
pub fn read_number(path: &Path) -> io::Result<Option<u64>> {
    match std::fs::read_to_string(path) {
        Ok(text) => text
            .trim()
            .parse()
            .map(Some)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, format!("{path:?}"))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Why this terminal's own pin could not be read: its state is damaged or unreadable. Not
/// the update's fault, so it is reported apart from a refusal.
#[derive(Debug)]
pub struct PinError(pub String);

/// This terminal's pin, re-verified from its own state: `roots.cbor` (genesis first, then
/// every accepted root) if present; else the genesis trust file the person who set the
/// terminal up installed; else the one compiled in; else no pin at all (an empty history,
/// which every release refuses as `NoChannel`). A damaged `roots.cbor` is an error, never a
/// fall-back to genesis: that would silently undo a rotation the terminal had accepted.
pub fn load_pin(roots_file: &Path, genesis_file: &Path) -> Result<Vec<VerifiedRoot>, PinError> {
    let damaged = |what: &Path, why: String| PinError(format!("{what:?} is damaged: {why}"));
    match std::fs::read(roots_file) {
        Ok(bytes) => {
            let roots: Vec<SignedRoot> =
                decode(&bytes).map_err(|e| damaged(roots_file, e.to_string()))?;
            let (first, rest) = roots
                .split_first()
                .ok_or_else(|| damaged(roots_file, "no roots".into()))?;
            let g = genesis(first).map_err(|e| damaged(roots_file, e.to_string()))?;
            return advance(&[g], rest).map_err(|e| damaged(roots_file, e.to_string()));
        }
        Err(e) if e.kind() != io::ErrorKind::NotFound => {
            return Err(PinError(format!("{roots_file:?}: {e}")))
        }
        Err(_) => {}
    }
    let first = match std::fs::read(genesis_file) {
        Ok(bytes) => {
            Some(decode::<SignedRoot>(&bytes).map_err(|e| damaged(genesis_file, e.to_string()))?)
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => official_root(),
        Err(e) => return Err(PinError(format!("{genesis_file:?}: {e}"))),
    };
    match first {
        Some(first) => {
            Ok(vec![genesis(&first).map_err(|e: Refusal| {
                damaged(genesis_file, e.to_string())
            })?])
        }
        None => Ok(Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// A directory the installer cannot fully remove (here: a read-only one deep inside, as
    /// an unprivileged user can build) must not leave `complete` behind, nor stop the rest.
    #[test]
    fn clear_removes_complete_first_and_tries_everything() {
        let dir = tempfile::tempdir().unwrap();
        let staging = dir.path();
        let deep = staging.join("a/b/c");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("stuck"), b"x").unwrap();
        std::fs::write(staging.join(COMPLETE), b"7").unwrap();
        std::fs::write(staging.join("z-other"), b"x").unwrap();
        let ro = std::fs::Permissions::from_mode(0o555);
        std::fs::set_permissions(&deep, ro).unwrap();
        let cleared = clear(staging);
        std::fs::set_permissions(&deep, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(cleared.is_err(), "the stuck file is reported");
        assert!(!staging.join(COMPLETE).exists());
        assert!(!staging.join("z-other").exists());
    }
}
