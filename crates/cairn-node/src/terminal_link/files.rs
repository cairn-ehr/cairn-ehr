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
    /// The files in `dir`. Nothing is touched until a method is called; each makes `dir` (0700)
    /// if it is missing.
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
        self.private_dir()?;
        let path = self.key_path();
        if !path.exists() {
            let (sk, _kid) = crate::keystore::generate_plaintext(&path)
                .with_context(|| format!("creating the terminal key {}", path.display()))?;
            return Ok(sk);
        }
        self.load_key()
    }

    /// The terminal's EXISTING key, never made: for a caller that only reports (the window's
    /// not-admitted launch text names this terminal's fingerprint) and must not mint a key, or
    /// make the directory, as a side effect. Refused like
    /// [`load_or_create_key`](Self::load_or_create_key) when the directory or the key can be
    /// reached by other users; an error when either is missing.
    pub fn load_key(&self) -> anyhow::Result<SigningKey> {
        refuse_mode(&self.dir, 0o022, DIR_OTHERS_CAN_WRITE)?;
        let path = self.key_path();
        refuse_mode(
            &path,
            0o077,
            "can be read by other users; anyone who read it can present this terminal. Ask the \
             node's operator to revoke this terminal, then delete the file so a new key is made",
        )?;
        crate::keystore::load(&path, None)
            .with_context(|| format!("loading the terminal key {}", path.display()))
    }

    /// The node's pinned key, or `None` before the first-launch ceremony has written one.
    /// A pin file that is not 64 hex digits is an error (never "no pin"): treating it as absent
    /// would let the next launch re-pin from a typed fingerprint without anyone noticing.
    pub fn read_pin(&self) -> anyhow::Result<Option<String>> {
        self.private_dir()?;
        let path = self.pin_path();
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        refuse_mode(
            &path,
            0o022,
            "can be written by other users, who could put their own node's key in it. Ask the \
             node's operator before deleting it to pin the node again",
        )?;
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
        self.private_dir()?;
        let path = self.pin_path();
        let line = format!("{}\n", hex.to_ascii_lowercase());
        crate::fsio::atomic_write(&path, line.as_bytes(), Some(0o600))
            .with_context(|| format!("writing {}", path.display()))
    }
}

impl TerminalFiles {
    /// Make the terminal directory if it is missing (mode 0700), and refuse one other users can
    /// write: they could replace `node.pin` with their own node's key, or `terminal.key` with
    /// theirs. Called before every read and write, so no file is trusted from such a directory.
    fn private_dir(&self) -> anyhow::Result<()> {
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder
            .create(&self.dir)
            .with_context(|| format!("creating the terminal directory {}", self.dir.display()))?;
        refuse_mode(&self.dir, 0o022, DIR_OTHERS_CAN_WRITE)
    }
}

/// Why a terminal directory other users can write is refused (`private_dir`, `load_key`).
const DIR_OTHERS_CAN_WRITE: &str =
    "can be written by other users, who could replace this terminal's node pin or key. Make it \
     private to this user (chmod 700), then start again";

/// Refuse `path` if its mode has any bit of `mask` set (group/other access), saying `problem`.
/// Refused, not repaired: someone widened it, and only a person can judge what that exposed.
/// Unix only: elsewhere POSIX modes do not apply.
#[cfg(unix)]
fn refuse_mode(path: &Path, mask: u32, problem: &str) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(path)
        .with_context(|| format!("reading {}", path.display()))?
        .permissions()
        .mode();
    if mode & mask != 0 {
        bail!(
            "{} {problem} (its mode is {:o}).",
            path.display(),
            mode & 0o777
        );
    }
    Ok(())
}

#[cfg(not(unix))]
fn refuse_mode(_path: &Path, _mask: u32, _problem: &str) -> anyhow::Result<()> {
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

    /// A reader that only reports (the window's launch refusal text) must never mint a key: a
    /// missing key is an error and leaves the directory as it was; an existing one loads back.
    #[test]
    fn loading_without_creating_never_makes_a_key() {
        let dir = tempfile::tempdir().unwrap();
        let files = TerminalFiles::at(dir.path());
        assert!(files.load_key().is_err());
        assert!(!dir.path().join("terminal.key").exists());
        let made = files.load_or_create_key().unwrap();
        assert_eq!(files.load_key().unwrap().to_bytes(), made.to_bytes());
        let absent = TerminalFiles::at(&dir.path().join("never-made"));
        assert!(absent.load_key().is_err());
        assert!(!dir.path().join("never-made").exists());
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

    #[cfg(unix)]
    #[test]
    fn a_terminal_directory_others_can_write_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let files = TerminalFiles::at(dir.path());
        files.write_pin(&pin_of(3)).unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o777)).unwrap();
        for err in [
            files.read_pin().unwrap_err().to_string(),
            files.load_or_create_key().unwrap_err().to_string(),
            files.write_pin(&pin_of(4)).unwrap_err().to_string(),
        ] {
            assert!(err.contains("other users"), "{err}");
        }
    }

    /// Group and others may READ the directory (0755, a common umask's default): only write
    /// access lets them swap a file in (final review T6).
    #[cfg(unix)]
    #[test]
    fn an_existing_0755_terminal_directory_is_accepted() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        let files = TerminalFiles::at(dir.path());
        files.write_pin(&pin_of(6)).unwrap();
        assert_eq!(files.read_pin().unwrap(), Some(pin_of(6)));
        files.load_or_create_key().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_pin_file_others_can_write_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let files = TerminalFiles::at(dir.path());
        files.write_pin(&pin_of(5)).unwrap();
        let pin = dir.path().join("node.pin");
        std::fs::set_permissions(&pin, std::fs::Permissions::from_mode(0o666)).unwrap();
        let err = files.read_pin().unwrap_err().to_string();
        assert!(err.contains("other users"), "{err}");
    }
}
