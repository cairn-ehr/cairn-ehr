use crate::seal;
use cairn_event::{generate_key, SigningKey};
use std::path::Path;

#[derive(thiserror::Error, Debug)]
pub enum KeystoreError {
    #[error("io: {0}")] Io(#[from] std::io::Error),
    #[error("key material: {0}")] Key(String),
    /// The file is a sealed bundle but no secret was supplied. A DISTINCT variant (not
    /// folded into `Key`) so a caller can react — e.g. the CLI prompts interactively
    /// for the passphrase — by matching ONE load attempt's error, with no separate
    /// file-classification read that could race the load (a TOCTOU).
    #[error("key is sealed: provide the passphrase (set CAIRN_KEY_PASSPHRASE) or recovery code")]
    Sealed,
}

/// At-rest posture of a key file, inspectable WITHOUT the secret (for `status`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyAtRest {
    /// A valid sealed bundle. `dual_recipient` is true when it carries a recovery wrap.
    Sealed { dual_recipient: bool },
    /// A raw 32-byte Ed25519 seed (legacy/insecure).
    Plaintext,
    /// No file at the path.
    Missing,
    /// A file exists but is neither a sealed bundle nor a 32-byte seed.
    Corrupt,
}

/// Generate a keypair and write the seed UNSEALED (mode 0600). Insecure — only for
/// throwaway test nodes and the explicit `--insecure-plaintext` path. The recovery
/// escrow does NOT exist for a plaintext key (key loss = node loss).
pub fn generate_plaintext(path: &Path) -> Result<(SigningKey, String), KeystoreError> {
    let (sk, kid) = generate_key().map_err(|e| KeystoreError::Key(e.to_string()))?;
    write_key_file(path, &sk.to_bytes())?;
    Ok((sk, kid))
}

/// Generate a keypair and write it SEALED under both secrets (ADR-0026 slice A).
/// The caller supplies (and is responsible for displaying) the recovery code.
pub fn generate_sealed(path: &Path, op_pass: &str, recovery_code: &str)
    -> Result<(SigningKey, String), KeystoreError> {
    let (sk, kid) = generate_key().map_err(|e| KeystoreError::Key(e.to_string()))?;
    let sealed = seal::seal(&sk.to_bytes(), op_pass, recovery_code)
        .map_err(|e| KeystoreError::Key(e.to_string()))?;
    write_key_file(path, &seal::to_cbor(&sealed))?;
    Ok((sk, kid))
}

/// Migrate an existing plaintext key file to the sealed format. Errors if the file
/// is already sealed (no double-seal) or is not a 32-byte seed.
///
/// This is the ONLY path that overwrites a live node's sole plaintext key in place.
/// After writing, the file is re-read and the sealed bundle is unsealed under BOTH
/// secrets to verify the write round-tripped correctly. If either unseal fails, or
/// the recovered seed does not match, the error is loud and explicit — the operator
/// still holds the recovery code shown by the CLI and must intervene.
pub fn seal_existing(path: &Path, op_pass: &str, recovery_code: &str) -> Result<(), KeystoreError> {
    let bytes = std::fs::read(path)?;
    if seal::from_cbor(&bytes).is_ok() {
        return Err(KeystoreError::Key("key is already sealed".into()));
    }
    let seed: [u8; 32] = bytes.as_slice().try_into()
        .map_err(|_| KeystoreError::Key("not a 32-byte plaintext key".into()))?;
    let sealed = seal::seal(&seed, op_pass, recovery_code)
        .map_err(|e| KeystoreError::Key(e.to_string()))?;
    write_key_file(path, &seal::to_cbor(&sealed))?;

    // Read-after-write integrity check: re-read the file, parse it, and unseal under
    // BOTH recipients. A bad write, a serialization edge, or a truncation would be
    // silently accepted without this — and the operator would lose the key. We use the
    // per-recipient helpers (one Argon2 derivation each) rather than the agnostic
    // `unseal` (which would re-try the op path for the recovery code), halving the
    // memory-hard cost of this check — it runs on every migration, incl. on Pi-class nodes.
    let readback = std::fs::read(path)?;
    let readback_sealed = seal::from_cbor(&readback)
        .map_err(|e| KeystoreError::Key(
            format!("seal verification failed after write (parse): {e}")))?;
    let seed_op = seal::unseal_op(&readback_sealed, op_pass)
        .ok_or_else(|| KeystoreError::Key(
            "seal verification failed after write: op passphrase did not unseal".into()))?;
    let seed_rec = seal::unseal_rec(&readback_sealed, recovery_code)
        .ok_or_else(|| KeystoreError::Key(
            "seal verification failed after write: recovery code did not unseal".into()))?;
    if seed_op != seed || seed_rec != seed {
        return Err(KeystoreError::Key(
            "seal verification failed after write: recovered seed does not match original".into()));
    }
    Ok(())
}

/// Load the signing key, auto-detecting sealed vs plaintext. A sealed file requires
/// `secret` (operational passphrase OR recovery code); a missing/wrong secret yields
/// a legible error, never a panic. A plaintext file ignores `secret`.
pub fn load(path: &Path, secret: Option<&str>) -> Result<SigningKey, KeystoreError> {
    let bytes = std::fs::read(path)?;
    if let Ok(sealed) = seal::from_cbor(&bytes) {
        let secret = secret.ok_or(KeystoreError::Sealed)?;
        let seed = seal::unseal(&sealed, secret).ok_or_else(|| KeystoreError::Key(
            "cannot unseal key: wrong passphrase/recovery code or corrupt file".into()))?;
        Ok(SigningKey::from_bytes(&seed))
    } else {
        let seed: [u8; 32] = bytes.as_slice().try_into()
            .map_err(|_| KeystoreError::Key("not a sealed bundle and not a 32-byte seed".into()))?;
        Ok(SigningKey::from_bytes(&seed))
    }
}

/// Inspect the at-rest posture without needing the secret (for `status`).
pub fn key_at_rest_state(path: &Path) -> KeyAtRest {
    match std::fs::read(path) {
        // NotFound is the ONLY genuinely-absent case; any other read error (e.g. permission
        // denied) means the file is present but unreadable, so we cannot vouch for its state.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => KeyAtRest::Missing,
        Err(_) => KeyAtRest::Corrupt, // present but unreadable — caller can't trust the state
        Ok(bytes) => {
            if let Ok(sealed) = seal::from_cbor(&bytes) {
                KeyAtRest::Sealed { dual_recipient: sealed.has_recovery_wrap() }
            } else if bytes.len() == 32 {
                KeyAtRest::Plaintext
            } else {
                KeyAtRest::Corrupt
            }
        }
    }
}

/// The sibling temp path used for an atomic write: the target's full filename plus a
/// `.tmp` suffix, in the SAME directory. Pure (no I/O) so it is trivially testable.
/// Same-directory matters: `rename` is only atomic within one filesystem, so a temp in
/// `/tmp` (possibly a different mount) would defeat the whole point.
fn tmp_sibling(path: &Path) -> std::path::PathBuf {
    let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".tmp");
    path.with_file_name(name)
}

/// Atomically write `bytes` to `path` with owner-only (0600) permissions.
///
/// WHY ATOMIC (issue #45): `seal_existing` (the `seal-key` migration) overwrites a live
/// node's SOLE key in place, and a sealed bundle is hundreds of bytes. Writing the target
/// directly with `truncate` means a process kill or power loss mid-write leaves a partial
/// file that boots to `KeyAtRest::Corrupt` — the node loses its identity. Instead we write
/// to a sibling temp file, fsync it so the bytes are durable, then `rename` it over the
/// target. A POSIX `rename` within one directory is atomic: any reader (concurrent, or
/// after a crash) sees either the complete OLD file or the complete NEW one, never a torn
/// mix. A crash before the rename simply leaves the intact original untouched.
#[cfg(unix)]
fn write_key_file(path: &Path, bytes: &[u8]) -> Result<(), KeystoreError> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let tmp = tmp_sibling(path);
    // create+truncate clobbers any stale temp a previous crashed write may have left.
    // mode(0o600) is set on the temp inode; rename preserves it, so the target ends 0600.
    let mut f = std::fs::OpenOptions::new()
        .write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
    // Write + fsync as one fallible block; on ANY failure remove the temp so we never
    // leave litter, and leave the original target untouched (the rename never happened).
    let write_then_sync = (|| -> std::io::Result<()> {
        f.write_all(bytes)?;
        f.sync_all()?; // bytes durable on disk BEFORE we expose them via the rename
        Ok(())
    })();
    if let Err(e) = write_then_sync {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    drop(f); // close before rename (tidy on all platforms)
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
}
#[cfg(not(unix))]
fn write_key_file(path: &Path, bytes: &[u8]) -> Result<(), KeystoreError> {
    // Same atomic tmp+rename discipline as the unix path, minus the 0600 mode (POSIX
    // perms do not apply). `std::fs::rename` replaces an existing target on Windows too.
    let tmp = tmp_sibling(path);
    if let Err(e) = std::fs::write(&tmp, bytes) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn sealed_key_roundtrips_via_both_secrets() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("node.key");
        let (sk, _kid) = generate_sealed(&p, "op", "REC-CODE").unwrap();
        assert_eq!(load(&p, Some("op")).unwrap().to_bytes(), sk.to_bytes());
        assert_eq!(load(&p, Some("REC-CODE")).unwrap().to_bytes(), sk.to_bytes());
        // No secret on a sealed key yields the DISTINCT `Sealed` variant (so the CLI
        // can decide to prompt), not a generic Key error.
        assert!(matches!(load(&p, None), Err(KeystoreError::Sealed)),
            "sealed key with no secret must return the Sealed variant");
        assert!(load(&p, Some("wrong")).is_err());
        assert!(matches!(key_at_rest_state(&p), KeyAtRest::Sealed { dual_recipient: true }));
    }

    #[test]
    fn plaintext_key_loads_without_secret() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("node.key");
        let (sk, _kid) = generate_plaintext(&p).unwrap();
        assert_eq!(load(&p, None).unwrap().to_bytes(), sk.to_bytes());
        assert!(matches!(key_at_rest_state(&p), KeyAtRest::Plaintext));
    }

    #[test]
    fn seal_existing_migrates_plaintext_then_blocks_plaintext_load() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("node.key");
        let (sk, _kid) = generate_plaintext(&p).unwrap();
        seal_existing(&p, "op", "REC-CODE").unwrap();
        // Both recipients must survive migration: op passphrase and recovery code.
        assert_eq!(load(&p, Some("op")).unwrap().to_bytes(), sk.to_bytes(),
            "op passphrase must unseal migrated key");
        assert_eq!(load(&p, Some("REC-CODE")).unwrap().to_bytes(), sk.to_bytes(),
            "recovery code must unseal migrated key (off-node escrow path)");
        assert!(load(&p, None).is_err(), "after sealing, no-secret load must fail");
        assert!(seal_existing(&p, "op", "REC-CODE").is_err(), "double-seal must error");
    }

    #[test]
    fn tmp_sibling_appends_tmp_suffix_in_same_dir() {
        // The atomic-write temp file must live in the SAME directory as the target
        // (rename is only atomic within one filesystem) and be named so it can never be
        // mistaken for the key itself.
        let p = Path::new("/var/lib/cairn/node.key");
        let t = tmp_sibling(p);
        assert_eq!(t, Path::new("/var/lib/cairn/node.key.tmp"));
        assert_eq!(t.parent(), p.parent(), "temp must be a sibling of the target");
    }

    #[test]
    fn write_is_atomic_and_leaves_no_temp_litter() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("node.key");
        generate_sealed(&p, "op", "REC-CODE").unwrap();
        // The temp sibling used during the atomic write must be cleaned up (renamed away).
        assert!(!tmp_sibling(&p).exists(), "atomic write must not leave a .tmp sibling");
        assert!(matches!(key_at_rest_state(&p), KeyAtRest::Sealed { .. }));
    }

    #[test]
    fn stale_temp_from_a_prior_crashed_write_is_overwritten() {
        // A previous crash could leave a stale `<key>.tmp`. A new write must clobber it
        // (truncate) and still succeed, never appending to or being confused by the junk.
        let dir = tempdir().unwrap();
        let p = dir.path().join("node.key");
        std::fs::write(tmp_sibling(&p), b"garbage from a half-finished write").unwrap();
        let (sk, _kid) = generate_sealed(&p, "op", "REC-CODE").unwrap();
        assert_eq!(load(&p, Some("op")).unwrap().to_bytes(), sk.to_bytes());
        assert!(!tmp_sibling(&p).exists(), "stale temp must be gone after a successful write");
    }

    #[cfg(unix)]
    #[test]
    fn written_key_has_owner_only_permissions() {
        // The atomic write creates the temp file 0600 and rename keeps that inode, so the
        // final key must be owner-read/write only — a regression that drops the mode on the
        // temp would leak the sealed bundle to other local users.
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().unwrap();
        let p = dir.path().join("node.key");
        generate_sealed(&p, "op", "REC-CODE").unwrap();
        let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "key file must be owner-read/write only");
    }

    #[test]
    fn state_reports_missing_and_corrupt() {
        let dir = tempdir().unwrap();
        assert!(matches!(key_at_rest_state(&dir.path().join("nope.key")), KeyAtRest::Missing));
        let bad = dir.path().join("bad.key");
        std::fs::write(&bad, b"only 5").unwrap(); // not 32 bytes, not a bundle
        assert!(matches!(key_at_rest_state(&bad), KeyAtRest::Corrupt));
    }
}
