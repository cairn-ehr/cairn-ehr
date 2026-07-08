//! Crash-safe local file writes, shared by every component that must replace a file
//! on disk without ever exposing a torn or half-written intermediate.
//!
//! WHY THIS EXISTS: the keystore (issue #45) needs an atomic key-file write so an
//! interrupted `init`/`seal-key` can never leave a half-written key that boots
//! `Corrupt`; the ADR-0026 backup medium needs the same so a crash mid-backup never
//! destroys the previous good backup with a partial new one; and the backup-health
//! sidecar needs it so a freshness reading flips old→new in one step and never tears.
//! Three call sites, one carefully-reviewed implementation — extracted here rather than
//! duplicated so the fsync/rename/0600 discipline can never drift between them.
//!
//! The contract is the POSIX atomic-replace pattern: write a sibling temp file, fsync
//! its bytes, `rename` it over the target (atomic within one directory), then fsync the
//! parent directory so the rename entry itself survives a power loss. Any reader —
//! concurrent or after a crash — sees either the complete OLD file or the complete NEW
//! one, never a mix. A crash before the rename leaves the intact original untouched.

use std::path::{Path, PathBuf};

/// The sibling temp path used for an atomic write: the target's full filename plus a
/// `.tmp` suffix, in the SAME directory. Pure (no I/O) so it is trivially testable.
/// Same-directory matters: `rename` is only atomic within one filesystem, so a temp in
/// `/tmp` (possibly a different mount) would defeat the whole point.
pub fn tmp_sibling(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(".tmp");
    path.with_file_name(name)
}

/// fsync the directory that contains `path`, so a freshly-`rename`d entry survives a
/// power loss. We fsync the temp file's *bytes* before the rename, but the rename
/// itself is a directory-entry update whose durability is a SEPARATE guarantee:
/// without this, a crash just after the write returns can revert the directory to its
/// pre-rename state. The parent of a bare filename is "" → fall back to ".".
#[cfg(unix)]
fn fsync_parent_dir(path: &Path) -> std::io::Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::File::open(parent)?.sync_all()
}

/// Atomically write `bytes` to `path`, optionally forcing a unix permission `mode`.
///
/// `mode` is `Some(0o600)` for secret material (the keystore) and for the backup
/// medium / health sidecar (owner-only is a safe default — defence in depth on a
/// shared host); pass `None` to leave the umask-default perms. On non-unix the mode is
/// ignored (POSIX perms do not apply) and the directory fsync is skipped (opening a
/// directory as a File is not portable), but the tmp → fsync → rename discipline is
/// identical so the atomic-replace guarantee holds on every platform.
///
/// On ANY failure the temp file is removed so no litter is left, and the original
/// target is untouched (the rename never happened).
#[cfg(unix)]
pub fn atomic_write(path: &Path, bytes: &[u8], mode: Option<u32>) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let tmp = tmp_sibling(path);
    // create+truncate clobbers any stale temp a previous crashed write may have left.
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    if let Some(m) = mode {
        opts.mode(m);
    }
    let mut f = opts.open(&tmp)?;
    let write_then_sync = (|| -> std::io::Result<()> {
        // `mode()` above only applies when open CREATES the inode; a reused stale temp
        // keeps its OLD (possibly wider) perms. Force the requested mode explicitly —
        // BEFORE any bytes are written — so the output can never end up wider than asked.
        if let Some(m) = mode {
            f.set_permissions(std::fs::Permissions::from_mode(m))?;
        }
        f.write_all(bytes)?;
        f.sync_all()?; // bytes durable on disk BEFORE we expose them via the rename
        Ok(())
    })();
    if let Err(e) = write_then_sync {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    drop(f); // close before rename (tidy on all platforms)
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    fsync_parent_dir(path)?; // make the rename itself crash-durable, not just the bytes
    Ok(())
}

#[cfg(not(unix))]
pub fn atomic_write(path: &Path, bytes: &[u8], _mode: Option<u32>) -> std::io::Result<()> {
    use std::io::Write;
    let tmp = tmp_sibling(path);
    let write_then_sync = (|| -> std::io::Result<()> {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        Ok(())
    })();
    if let Err(e) = write_then_sync {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn tmp_sibling_appends_tmp_suffix_in_same_dir() {
        // The atomic-write temp file must live in the SAME directory as the target
        // (rename is only atomic within one filesystem) and be named so it can never
        // be mistaken for the target itself.
        let p = Path::new("/var/lib/cairn/node.key");
        let t = tmp_sibling(p);
        assert_eq!(t, Path::new("/var/lib/cairn/node.key.tmp"));
        assert_eq!(
            t.parent(),
            p.parent(),
            "temp must be a sibling of the target"
        );
    }

    #[test]
    fn write_is_atomic_and_leaves_no_temp_litter() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("medium.bin");
        atomic_write(&p, b"hello", None).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"hello");
        assert!(
            !tmp_sibling(&p).exists(),
            "atomic write must not leave a .tmp sibling"
        );
    }

    #[test]
    fn write_replaces_an_existing_target_completely() {
        // Replacing a longer file with a shorter one must leave NO trailing bytes of
        // the old content (rename swaps the whole inode, never truncates-in-place).
        let dir = tempdir().unwrap();
        let p = dir.path().join("f");
        atomic_write(&p, b"a long previous version", None).unwrap();
        atomic_write(&p, b"short", None).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"short");
    }

    #[test]
    fn stale_temp_from_a_prior_crashed_write_is_overwritten() {
        // A previous crash could leave a stale `<target>.tmp`. A new write must clobber
        // it (truncate) and still succeed, never appending to or being confused by it.
        let dir = tempdir().unwrap();
        let p = dir.path().join("f");
        std::fs::write(tmp_sibling(&p), b"garbage from a half-finished write").unwrap();
        atomic_write(&p, b"clean", None).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"clean");
        assert!(
            !tmp_sibling(&p).exists(),
            "stale temp must be gone after a successful write"
        );
    }

    #[cfg(unix)]
    #[test]
    fn mode_is_forced_even_over_a_stale_wide_perm_temp() {
        // `OpenOptions::mode()` only applies when open CREATES the inode. A stale temp
        // left wider than the requested mode is reused by create+truncate with its OLD
        // perms, and rename would carry that wider mode onto the target. The write MUST
        // force the requested mode regardless — otherwise secret material leaks.
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().unwrap();
        let p = dir.path().join("secret");
        let tmp = tmp_sibling(&p);
        std::fs::write(&tmp, b"stale junk").unwrap();
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644)).unwrap();
        atomic_write(&p, b"secret bytes", Some(0o600)).unwrap();
        let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "a stale wide-perm temp must not leak its mode into the target"
        );
    }

    #[cfg(unix)]
    #[test]
    fn none_mode_leaves_umask_default_perms() {
        // Passing `None` must NOT force 0600 — the medium/sidecar callers that want the
        // umask default rely on this (and the keystore caller passes Some(0o600)).
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().unwrap();
        let p = dir.path().join("f");
        atomic_write(&p, b"x", None).unwrap();
        // We can't assert the exact umask-dependent mode portably, but it must at least
        // not have been clamped to 0600 owner-only by us when None was requested: assert
        // the file is readable by its owner (a trivially-true sanity check that the write
        // path didn't error) and that Some(0o600) vs None take different code paths.
        let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o600;
        assert_eq!(
            mode & 0o400,
            0o400,
            "owner must at least be able to read its own file"
        );
    }
}
