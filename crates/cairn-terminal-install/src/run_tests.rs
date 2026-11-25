//! The installer end to end, on temp dirs: a tiny `#!/bin/sh` script stands in for the window
//! program (it answers the two print flags), and the status file is read back with the very
//! parser the window uses (`InstallerStatus::from_json`).
use super::*;
use crate::fixtures::*;
use cairn_release::*;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::Duration;

/// A terminal: `<dir>/incoming`, `<dir>/state` (the window's `--release-dir` layout), the
/// installed program at `<dir>/bin/cairn-window` (release 1), and the channel's genesis
/// trust file (v1) at `<dir>/channel-root.cbor`.
struct Rig {
    _dir: tempfile::TempDir,
    cfg: Config,
}

fn terminal() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    for sub in ["incoming", "state", "bin"] {
        std::fs::create_dir(root.join(sub)).unwrap();
    }
    let program = root.join("bin/cairn-window");
    write_program(&program, &script(GEN, 1));
    std::fs::write(root.join("channel-root.cbor"), encode(&chain().v1)).unwrap();
    let cfg = Config {
        staging: root.join("incoming"),
        state: root.join("state"),
        program,
        run_as: None,
        genesis_root: root.join("channel-root.cbor"),
        this: this(),
        check_timeout: Duration::from_secs(5),
    };
    Rig { _dir: dir, cfg }
}

fn write_program(path: &Path, bytes: &[u8]) {
    std::fs::write(path, bytes).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// Stage files as the window does: meta, artifact, then `complete`.
fn stage(rig: &Rig, meta: &ReleaseMeta, artifact: &[u8], complete: &str) {
    let incoming = &rig.cfg.staging;
    std::fs::write(incoming.join(staging::META), encode(meta)).unwrap();
    std::fs::write(incoming.join(staging::ARTIFACT), artifact).unwrap();
    std::fs::write(incoming.join(staging::COMPLETE), complete).unwrap();
}

/// A release `version` of `program`, signed by v1's release key, staged as is.
fn stage_release(rig: &Rig, version: u64, program: &[u8]) {
    let h = history(&[&chain().v1]);
    let b = release(
        &h,
        version,
        vec![(window_for(&this()), program.to_vec())],
        &r1(),
    );
    stage(rig, &b.meta, &b.artifacts[0], &version.to_string());
}

fn status(rig: &Rig) -> InstallerStatus {
    let bytes = std::fs::read(rig.cfg.state.join("status.json")).unwrap();
    InstallerStatus::from_json(&bytes).expect("the window can read it")
}

fn staging_is_empty(rig: &Rig) -> bool {
    std::fs::read_dir(&rig.cfg.staging)
        .unwrap()
        .next()
        .is_none()
}

/// The status a refusal writes; also checks staging was cleared (every outcome clears it).
fn refused(rig: &Rig, refusal: &Refusal, installed: u64, release: Option<u64>) -> InstallerStatus {
    assert!(staging_is_empty(rig), "staging is cleared on every outcome");
    InstallerStatus {
        state: "refused".into(),
        installed_release: installed,
        pinned_root_version: 1,
        sentence: Some(refusal.sentence(Who::Terminal)),
        refused_release: release,
    }
}

/// The program is still release 1 and nothing was put beside it.
fn untouched(rig: &Rig) {
    let p = &rig.cfg.program;
    assert_eq!(std::fs::read(p).unwrap(), script(GEN, 1));
    for suffix in [".new", ".prev", ".new.tmp"] {
        let beside = cairn_keystore::fsio::sibling_with_suffix(p, suffix);
        assert!(!beside.exists(), "{beside:?}");
    }
}

#[test]
fn a_verified_release_replaces_the_program_and_keeps_prev() {
    let rig = terminal();
    stage_release(&rig, 7, &script(GEN, 7));
    assert!(install_staged(&rig.cfg));
    let p = &rig.cfg.program;
    let prev = cairn_keystore::fsio::sibling_with_suffix(p, ".prev");
    assert_eq!(std::fs::read(p).unwrap(), script(GEN, 7));
    assert_eq!(std::fs::read(&prev).unwrap(), script(GEN, 1));
    assert_eq!(
        std::fs::metadata(p).unwrap().permissions().mode() & 0o777,
        0o755
    );
    assert!(!cairn_keystore::fsio::sibling_with_suffix(p, ".new").exists());
    assert!(staging_is_empty(&rig));
    let state = &rig.cfg.state;
    assert_eq!(
        std::fs::read_to_string(state.join("installed_release")).unwrap(),
        "7"
    );
    let roots: Vec<SignedRoot> = decode(&std::fs::read(state.join("roots.cbor")).unwrap()).unwrap();
    assert_eq!(roots, vec![chain().v1]);
    // The next release moves 7 to .prev.
    stage_release(&rig, 8, &script(GEN, 8));
    assert!(install_staged(&rig.cfg));
    assert_eq!(std::fs::read(p).unwrap(), script(GEN, 8));
    assert_eq!(std::fs::read(&prev).unwrap(), script(GEN, 7));
}

#[test]
fn a_first_install_needs_no_program_in_place() {
    let rig = terminal();
    std::fs::remove_file(&rig.cfg.program).unwrap();
    stage_release(&rig, 7, &script(GEN, 7));
    assert!(install_staged(&rig.cfg));
    assert_eq!(std::fs::read(&rig.cfg.program).unwrap(), script(GEN, 7));
}

#[test]
fn a_program_that_will_not_start_is_never_swapped_in() {
    let hangs = b"#!/bin/sh\nexec sleep 30\n".to_vec();
    for program in [
        script(GEN, 6),                   // signed as 7, says 6
        script("85 00", 7),               // another generation
        b"#!/bin/sh\nexit 1\n".to_vec(),  // fails
        b"not a program at all".to_vec(), // cannot be run
        hangs,                            // never answers
    ] {
        let mut rig = terminal();
        rig.cfg.check_timeout = Duration::from_millis(300);
        stage_release(&rig, 7, &program);
        assert!(!install_staged(&rig.cfg));
        untouched(&rig);
        let want = refused(&rig, &Refusal::WontStart, 0, None); // R23: may be a slow start
        assert_eq!(status(&rig), want, "{}", String::from_utf8_lossy(&program));
        assert!(!rig.cfg.state.join("installed_release").exists());
    }
}

#[test]
fn a_half_staged_folder_is_cleared() {
    let h = history(&[&chain().v1]);
    let b = release(&h, 7, vec![(window_for(&this()), script(GEN, 7))], &r1());
    let damaged = Refusal::Malformed(String::new());
    // No artifact; a garbled `complete`; a `complete` naming another release than the
    // manifest; meta that is not a release; an artifact that is not the signed one.
    type Mess = fn(&Path);
    let cases: [(Mess, Refusal); 5] = [
        (
            |d| std::fs::remove_file(d.join(staging::ARTIFACT)).unwrap(),
            damaged.clone(),
        ),
        (
            |d| std::fs::write(d.join(staging::COMPLETE), "seven").unwrap(),
            damaged.clone(),
        ),
        (
            |d| std::fs::write(d.join(staging::COMPLETE), "8").unwrap(),
            damaged.clone(),
        ),
        (
            |d| std::fs::write(d.join(staging::META), b"junk").unwrap(),
            damaged.clone(),
        ),
        (
            |d| std::fs::write(d.join(staging::ARTIFACT), script(GEN, 9)).unwrap(),
            Refusal::ArtifactMismatch { index: 0 },
        ),
    ];
    for (mess, refusal) in cases {
        let rig = terminal();
        stage(&rig, &b.meta, &b.artifacts[0], "7");
        std::fs::create_dir(rig.cfg.staging.join("left-over")).unwrap();
        mess(&rig.cfg.staging);
        assert!(!install_staged(&rig.cfg));
        untouched(&rig);
        // Transit damage: the window may fetch it again (`refused_release` None).
        assert_eq!(status(&rig), refused(&rig, &refusal, 0, None));
    }
}

#[test]
fn a_folder_still_being_written_is_left_alone() {
    // No `complete` yet: the window may be mid-download, so nothing is touched, and a
    // terminal that has no status yet gets an idle one (the window fetches only once it exists).
    let rig = terminal();
    std::fs::write(rig.cfg.staging.join(staging::META), b"half").unwrap();
    assert!(install_staged(&rig.cfg));
    assert!(rig.cfg.staging.join(staging::META).exists());
    untouched(&rig);
    let idle = InstallerStatus {
        state: "idle".into(),
        installed_release: 0,
        pinned_root_version: 1,
        sentence: None,
        refused_release: None,
    };
    assert_eq!(status(&rig), idle);
}

#[test]
fn a_lower_release_is_not_newer() {
    let rig = terminal();
    std::fs::write(rig.cfg.state.join("installed_release"), "7").unwrap();
    stage_release(&rig, 5, &script(GEN, 5));
    assert!(!install_staged(&rig.cfg));
    untouched(&rig);
    let r = Refusal::NotNewer {
        installed: 7,
        offered: 5,
    };
    assert_eq!(status(&rig), refused(&rig, &r, 7, Some(5)));
}

#[test]
fn a_terminal_pinned_two_rotations_back_installs() {
    let rig = terminal();
    let c = chain();
    std::fs::remove_file(&rig.cfg.genesis_root).unwrap(); // the stored pin is what counts
    std::fs::write(
        rig.cfg.state.join("roots.cbor"),
        encode(&vec![c.v1.clone()]),
    )
    .unwrap();
    let full = history(&[&c.v1, &c.v2, &c.v3]);
    let mut b = release(&full, 7, vec![(window_for(&this()), script(GEN, 7))], &r2());
    b.meta.root_chain = vec![c.v2.clone(), c.v3.clone()]; // `cairn_release_roots(1)`
    stage(&rig, &b.meta, &b.artifacts[0], "7");
    assert!(install_staged(&rig.cfg));
    let roots: Vec<SignedRoot> =
        decode(&std::fs::read(rig.cfg.state.join("roots.cbor")).unwrap()).unwrap();
    assert_eq!(roots, vec![c.v1, c.v2, c.v3]);
    assert_eq!(status(&rig).pinned_root_version, 3);
}

#[test]
fn status_json_matches_the_windows_reader() {
    let rig = terminal();
    stage_release(&rig, 7, &script(GEN, 7));
    assert!(install_staged(&rig.cfg));
    let path = rig.cfg.state.join("status.json");
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o644
    );
    let installed = InstallerStatus {
        state: "installed".into(),
        installed_release: 7,
        pinned_root_version: 1,
        sentence: None,
        refused_release: None,
    };
    assert_eq!(status(&rig), installed);
}

#[test]
fn a_symlink_or_a_fifo_is_never_read() {
    // A link to a file holding exactly the signed bytes is still refused: the installer reads
    // only what is in the folder itself. A FIFO must not hang the root-owned installer.
    let rig = terminal();
    stage_release(&rig, 7, &script(GEN, 7));
    let real = rig.cfg.state.join("elsewhere");
    std::fs::rename(rig.cfg.staging.join(staging::ARTIFACT), &real).unwrap();
    std::os::unix::fs::symlink(&real, rig.cfg.staging.join(staging::ARTIFACT)).unwrap();
    assert!(!install_staged(&rig.cfg));
    untouched(&rig);
    assert!(real.exists(), "a link's target is never removed");
    assert_eq!(
        status(&rig).sentence,
        Some(Refusal::Malformed(String::new()).sentence(Who::Terminal))
    );

    let rig = terminal();
    stage_release(&rig, 7, &script(GEN, 7));
    let complete = rig.cfg.staging.join(staging::COMPLETE);
    std::fs::remove_file(&complete).unwrap();
    let c = std::ffi::CString::new(complete.as_os_str().as_encoded_bytes()).unwrap();
    // SAFETY: `c` is a valid NUL-terminated path for the duration of the call.
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o644) }, 0);
    assert!(!install_staged(&rig.cfg));
    untouched(&rig);
    assert!(staging_is_empty(&rig));
}

#[test]
fn a_fork_freezes_the_terminal_until_a_human_clears_it() {
    let rig = terminal();
    let c = chain();
    let h = history(&[&c.v1]);
    let mut b = release(&h, 7, vec![(window_for(&this()), script(GEN, 7))], &r1());
    b.meta.root_chain = vec![c.v2.clone(), c.rival_v2.clone()];
    stage(&rig, &b.meta, &b.artifacts[0], "7");
    assert!(!install_staged(&rig.cfg));
    let fork = Refusal::Fork { version: 2 };
    assert_eq!(status(&rig), refused(&rig, &fork, 0, None));
    assert!(rig.cfg.state.join("incident").exists());
    // A good release is refused while the incident stands...
    stage_release(&rig, 8, &script(GEN, 8));
    assert!(!install_staged(&rig.cfg));
    untouched(&rig);
    assert_eq!(status(&rig), refused(&rig, &fork, 0, None));
    // ...and installs once a human removes it.
    std::fs::remove_file(rig.cfg.state.join("incident")).unwrap();
    stage_release(&rig, 8, &script(GEN, 8));
    assert!(install_staged(&rig.cfg));
}

#[test]
fn no_channel_refuses_and_says_so() {
    let rig = terminal();
    std::fs::remove_file(&rig.cfg.genesis_root).unwrap();
    stage_release(&rig, 7, &script(GEN, 7));
    assert!(!install_staged(&rig.cfg));
    untouched(&rig);
    let mut want = refused(&rig, &Refusal::NoChannel, 0, None);
    want.pinned_root_version = 0;
    assert_eq!(status(&rig), want);
}

/// Ruling R22: the terminal's own faults say so, are never refused for good, and still write
/// the status first and clear staging (else `complete` would re-trigger the unit forever).
fn faulted(rig: &Rig) {
    untouched(rig);
    let mut want = refused(rig, &Refusal::TerminalFault, 0, None);
    want.pinned_root_version = status(rig).pinned_root_version;
    assert_eq!(status(rig), want);
}

#[test]
fn a_damaged_pin_refuses_rather_than_falling_back_to_genesis() {
    let rig = terminal();
    std::fs::write(rig.cfg.state.join("roots.cbor"), b"damaged").unwrap();
    stage_release(&rig, 7, &script(GEN, 7));
    assert!(!install_staged(&rig.cfg));
    faulted(&rig);
}

#[test]
fn a_write_that_fails_is_the_terminals_own_fault() {
    let rig = terminal();
    stage_release(&rig, 7, &script(GEN, 7));
    let bin = rig.cfg.program.parent().unwrap().to_path_buf();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o555)).unwrap();
    let installed = install_staged(&rig.cfg);
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(!installed);
    faulted(&rig);
}

#[test]
fn a_bad_run_as_user_is_the_terminals_own_fault() {
    // Not a user, or root: the new program must never be test-run as either.
    for user in ["no-such-cairn-user", "root"] {
        let mut rig = terminal();
        rig.cfg.run_as = Some(user.into());
        stage_release(&rig, 7, &script(GEN, 7));
        assert!(!install_staged(&rig.cfg), "{user}");
        faulted(&rig);
    }
}

/// Ruling R23: a release whose signatures do not verify cannot lock out the genuine release of
/// the same number; only a verdict on a verified release is for good.
#[test]
fn a_forged_release_never_locks_out_the_genuine_one() {
    let rig = terminal();
    let h = history(&[&chain().v1]);
    let forged = release(&h, 7, vec![(window_for(&this()), script(GEN, 7))], &r2());
    stage(&rig, &forged.meta, &forged.artifacts[0], "7");
    assert!(!install_staged(&rig.cfg));
    let r = Refusal::ReleaseSignaturesShort { have: 0, need: 1 };
    assert_eq!(status(&rig), refused(&rig, &r, 0, None));
    stage_release(&rig, 7, &script(GEN, 7));
    assert!(install_staged(&rig.cfg));
}

#[test]
fn a_frozen_terminal_says_fork_before_reading_anything() {
    let rig = terminal();
    std::fs::write(rig.cfg.state.join("incident"), "2").unwrap();
    stage_release(&rig, 7, &script(GEN, 7));
    std::fs::write(rig.cfg.staging.join(staging::META), b"junk").unwrap();
    assert!(!install_staged(&rig.cfg));
    untouched(&rig);
    let fork = Refusal::Fork { version: 2 };
    assert_eq!(status(&rig), refused(&rig, &fork, 0, None));
}
