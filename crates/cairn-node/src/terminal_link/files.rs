//! The two files a terminal keeps in its own directory (#909 slice B, design D4 step 2).
//!
//! * `terminal.key` — the terminal's own Ed25519 key, generated on first launch. It is the
//!   terminal's TLS identity at the gateway: the node admits it by fingerprint, so losing the file
//!   means admitting the terminal again, nothing worse. It is stored unsealed (mode 0600), because
//!   the window must open the link before anyone has typed anything.
//! * `node.pin` — the node's FULL public key (64 hex digits), written once by the first-launch
//!   ceremony after the node proved it holds that key. Every later connection pins it.
//!
//! Nothing here talks to the network: [`super::enrol`] produces the pin, [`super::prepare`]
//! decides when to read and write these files.

use anyhow::{bail, Context};
use cairn_event::SigningKey;
use std::path::{Path, PathBuf};

/// Where a terminal's key and node pin live. Only paths: nothing is read until asked.
pub struct TerminalFiles {
    dir: PathBuf,
}

/// Is `s` a full Ed25519 public key in hex (64 hex digits)? Pure.
fn is_full_key_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

impl TerminalFiles {
    /// The files in `dir` (created on first write if missing).
    pub fn at(dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
        }
    }

    fn key_path(&self) -> PathBuf {
        self.dir.join("terminal.key")
    }

    fn pin_path(&self) -> PathBuf {
        self.dir.join("node.pin")
    }

    /// The terminal's key: loaded if `terminal.key` exists, generated (mode 0600) if not.
    ///
    /// A key file readable by group or others is REFUSED, not repaired: someone widened it, and
    /// anyone who read it can present this terminal at the gateway. The operator decides whether
    /// to revoke the terminal; silently tightening the mode would hide the exposure.
    pub fn load_or_create_key(&self) -> anyhow::Result<SigningKey> {
        let path = self.key_path();
        if !path.exists() {
            std::fs::create_dir_all(&self.dir).with_context(|| {
                format!("creating the terminal directory {}", self.dir.display())
            })?;
            let (sk, _kid) = crate::keystore::generate_plaintext(&path)
                .with_context(|| format!("creating the terminal key {}", path.display()))?;
            return Ok(sk);
        }
        refuse_a_shared_key_file(&path)?;
        crate::keystore::load(&path, None)
            .with_context(|| format!("loading the terminal key {}", path.display()))
    }

    /// The node's pinned key, or `None` before the first-launch ceremony has written one.
    /// A pin file that is not 64 hex digits is an error (never "no pin"): treating it as absent
    /// would let the next launch re-pin from a typed fingerprint without anyone noticing.
    pub fn read_pin(&self) -> anyhow::Result<Option<String>> {
        let path = self.pin_path();
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        let pin = text.trim();
        if !is_full_key_hex(pin) {
            bail!(
                "{} is not a node key (64 hex digits); it may be damaged. Ask the node's operator \
                 before deleting it to pin the node again.",
                path.display()
            );
        }
        Ok(Some(pin.to_ascii_lowercase()))
    }

    /// Record the node's full key as this terminal's pin (atomic, mode 0600).
    pub fn write_pin(&self, hex: &str) -> anyhow::Result<()> {
        if !is_full_key_hex(hex) {
            bail!("a node pin must be a full key (64 hex digits)");
        }
        std::fs::create_dir_all(&self.dir)
            .with_context(|| format!("creating the terminal directory {}", self.dir.display()))?;
        let path = self.pin_path();
        let line = format!("{}\n", hex.to_ascii_lowercase());
        crate::fsio::atomic_write(&path, line.as_bytes(), Some(0o600))
            .with_context(|| format!("writing {}", path.display()))
    }
}

/// Refuse a key file whose mode lets group or others read or write it. Unix only: elsewhere
/// POSIX modes do not apply.
#[cfg(unix)]
fn refuse_a_shared_key_file(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(path)
        .with_context(|| format!("reading {}", path.display()))?
        .permissions()
        .mode();
    if mode & 0o077 != 0 {
        bail!(
            "the terminal key {} can be read by other users (mode {:o}); anyone who read it can \
             present this terminal. Ask the node's operator to revoke this terminal, then delete \
             the file so a new key is made.",
            path.display(),
            mode & 0o777
        );
    }
    Ok(())
}

#[cfg(not(unix))]
fn refuse_a_shared_key_file(_path: &Path) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pin_of(byte: u8) -> String {
        let raw: [u8; 32] = std::array::from_fn(|i| byte.wrapping_add(i as u8));
        hex::encode(raw)
    }

    #[test]
    fn a_new_key_is_made_at_0600_and_loads_back_the_same() {
        let dir = tempfile::tempdir().unwrap();
        let files = TerminalFiles::at(&dir.path().join("t"));
        let made = files.load_or_create_key().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join("t/terminal.key"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let again = files.load_or_create_key().unwrap();
        assert_eq!(made.to_bytes(), again.to_bytes());
    }

    #[cfg(unix)]
    #[test]
    fn a_key_readable_by_others_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let files = TerminalFiles::at(dir.path());
        files.load_or_create_key().unwrap();
        let path = dir.path().join("terminal.key");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let err = files.load_or_create_key().unwrap_err().to_string();
        assert!(err.contains("other users"), "{err}");
    }

    #[test]
    fn a_pin_round_trips_and_is_absent_before_it_is_written() {
        let dir = tempfile::tempdir().unwrap();
        let files = TerminalFiles::at(dir.path());
        assert_eq!(files.read_pin().unwrap(), None);
        let pin = pin_of(7);
        files.write_pin(&pin).unwrap();
        assert_eq!(files.read_pin().unwrap(), Some(pin));
    }

    #[test]
    fn a_malformed_pin_file_is_an_error_not_an_absent_pin() {
        let dir = tempfile::tempdir().unwrap();
        let files = TerminalFiles::at(dir.path());
        std::fs::write(dir.path().join("node.pin"), "ABCD-1234\n").unwrap();
        assert!(files.read_pin().is_err());
        assert!(files.write_pin("not hex").is_err());
    }
}
