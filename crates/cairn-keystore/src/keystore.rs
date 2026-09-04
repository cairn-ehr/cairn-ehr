use crate::seal;
use cairn_event::keys::{PublicKey32, Secret32};
use cairn_event::{generate_key, SigningKey};
use std::path::{Path, PathBuf};

#[derive(thiserror::Error, Debug)]
pub enum KeystoreError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("key material: {0}")]
    Key(String),
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
    // Wipe the seed temporary on drop (#213: seal.rs is scrupulous about this — issue
    // #54 — but the discipline stopped one layer up, in this file).
    let seed = zeroize::Zeroizing::new(sk.to_bytes());
    crate::fsio::atomic_write(path, seed.as_ref(), Some(0o600))?;
    Ok((sk, kid))
}

/// Generate a keypair and write it SEALED under both secrets (ADR-0026 slice A).
/// The caller supplies (and is responsible for displaying) the recovery code.
pub fn generate_sealed(
    path: &Path,
    op_pass: &str,
    recovery_code: &str,
) -> Result<(SigningKey, String), KeystoreError> {
    let (sk, kid) = generate_key().map_err(|e| KeystoreError::Key(e.to_string()))?;
    // The Ed25519 SIGNING seed. `Secret32` does not distinguish it from an unwrap secret
    // (#511 §2) — the distinction here is carried by the PATH this is written to, and by
    // `unwrap_secret_is_the_signing_seed` below, which exists precisely because the two
    // files are byte-format indistinguishable.
    let seed = Secret32::from_bytes(sk.to_bytes());
    let sealed =
        seal::seal(&seed, op_pass, recovery_code).map_err(|e| KeystoreError::Key(e.to_string()))?;
    crate::fsio::atomic_write(path, &seal::to_cbor(&sealed), Some(0o600))?;
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
    // The file content IS the plaintext seed on this path — hold both the raw read
    // and the fixed-size copy in Zeroizing so neither outlives its use (#213).
    let bytes = zeroize::Zeroizing::new(std::fs::read(path)?);
    if seal::from_cbor(&bytes).is_ok() {
        return Err(KeystoreError::Key("key is already sealed".into()));
    }
    let seed = Secret32::from_slice(&bytes)
        .ok_or_else(|| KeystoreError::Key("not a 32-byte plaintext key".into()))?;
    let sealed =
        seal::seal(&seed, op_pass, recovery_code).map_err(|e| KeystoreError::Key(e.to_string()))?;
    crate::fsio::atomic_write(path, &seal::to_cbor(&sealed), Some(0o600))?;

    // Read-after-write integrity check: re-read the file, parse it, and unseal under
    // BOTH recipients. A bad write, a serialization edge, or a truncation would be
    // silently accepted without this — and the operator would lose the key. We use the
    // per-recipient helpers (one Argon2 derivation each) rather than the agnostic
    // `unseal` (which would re-try the op path for the recovery code), halving the
    // memory-hard cost of this check — it runs on every migration, incl. on Pi-class nodes.
    let readback = std::fs::read(path)?;
    let readback_sealed = seal::from_cbor(&readback).map_err(|e| {
        KeystoreError::Key(format!("seal verification failed after write (parse): {e}"))
    })?;
    let seed_op = seal::unseal_op(&readback_sealed, op_pass).ok_or_else(|| {
        KeystoreError::Key(
            "seal verification failed after write: op passphrase did not unseal".into(),
        )
    })?;
    let seed_rec = seal::unseal_rec(&readback_sealed, recovery_code).ok_or_else(|| {
        KeystoreError::Key(
            "seal verification failed after write: recovery code did not unseal".into(),
        )
    })?;
    // `Secret32`'s `PartialEq` is constant-time (#511); the bytes never leave the wrapper.
    if seed_op != seed || seed_rec != seed {
        return Err(KeystoreError::Key(
            "seal verification failed after write: recovered seed does not match original".into(),
        ));
    }
    Ok(())
}

/// Load the signing key, auto-detecting sealed vs plaintext. A sealed file requires
/// `secret` (operational passphrase OR recovery code); a missing/wrong secret yields
/// a legible error, never a panic. A plaintext file ignores `secret`.
pub fn load(path: &Path, secret: Option<&str>) -> Result<SigningKey, KeystoreError> {
    // On the plaintext branch the raw read IS the seed — Zeroizing on both the Vec
    // and the fixed-size copy so neither survives the load (#213). Harmless on the
    // sealed branch (the bundle is ciphertext).
    let bytes = zeroize::Zeroizing::new(std::fs::read(path)?);
    if let Ok(sealed) = seal::from_cbor(&bytes) {
        let secret = secret.ok_or(KeystoreError::Sealed)?;
        let seed = seal::unseal(&sealed, secret).ok_or_else(|| {
            KeystoreError::Key(
                "cannot unseal key: wrong passphrase/recovery code or corrupt file".into(),
            )
        })?;
        Ok(SigningKey::from_bytes(seed.as_bytes()))
    } else {
        let seed = Secret32::from_slice(&bytes).ok_or_else(|| {
            KeystoreError::Key("not a sealed bundle and not a 32-byte seed".into())
        })?;
        Ok(SigningKey::from_bytes(seed.as_bytes()))
    }
}

/// The unwrap-key file for a signing-key path: `<key>.unwrap`, a sibling — discoverable
/// from what every command already has, exactly like the `.lsk` sidecar. Pure.
///
/// Built on `fsio::sibling_with_suffix`, the same same-directory-sibling naming
/// `fsio::tmp_sibling` uses for its `.tmp` sidecar — one naming rule for both, so a
/// future change to how a sibling path is formed can't quietly diverge between them.
pub fn unwrap_key_path_for(key: &Path) -> PathBuf {
    crate::fsio::sibling_with_suffix(key, ".unwrap")
}

/// Mint this node's INDEPENDENT X25519 unwrap secret and write it sealed (mode 0600)
/// under both operator secrets. Returns the PUBLIC half, which the caller registers in
/// the database (`cairn_register_unwrap_key`) — the secret half never enters the DB, so a
/// database backup alone can never unwrap a DEK.
///
/// The sealed format is the SAME dual-recipient bundle the signing key uses: both are
/// 32-byte secrets, so `seal::seal` covers this with no new ceremony and no new format
/// (ADR-0066 decision 1 — the operator still holds one passphrase and one recovery code).
///
/// **Read-after-write integrity check**, mirroring `seal_existing`'s for the signing key
/// — and for strictly higher stakes: losing the signing key to a bad write costs a node
/// *identity*, which DR deliberately re-mints anyway; losing the unwrap key this way
/// costs the *clinical record*, because the public half returned here is what every
/// subsequent `event_dek` row gets wrapped to. If the sealed bytes on disk don't actually
/// recover under BOTH secrets — a serialization edge, a lying fsync, a truncation — a
/// caller that trusted the in-memory secret would register a public half nothing can
/// ever unwrap again, reintroducing the ADR-0066 failure shape one layer up. So this
/// re-reads the file, unseals it under both recipients (one Argon2 derivation each, the
/// same halved cost `seal_existing` chose, paid once per provisioning), confirms the
/// recovered bytes match what was generated, and derives the returned public half from
/// the READBACK — never from the in-memory `secret` — so a caller only ever registers a
/// public half this function has actually proven recoverable from disk.
pub fn generate_unwrap_sealed(
    path: &Path,
    op_pass: &str,
    recovery_code: &str,
) -> Result<PublicKey32, KeystoreError> {
    let secret = cairn_event::seal::generate_unwrap_secret()
        .map_err(|e| KeystoreError::Key(e.to_string()))?;
    write_unwrap_sealed(path, &secret, op_pass, recovery_code)?;

    let readback = std::fs::read(path)?;
    let readback_sealed = seal::from_cbor(&readback).map_err(|e| {
        KeystoreError::Key(format!(
            "unwrap key verification failed after write (parse): {e}"
        ))
    })?;
    let secret_op = seal::unseal_op(&readback_sealed, op_pass).ok_or_else(|| {
        KeystoreError::Key(
            "unwrap key verification failed after write: op passphrase did not unseal".into(),
        )
    })?;
    let secret_rec = seal::unseal_rec(&readback_sealed, recovery_code).ok_or_else(|| {
        KeystoreError::Key(
            "unwrap key verification failed after write: recovery code did not unseal".into(),
        )
    })?;
    if secret_op != secret || secret_rec != secret {
        return Err(KeystoreError::Key(
            "unwrap key verification failed after write: recovered secret does not match \
             the one generated"
                .into(),
        ));
    }
    Ok(cairn_event::seal::unwrap_public(&secret_op))
}

/// Write a KNOWN unwrap secret sealed under both operator secrets. Two callers, both
/// carrying a secret that must not change: the ADR-0066 adoption migration (which keeps a
/// pre-ADR-0066 node's existing `event_dek` rows openable) and `restore`, which installs
/// the dead node's unwrap secret so the restored node inherits its custody.
pub fn write_unwrap_sealed(
    path: &Path,
    secret: &Secret32,
    op_pass: &str,
    recovery_code: &str,
) -> Result<(), KeystoreError> {
    // `secret` is already the exact shape `seal::seal` wants and owns its own wiping, so
    // there is nothing to copy here.
    let sealed = seal::seal(secret, op_pass, recovery_code)
        .map_err(|e| KeystoreError::Key(e.to_string()))?;
    crate::fsio::atomic_write(path, &seal::to_cbor(&sealed), Some(0o600))?;
    Ok(())
}

/// Write a KNOWN unwrap secret UNSEALED (mode 0600) — the custody-plane counterpart of
/// [`generate_plaintext`] for the signing key, and used on exactly the same path: a node
/// provisioned with `--insecure-plaintext`, where no operator passphrase and no recovery
/// code exist to seal anything under.
///
/// WHY THIS EXISTS AT ALL (a junior reader will reasonably ask why we would write a key
/// in the clear). ADR-0066 decision 6 moved unwrap-key registration out of the write path
/// and into provisioning. A throwaway test node has an unsealed signing key by explicit
/// operator choice; without an unwrap key beside it, that node could no longer write a
/// single clinical event, because every sealed body's DEK is wrapped to the node's unwrap
/// key. So the custody key follows the signing key's at-rest posture: sealed beside a
/// sealed key, plaintext beside a plaintext one. Never use this for a real node — the
/// escrow does not exist for a plaintext key (key loss = record loss).
pub fn write_unwrap_plaintext(path: &Path, secret: &Secret32) -> Result<(), KeystoreError> {
    crate::fsio::atomic_write(path, secret.as_bytes(), Some(0o600))?;
    Ok(())
}

/// Mint this node's INDEPENDENT X25519 unwrap secret and write it UNSEALED (mode 0600),
/// returning the PUBLIC half for the caller to register. The `--insecure-plaintext`
/// counterpart of [`generate_unwrap_sealed`]; see [`write_unwrap_plaintext`] for why the
/// unsealed variant exists and when it is legitimate.
///
/// Carries the same read-after-write check its sealed sibling does, and for the same
/// reason: the returned public half is what every subsequent `event_dek` row gets wrapped
/// to, so registering one this function has not proven readable back from disk would leave
/// the node unable to open its own custody — the ADR-0066 failure shape one layer up.
pub fn generate_unwrap_plaintext(path: &Path) -> Result<PublicKey32, KeystoreError> {
    let secret = cairn_event::seal::generate_unwrap_secret()
        .map_err(|e| KeystoreError::Key(e.to_string()))?;
    write_unwrap_plaintext(path, &secret)?;

    let readback = load_unwrap_secret(path, None)?;
    if readback != secret {
        return Err(KeystoreError::Key(
            "unwrap key verification failed after write: the bytes read back do not match \
             the secret generated"
                .into(),
        ));
    }
    // Derive the public half from the READBACK, never the in-memory secret, so a caller
    // only ever registers a key this function has actually proven recoverable from disk.
    Ok(cairn_event::seal::unwrap_public(&readback))
}

/// Load the node's unwrap secret, auto-detecting sealed vs plaintext exactly as [`load`]
/// does for the signing key — including the distinct [`KeystoreError::Sealed`] variant, so
/// the CLI can prompt for the passphrase from ONE load attempt with no TOCTOU-prone
/// pre-classification read.
pub fn load_unwrap_secret(path: &Path, secret: Option<&str>) -> Result<Secret32, KeystoreError> {
    let bytes = zeroize::Zeroizing::new(std::fs::read(path)?);
    if let Ok(sealed) = seal::from_cbor(&bytes) {
        let secret = secret.ok_or(KeystoreError::Sealed)?;
        seal::unseal(&sealed, secret).ok_or_else(|| {
            KeystoreError::Key(
                "cannot unseal the unwrap key: wrong passphrase/recovery code or corrupt file"
                    .into(),
            )
        })
    } else {
        Secret32::from_slice(&bytes).ok_or_else(|| {
            KeystoreError::Key("not a sealed bundle and not a 32-byte unwrap secret".into())
        })
    }
}

/// ADR-0066 decision 5 — THE MIGRATION, and the only production caller of
/// `derive_unwrap_secret` outside `cairn-sync`'s pre-ADR-0066 startup fallback
/// (`unwrap_key::resolve_at_startup`). Both are named, with their reasons, in
/// `crates/cairn-node/tests/unwrap_secret_is_not_derived.rs`'s `ALLOWED` list — which is the
/// authority here, not this sentence.
///
/// A node provisioned before ADR-0066 wrapped every `event_dek` row to the public half of
/// the secret HKDF-derived from its signing seed. Re-deriving it once and adopting it as
/// that node's first INDEPENDENT key keeps all of them openable — no rewrap, no migration
/// of custody rows, nothing to get wrong at 3am. It works only while the signing seed
/// still reconstructs the old secret, which is why this migration is cheap now and never
/// cheaper.
pub fn adopt_derived_unwrap_secret(sk: &SigningKey) -> Secret32 {
    // ⚠️ ONE OF THE TWO PRODUCTION LINES that turn the Ed25519 signing seed into this node's
    // UNWRAP SECRET — this one and `cairn-sync`'s `resolve_at_startup` fallback. (The tree holds
    // other `Secret32::from_bytes` calls, including two in this very file, but they mint fresh
    // CSPRNG output, seal the seed AS the seed, or compare; none of them installs a custody key.
    // The full inventory, per file and by count, is pinned in
    // `crates/cairn-node/tests/secret32_conversions_are_named.rs` — read it there, not from a
    // number in a comment. An earlier version of this line said "THE ONE PRODUCTION LINE IN THE
    // TREE" and was contradicted 40 lines below by its own file.)
    //
    // #511's newtypes make a PUBLIC-for-secret mix-up a compile error, but they deliberately do
    // NOT separate one secret role from another — an unwrap secret, a signing seed and a DEK are
    // all `Secret32`. So this conversion is exactly the shape that, written anywhere else, IS the
    // #495 coupling. Two guards cover it: `unwrap_secret_is_not_derived.rs` pins which FILES may
    // call `derive_unwrap_secret`, and `secret32_conversions_are_named.rs` pins the conversion
    // count in each. Either one reddening is the guard working — do not edit the number to match.
    let seed = Secret32::from_bytes(sk.to_bytes());
    cairn_event::seal::derive_unwrap_secret(&seed)
}

/// True iff `unwrap` IS the node's Ed25519 signing seed, rather than a genuine unwrap
/// secret — the shape a `node.key.unwrap` file takes after a file-swap accident (a
/// fat-fingered restore, a path bug, an rsync of the wrong sibling — e.g. `cp node.key
/// node.key.unwrap`).
///
/// This matters because the two files are byte-format INDISTINGUISHABLE: same
/// `CAIRNK1` magic, same dual-recipient escrow, no purpose tag anywhere in the sealed
/// bundle. So a swapped file unseals SUCCESSFULLY — `load_unwrap_secret` reports no
/// error at all — and silently hands back the signing seed as though it were the
/// node's unwrap secret. Every DEK from that moment is wrapped to a key derived from
/// the signing seed: the exact ADR-0066 coupling this whole task exists to break,
/// reintroduced by accident, with every surface reporting success.
///
/// A plain byte comparison is the right test, and it cannot false-positive on either
/// LEGITIMATE case:
/// - A GENERATED secret ([`cairn_event::seal::generate_unwrap_secret`]) is drawn fresh
///   from the OS CSPRNG, independent of the signing seed — colliding with it by chance
///   is a 2^-256 event.
/// - An ADOPTED secret ([`adopt_derived_unwrap_secret`]) is the HKDF *derivation* of the
///   seed (domain-separated by the `cairn-node-unwrap-x25519-v1` info tag), which is
///   computationally indistinguishable from random and is never equal to its own input
///   key material — an equality here would mean a preimage break of HKDF, not a
///   legitimate adoption.
///
/// So `unwrap == sk.to_bytes()` can mean only one thing: the file was never produced by
/// [`generate_unwrap_sealed`] or [`adopt_derived_unwrap_secret`] at all — it IS the
/// signing key's content, misplaced.
///
/// Deliberately NOT wired into [`load_unwrap_secret`] itself: that loader is also the
/// path a future `restore` uses to install a DEAD node's unwrap secret, where the live
/// signing key is a different key entirely and this comparison would be meaningless. The
/// check belongs where both keys are genuinely this node's — `main.rs`'s
/// `establish-unwrap-key`, via `load_unwrap_secret_or_refuse_swapped_file`, which refuses
/// the command rather than registering a public half derived from the signing seed.
pub fn unwrap_secret_is_the_signing_seed(unwrap: &Secret32, sk: &SigningKey) -> bool {
    // Constant-time via `Secret32`'s `PartialEq`. This conversion is a COMPARISON, never an
    // installation, which is why it is safe where `adopt_derived_unwrap_secret`'s is delicate:
    // the `Secret32` built here is dropped (and wiped) before this function returns, and nothing
    // it touches reaches a keystore file or the `node_unwrap_key` singleton.
    *unwrap == Secret32::from_bytes(sk.to_bytes())
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
                KeyAtRest::Sealed {
                    dual_recipient: sealed.has_recovery_wrap(),
                }
            } else if bytes.len() == 32 {
                KeyAtRest::Plaintext
            } else {
                KeyAtRest::Corrupt
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fsio::tmp_sibling; // the atomic-write mechanics now live in fsio
    use tempfile::tempdir;

    /// House rule 6 (CLAUDE.md / issue #146): an operational-passphrase or recovery-code
    /// fixture built at RUNTIME, never a literal. These strings reach Argon2id key
    /// derivation via `seal()`/`unseal()` below — a crypto context — so a hard-coded one
    /// trips CodeQL's `rust/hard-coded-cryptographic-value` (critical) exactly like a
    /// hard-coded byte array does. The same `tag` always yields the same string, so a
    /// seal/load round-trip stays deterministic; a different tag yields a genuinely
    /// different secret, which is what a "wrong secret" test needs to still test
    /// something.
    fn secret_fixture(tag: u8) -> String {
        (0..16u8)
            .map(|i| char::from(b'a' + ((tag ^ i) % 26)))
            .collect()
    }

    #[test]
    fn sealed_key_roundtrips_via_both_secrets() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("node.key");
        let op = secret_fixture(1);
        let rec = secret_fixture(2);
        let (sk, _kid) = generate_sealed(&p, &op, &rec).unwrap();
        assert_eq!(
            load(&p, Some(op.as_str())).unwrap().to_bytes(),
            sk.to_bytes()
        );
        assert_eq!(
            load(&p, Some(rec.as_str())).unwrap().to_bytes(),
            sk.to_bytes()
        );
        // No secret on a sealed key yields the DISTINCT `Sealed` variant (so the CLI
        // can decide to prompt), not a generic Key error.
        assert!(
            matches!(load(&p, None), Err(KeystoreError::Sealed)),
            "sealed key with no secret must return the Sealed variant"
        );
        assert!(load(&p, Some(secret_fixture(3).as_str())).is_err());
        assert!(matches!(
            key_at_rest_state(&p),
            KeyAtRest::Sealed {
                dual_recipient: true
            }
        ));
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
        let op = secret_fixture(1);
        let rec = secret_fixture(2);
        seal_existing(&p, &op, &rec).unwrap();
        // Both recipients must survive migration: op passphrase and recovery code.
        assert_eq!(
            load(&p, Some(op.as_str())).unwrap().to_bytes(),
            sk.to_bytes(),
            "op passphrase must unseal migrated key"
        );
        assert_eq!(
            load(&p, Some(rec.as_str())).unwrap().to_bytes(),
            sk.to_bytes(),
            "recovery code must unseal migrated key (off-node escrow path)"
        );
        assert!(
            load(&p, None).is_err(),
            "after sealing, no-secret load must fail"
        );
        assert!(
            seal_existing(&p, &op, &rec).is_err(),
            "double-seal must error"
        );
    }

    #[test]
    fn write_is_atomic_and_leaves_no_temp_litter() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("node.key");
        generate_sealed(&p, &secret_fixture(1), &secret_fixture(2)).unwrap();
        // The temp sibling used during the atomic write must be cleaned up (renamed away).
        assert!(
            !tmp_sibling(&p).exists(),
            "atomic write must not leave a .tmp sibling"
        );
        assert!(matches!(key_at_rest_state(&p), KeyAtRest::Sealed { .. }));
    }

    #[test]
    fn stale_temp_from_a_prior_crashed_write_is_overwritten() {
        // A previous crash could leave a stale `<key>.tmp`. A new write must clobber it
        // (truncate) and still succeed, never appending to or being confused by the junk.
        let dir = tempdir().unwrap();
        let p = dir.path().join("node.key");
        std::fs::write(tmp_sibling(&p), b"garbage from a half-finished write").unwrap();
        let op = secret_fixture(1);
        let (sk, _kid) = generate_sealed(&p, &op, &secret_fixture(2)).unwrap();
        assert_eq!(
            load(&p, Some(op.as_str())).unwrap().to_bytes(),
            sk.to_bytes()
        );
        assert!(
            !tmp_sibling(&p).exists(),
            "stale temp must be gone after a successful write"
        );
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
        generate_sealed(&p, &secret_fixture(1), &secret_fixture(2)).unwrap();
        let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "key file must be owner-read/write only");
    }

    #[cfg(unix)]
    #[test]
    fn stale_temp_with_wide_perms_does_not_leak_into_the_key_mode() {
        // `OpenOptions::mode()` only applies when open CREATES the inode. A stale `<key>.tmp`
        // left wider than 0600 (a foreign tool, a manual op, a different-umask process) is
        // reused with its OLD perms by create+truncate, and rename would then carry that
        // wider mode onto the key. The write MUST force 0600 regardless of the temp's prior
        // perms — otherwise a `--insecure-plaintext` seed or a sealed bundle leaks to other
        // local users.
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().unwrap();
        let p = dir.path().join("node.key");
        let tmp = tmp_sibling(&p);
        std::fs::write(&tmp, b"stale junk from a foreign write").unwrap();
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644)).unwrap();
        generate_sealed(&p, &secret_fixture(1), &secret_fixture(2)).unwrap();
        let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "a stale wide-perm temp must not leak its mode into the key"
        );
    }

    #[test]
    fn state_reports_missing_and_corrupt() {
        let dir = tempdir().unwrap();
        assert!(matches!(
            key_at_rest_state(&dir.path().join("nope.key")),
            KeyAtRest::Missing
        ));
        let bad = dir.path().join("bad.key");
        std::fs::write(&bad, b"only 5").unwrap(); // not 32 bytes, not a bundle
        assert!(matches!(key_at_rest_state(&bad), KeyAtRest::Corrupt));
    }

    /// Owner-only (0600) mode check for a written key/unwrap file, factored out so the
    /// roundtrip test below can make good on its own name (`..._and_is_owner_only`)
    /// without duplicating the assertion `written_key_has_owner_only_permissions`
    /// already carries for the signing key. A no-op on non-unix, where POSIX modes
    /// don't apply (mirrors `fsio::atomic_write`'s own unix/non-unix split).
    #[cfg(unix)]
    fn assert_owner_only_mode(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "file must be owner-read/write only: {path:?}");
    }
    #[cfg(not(unix))]
    fn assert_owner_only_mode(_path: &Path) {}

    #[test]
    fn unwrap_key_roundtrips_under_both_secrets_and_is_owner_only() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("node.unwrap");
        let op = secret_fixture(1);
        let rec = secret_fixture(2);
        let public = generate_unwrap_sealed(&p, &op, &rec).unwrap();

        let via_op = load_unwrap_secret(&p, Some(op.as_str())).unwrap();
        let via_rec = load_unwrap_secret(&p, Some(rec.as_str())).unwrap();
        assert_eq!(via_op, via_rec, "both secrets recover the same key");
        assert_eq!(
            cairn_event::seal::unwrap_public(&via_op),
            public,
            "the returned public half must match the sealed secret's"
        );
        assert!(
            matches!(load_unwrap_secret(&p, None), Err(KeystoreError::Sealed)),
            "a sealed unwrap key with no secret returns the distinct Sealed variant"
        );
        // The name promises "is_owner_only" — make good on it, exactly like the signing
        // key's dedicated `written_key_has_owner_only_permissions`.
        assert_owner_only_mode(&p);
    }

    #[test]
    fn an_adopted_secret_still_opens_a_dek_wrapped_before_adoption() {
        // The migration promise (ADR-0066 decision 5): a node provisioned before ADR-0066 has
        // event_dek rows wrapped to its DERIVED public half. Adoption must keep every one of
        // them openable — that is what makes the migration lossless and rewrap-free.
        let dir = tempdir().unwrap();
        let p = dir.path().join("node.unwrap");
        let (sk, _kid) = cairn_event::generate_key().unwrap();

        // A DEK wrapped the OLD way, before adoption. House rule 6: derived at runtime.
        let old_secret =
            cairn_event::seal::derive_unwrap_secret(&Secret32::from_bytes(sk.to_bytes()));
        let dek = Secret32::from_bytes(std::array::from_fn(|i| {
            (i as u8).wrapping_mul(7).wrapping_add(3)
        }));
        let wrapped =
            cairn_event::seal::wrap_dek_for(&dek, &cairn_event::seal::unwrap_public(&old_secret))
                .unwrap();

        let adopted = adopt_derived_unwrap_secret(&sk);
        let op = secret_fixture(1);
        write_unwrap_sealed(&p, &adopted, &op, &secret_fixture(2)).unwrap();

        let loaded = load_unwrap_secret(&p, Some(op.as_str())).unwrap();
        assert_eq!(
            cairn_event::seal::unwrap_dek(&wrapped, &loaded)
                .expect("an adopted key must open a pre-adoption wrap"),
            dek,
            "adoption is lossless: no event_dek row needs rewrapping"
        );
    }

    #[test]
    fn unwrap_key_at_rest_reports_missing_sealed_and_corrupt() {
        let dir = tempdir().unwrap();
        assert!(matches!(
            key_at_rest_state(&dir.path().join("nope.unwrap")),
            KeyAtRest::Missing
        ));
        let p = dir.path().join("node.unwrap");
        generate_unwrap_sealed(&p, &secret_fixture(1), &secret_fixture(2)).unwrap();
        assert!(matches!(
            key_at_rest_state(&p),
            KeyAtRest::Sealed {
                dual_recipient: true
            }
        ));
        // The name promises "and_corrupt" too — exercise it, exactly like the signing
        // key's `state_reports_missing_and_corrupt` (neither 32 bytes nor a sealed bundle).
        let bad = dir.path().join("bad.unwrap");
        std::fs::write(&bad, b"only 5").unwrap();
        assert!(matches!(key_at_rest_state(&bad), KeyAtRest::Corrupt));
    }

    #[test]
    fn unwrap_key_path_for_appends_unwrap_suffix_in_same_dir() {
        // Pinned exactly, modelled on fsio's own `tmp_sibling_appends_tmp_suffix_in_same_dir`:
        // if this were ever "simplified" to `with_extension("unwrap")`, `node.key` would map
        // to `node.unwrap` instead of `node.key.unwrap`. A restored/existing deployment would
        // then find no file at the new path, and provisioning could mint a FRESH unwrap key
        // that orphans every existing `event_dek` row — #495 all over again.
        let p = Path::new("/var/lib/cairn/node.key");
        let u = unwrap_key_path_for(p);
        assert_eq!(u, Path::new("/var/lib/cairn/node.key.unwrap"));
        assert_eq!(
            u.parent(),
            p.parent(),
            "the unwrap file must be a sibling of the signing key"
        );
    }

    #[test]
    fn unwrap_secret_is_the_signing_seed_detects_the_swap_but_not_legitimate_secrets() {
        let (sk, _kid) = cairn_event::generate_key().unwrap();

        // The file-swap accident this predicate exists to catch: node.unwrap holding
        // the signing seed verbatim (e.g. `cp node.key node.key.unwrap`).
        assert!(
            unwrap_secret_is_the_signing_seed(&Secret32::from_bytes(sk.to_bytes()), &sk),
            "the exact signing seed must be caught"
        );

        // A genuinely GENERATED secret is independent of the seed — must never false-positive.
        let generated = cairn_event::seal::generate_unwrap_secret().unwrap();
        assert!(
            !unwrap_secret_is_the_signing_seed(&generated, &sk),
            "an independently generated secret must not be flagged as the signing seed"
        );

        // An ADOPTED secret is the HKDF *derivation* of the seed, never the seed itself —
        // must also never false-positive (this is the legitimate migration path, I4).
        let adopted = adopt_derived_unwrap_secret(&sk);
        assert!(
            !unwrap_secret_is_the_signing_seed(&adopted, &sk),
            "an adopted (derived) secret is a transform of the seed, not the seed itself, \
             and must not be flagged"
        );
    }
}
