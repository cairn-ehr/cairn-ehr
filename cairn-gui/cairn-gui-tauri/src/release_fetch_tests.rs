//! Tests for `release_fetch.rs` (#909 slice C2): which offer is due, when the window fetches
//! nothing, the staging folder's order, and what the update line says about the installer.
use super::*;
use crate::update::{generation_line, update_poll, update_view};
use cairn_node::db::{embedded_schema_hash, embedded_schema_version, SchemaStamp};
use cairn_node::window_floor::WindowVerdict;
use std::cell::Cell;

/// This build as the installer would see it, at release `release`.
fn running(release: u64) -> InstalledBuild {
    InstalledBuild {
        generation: embedded_schema_version(),
        hash: embedded_schema_hash(),
        release,
    }
}

/// A stamp at this build's own generation, stamped by any cairn-node build: this window is
/// current against it.
fn our_stamp() -> SchemaStamp {
    SchemaStamp {
        version: embedded_schema_version(),
        loader_build: "cairn-node 0.0.0".to_string(),
        schema_hash: Some(embedded_schema_hash().to_vec()),
        window_floor: None,
    }
}

/// An offer for release `release` whose program has schema `generation` (this build's hash).
fn offer(release: u64, generation: i32) -> Offer {
    Offer {
        release_version: release,
        manifest: vec![release as u8; 3],
        release_signatures: vec![vec![1, 2]],
        digest: vec![release as u8; 32],
        byte_len: 10,
        generation_line: Some(generation_line(generation, embedded_schema_hash())),
    }
}

/// What the installer last wrote.
fn status(state: &str, installed: u64, sentence: Option<&str>) -> InstallerStatus {
    InstallerStatus {
        state: state.into(),
        installed_release: installed,
        pinned_root_version: 1,
        sentence: sentence.map(str::to_string),
    }
}

/// A fresh, empty directory for one test.
fn scratch() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("cairn-release-fetch-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A release directory whose installer has written `status`.
fn release_dir_with(status: &InstallerStatus) -> PathBuf {
    let dir = scratch();
    std::fs::create_dir_all(dir.join("state")).unwrap();
    std::fs::write(dir.join("state/status.json"), status.to_json()).unwrap();
    dir
}

const OS_RELEASE: &str =
    "PRETTY_NAME=\"Debian GNU/Linux 12 (bookworm)\"\nID=debian\nVERSION_ID=\"12\"\n";

#[test]
fn due_picks_the_newest_fitting_newer_offer() {
    let g = embedded_schema_version();
    // Release 9 needs a newer schema than this database serves: it would be refused, so the
    // newest that FITS is 8. Release 3 is older than what is installed.
    let offers = [offer(9, g + 1), offer(8, g), offer(5, g), offer(3, g)];
    let s = status("installed", 4, None);
    let due = due(&s, &offers, running(0), Some(&our_stamp()), None);
    assert_eq!(due.map(|o| o.release_version), Some(8));
    // An offer without a readable generation line is never fetched: it cannot be judged.
    let mut blank = offer(8, g);
    blank.generation_line = None;
    assert_eq!(due_of(&s, &[blank]), None);
    // No stamp: nothing can be judged to fit.
    assert_eq!(super::due(&s, &offers, running(0), None, None), None);
}

/// [`due`] against this build at release 0 and this build's stamp, nothing staged.
fn due_of(s: &InstallerStatus, offers: &[Offer]) -> Option<u64> {
    due(s, offers, running(0), Some(&our_stamp()), None).map(|o| o.release_version)
}

#[test]
fn due_skips_a_release_already_installed() {
    let g = embedded_schema_version();
    assert_eq!(due_of(&status("installed", 6, None), &[offer(6, g)]), None);
    // Not newer than the program running now either (a terminal updated by hand).
    assert_eq!(
        due(
            &status("idle", 0, None),
            &[offer(6, g)],
            running(6),
            Some(&our_stamp()),
            None
        ),
        None
    );
    // Already staged, this session or still waiting in `incoming/`: not fetched twice.
    let staged = due(
        &status("idle", 0, None),
        &[offer(6, g)],
        running(0),
        Some(&our_stamp()),
        Some(6),
    );
    assert_eq!(staged, None);
    assert_eq!(due_of(&status("idle", 0, None), &[offer(6, g)]), Some(6));
}

#[test]
fn no_status_file_fetches_nothing() {
    let dir = scratch();
    assert!(
        ready(false, Some(&dir), Some(OS_RELEASE)).is_none(),
        "no file"
    );
    std::fs::create_dir_all(dir.join("state")).unwrap();
    std::fs::write(dir.join("state/status.json"), b"{\"state\":").unwrap();
    assert!(
        ready(false, Some(&dir), Some(OS_RELEASE)).is_none(),
        "half a file"
    );
    let dir = release_dir_with(&status("idle", 0, None));
    let ready = ready(false, Some(&dir), Some(OS_RELEASE)).expect("a status and a target");
    assert_eq!(ready.status, status("idle", 0, None));
    assert_eq!(ready.target.platform, "debian-12");
    // No /etc/os-release: no target, so nothing to ask the node for.
    assert!(super::ready(false, Some(&dir), None).is_none());
}

#[test]
fn no_release_dir_fetches_nothing() {
    assert!(ready(false, None, Some(OS_RELEASE)).is_none());
}

/// Review Focus 5 of C1, carried: `--mock` never polls, never restarts, and never fetches.
#[tokio::test]
async fn mock_never_fetches() {
    let dir = release_dir_with(&status("idle", 0, None));
    assert!(ready(true, Some(&dir), Some(OS_RELEASE)).is_none());
    let mut state = AppState::mock(None);
    state.release = Courier::new(Some(dir.clone()));
    let (view, job) = update_poll(&state).await;
    assert!(job.is_none());
    assert_eq!((view.kind, view.note), ("current", None));
    assert!(!dir.join("incoming").exists());
}

/// `/etc/os-release`'s two keys, quoted or not; anything without both is no platform.
#[test]
fn the_platform_is_id_and_version_id() {
    assert_eq!(platform_of(OS_RELEASE).as_deref(), Some("debian-12"));
    assert_eq!(
        platform_of("ID='raspbian'\nVERSION_ID=11").as_deref(),
        Some("raspbian-11")
    );
    assert_eq!(
        platform_of("VERSION_ID=\"12\"\nID=debian").as_deref(),
        Some("debian-12")
    );
    // A `#` comment and an `ID_LIKE` line are not `ID`.
    assert_eq!(
        platform_of("# ID=fake\nID_LIKE=debian\nID=ubuntu\nVERSION_ID=\"24.04\"").as_deref(),
        Some("ubuntu-24.04")
    );
    for text in ["", "ID=debian", "VERSION_ID=12", "ID=\nVERSION_ID=12"] {
        assert_eq!(platform_of(text), None, "{text:?}");
    }
    let target = this_target(Some(OS_RELEASE)).unwrap();
    assert_eq!(
        (target.os.as_str(), target.arch.as_str()),
        (std::env::consts::OS, std::env::consts::ARCH)
    );
}

/// The artifact arrives by chunks of at most [`CHUNK`], from offset 0 to its length.
#[tokio::test]
async fn a_staged_release_is_meta_artifact_then_complete() {
    let incoming = scratch().join("incoming");
    let artifact: Vec<u8> = (0..(CHUNK as usize * 2 + 7)).map(|i| i as u8).collect();
    let asked = std::cell::RefCell::new(vec![]);
    let read = |offset: u64, len: i32| {
        asked.borrow_mut().push((offset, len));
        let at = offset as usize;
        std::future::ready(Ok(artifact[at..at + len as usize].to_vec()))
    };
    stage(&incoming, 7, b"meta", artifact.len() as u64, read)
        .await
        .unwrap();
    assert_eq!(std::fs::read(incoming.join(META)).unwrap(), b"meta");
    assert_eq!(std::fs::read(incoming.join(ARTIFACT)).unwrap(), artifact);
    assert_eq!(
        std::fs::read_to_string(incoming.join(COMPLETE)).unwrap(),
        "7"
    );
    assert_eq!(staged_release(incoming.parent().unwrap()), Some(7));
    let c = CHUNK as i32;
    assert_eq!(*asked.borrow(), vec![(0, c), (CHUNK, c), (CHUNK * 2, 7)]);
}

/// The installer acts on `complete` alone, so it is written last: a fetch cut off after the
/// artifact (or in it) leaves no `complete`, and a stale one from before is gone first.
#[tokio::test]
async fn complete_is_written_last() {
    let incoming = scratch().join("incoming");
    std::fs::create_dir_all(&incoming).unwrap();
    std::fs::write(incoming.join(COMPLETE), "3").unwrap();
    let calls = Cell::new(0);
    let cut_after_two = |_offset: u64, len: i32| {
        calls.set(calls.get() + 1);
        std::future::ready(match calls.get() {
            1 | 2 => Ok(vec![0; len as usize]),
            _ => Err(anyhow::anyhow!("the node went away")),
        })
    };
    let err = stage(&incoming, 4, b"meta", CHUNK * 3, cut_after_two).await;
    assert!(err.is_err());
    assert!(incoming.join(META).exists() && incoming.join(ARTIFACT).exists());
    assert!(
        !incoming.join(COMPLETE).exists(),
        "a cut fetch is not complete"
    );
    assert_eq!(staged_release(incoming.parent().unwrap()), None);

    // Every chunk arrived, but the file ended early (the node answered '' before its length):
    // no `complete` either, so the installer never sees a short file.
    let short = |offset: u64, _len: i32| {
        std::future::ready(Ok(if offset == 0 { vec![1; 5] } else { vec![] }))
    };
    assert!(stage(&incoming, 4, b"meta", 10, short).await.is_err());
    assert!(!incoming.join(COMPLETE).exists());
}

#[test]
fn a_newer_installed_release_shows_waiting() {
    let current = || update_view(&WindowVerdict::Current);
    let seen = installer_view(current(), Some(&status("installed", 5, None)), 4, false);
    assert_eq!((seen.kind, seen.note), ("waiting", None));
    // The same release as this program, or none: nothing is waiting.
    assert_eq!(
        installer_view(current(), Some(&status("installed", 4, None)), 4, false).kind,
        "current"
    );
    assert_eq!(installer_view(current(), None, 4, false).kind, "current");
    // Below the floor stays below the floor: the alert outranks the calm line.
    let below = update_view(&WindowVerdict::BelowFloor { reason: None });
    assert_eq!(
        installer_view(below, Some(&status("installed", 5, None)), 4, false).kind,
        "below_floor"
    );
}

/// The update line's note: fetching, or the installer's refusal in its own words.
#[test]
fn the_note_says_fetching_or_the_refusal() {
    let current = || update_view(&WindowVerdict::Current);
    let fetching = installer_view(current(), Some(&status("idle", 0, None)), 0, true);
    assert_eq!(fetching.note.as_deref(), Some(DOWNLOADING));
    assert_eq!(
        DOWNLOADING,
        "This terminal is fetching its update from the node."
    );
    let said = "This update file is damaged or is not a Cairn update; fetch it again.";
    let refused = installer_view(current(), Some(&status("refused", 0, Some(said))), 0, false);
    assert_eq!(
        refused.note.as_deref(),
        Some("This terminal did not install the update: This update file is damaged or is not a Cairn update; fetch it again.")
    );
    // A new fetch after a refusal says it is fetching.
    let again = installer_view(current(), Some(&status("refused", 0, Some(said))), 0, true);
    assert_eq!(again.note.as_deref(), Some(DOWNLOADING));
    // Installed or idle says nothing.
    assert_eq!(
        installer_view(current(), Some(&status("installed", 0, None)), 0, false).note,
        None
    );
}

/// At most one fetch at a time: a second claim while the first is held is refused.
#[test]
fn one_fetch_at_a_time() {
    let courier = Courier::new(None);
    let first = courier.claim().expect("the first claim");
    assert!(courier.is_fetching());
    assert!(courier.claim().is_none());
    drop(first);
    assert!(!courier.is_fetching());
    assert!(courier.claim().is_some());
}
