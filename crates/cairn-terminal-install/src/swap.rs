//! Putting a verified program in place, effectfully and never leaving the program path
//! missing or half-written.
//!
//! Flow ([`install`]):
//! 1. write the VERIFIED in-memory bytes to `<program>.new` (atomic write, mode 0755, fsynced).
//!    The staged file itself is never run or copied: it belongs to an unprivileged user who
//!    could change it after the check;
//! 2. run `<program>.new` unprivileged with both print flags ([`ask`]) and require what the
//!    signed manifest names (`decide::answers_match`); otherwise remove `.new` and stop;
//! 3. hard-link the current program to `<program>.prev` (an old `.prev` removed first; no
//!    current program is fine on a first install), so the previous one stays at hand;
//! 4. `rename` `.new` over the program (atomic: a reader sees the old or the new, never none)
//!    and fsync the directory so the rename survives a power cut.

use crate::decide::answers_match;
use cairn_keystore::fsio::{atomic_write, sibling_with_suffix};
use cairn_release::Artifact;
use std::io::{self, Read};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The flags every window answers before anything else (cairn-gui-tauri `update.rs`). They
/// are a contract across releases and never change.
pub const PRINT_FLAG: &str = "--print-schema-generation";
pub const RELEASE_FLAG: &str = "--print-release-version";

/// The most of a program's answer that is read: one line is far shorter.
const MAX_ANSWER: u64 = 4096;

/// Install `bytes` (already checked against artifact `artifact` of release `release`) at
/// `program`. `Ok(false)`: the new program did not start as signed, nothing changed.
pub fn install(
    program: &Path,
    bytes: &[u8],
    artifact: &Artifact,
    release: u64,
    run_as: Option<(u32, u32)>,
    timeout: Duration,
) -> io::Result<bool> {
    let new = sibling_with_suffix(program, ".new");
    atomic_write(&new, bytes, Some(0o755))?;
    let generation = ask(&new, PRINT_FLAG, run_as, timeout);
    let answered = ask(&new, RELEASE_FLAG, run_as, timeout);
    if !answers_match(
        artifact,
        release,
        generation.as_deref(),
        answered.as_deref(),
    ) {
        std::fs::remove_file(&new)?;
        return Ok(false);
    }
    let prev = sibling_with_suffix(program, ".prev");
    let swapped = keep_previous(program, &prev).and_then(|()| std::fs::rename(&new, program));
    if let Err(e) = swapped {
        let _ = std::fs::remove_file(&new);
        return Err(e);
    }
    let dir = program.parent().filter(|p| !p.as_os_str().is_empty());
    std::fs::File::open(dir.unwrap_or(Path::new(".")))?.sync_all()?;
    Ok(true)
}

/// Hard-link `program` to `prev`, replacing an older `prev`. No `program` yet is fine.
fn keep_previous(program: &Path, prev: &Path) -> io::Result<()> {
    match std::fs::remove_file(prev) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    match std::fs::hard_link(program, prev) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// Run `program flag` as `run_as` (uid, gid; `None` keeps this process's user, for tests),
/// with an empty environment, stdin and stderr null, and return its stdout if it exited
/// successfully within `timeout`. A program still running at the deadline is killed. A
/// helper it left behind holding stdout open cannot hang the installer either: stdout is
/// read on a thread, and waited for only until the deadline.
pub fn ask(
    program: &Path,
    flag: &str,
    run_as: Option<(u32, u32)>,
    timeout: Duration,
) -> Option<String> {
    use std::os::unix::process::CommandExt;
    let deadline = Instant::now() + timeout;
    let mut command = Command::new(program);
    command
        .arg(flag)
        .env_clear()
        .current_dir("/")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some((uid, gid)) = run_as {
        command.uid(uid).gid(gid);
    }
    let mut child = spawn(&mut command)?;
    let stdout = child.stdout.take()?;
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut out = Vec::new();
        let read = stdout.take(MAX_ANSWER).read_to_end(&mut out);
        let _ = send.send(read.map(|_| out));
    });
    // shortcut: polls every 20 ms (std has no wait-with-timeout); fine for one short child.
    let status = loop {
        if let Some(status) = child.try_wait().ok()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let left = deadline.saturating_duration_since(Instant::now());
    let out = receive.recv_timeout(left).ok()?.ok()?;
    status.success().then(|| String::from_utf8(out).ok())?
}

/// Spawn, retrying briefly on "text file busy": a just-written program can be busy for a
/// moment when another thread of this process forks meanwhile (Linux; the tests run in
/// parallel threads). The installer itself is single-threaded.
fn spawn(command: &mut Command) -> Option<std::process::Child> {
    for _ in 0..5 {
        match command.spawn() {
            Err(e) if e.kind() == io::ErrorKind::ExecutableFileBusy => {
                std::thread::sleep(Duration::from_millis(50))
            }
            other => return other.ok(),
        }
    }
    None
}

/// The (uid, gid) of user `name` through the system's user database (`getpwnam_r`, so NSS
/// users resolve too), or `None` if there is no such user.
pub fn user_ids(name: &str) -> Option<(u32, u32)> {
    let name = std::ffi::CString::new(name).ok()?;
    let mut buf = vec![0 as libc::c_char; 16 * 1024];
    // SAFETY: `passwd` is plain C data, so all-zero is a valid value to be overwritten.
    let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
    let mut found: *mut libc::passwd = std::ptr::null_mut();
    // SAFETY: every pointer is valid for the call: `name` is NUL-terminated, `buf` is
    // `buf.len()` bytes, and `pwd`/`found` are live locals. The strings `pwd` points into live
    // in `buf`, but only the two integers are read, and before `buf` is dropped.
    let rc = unsafe {
        libc::getpwnam_r(
            name.as_ptr(),
            &mut pwd,
            buf.as_mut_ptr(),
            buf.len(),
            &mut found,
        )
    };
    (rc == 0 && !found.is_null()).then_some((pwd.pw_uid, pwd.pw_gid))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_user_is_looked_up_by_name() {
        assert_eq!(user_ids("root"), Some((0, 0)));
        assert_eq!(user_ids("no-such-cairn-user"), None);
        assert_eq!(user_ids("ro\0ot"), None);
    }
}
