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
        idx: 0,
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
        refused_release: None,
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
    assert_eq!(due_of(&s, &offers), Some(8));
    // An offer without a readable generation line is never fetched: it cannot be judged.
    let mut blank = offer(8, g);
    blank.generation_line = None;
    assert_eq!(due_of(&s, &[blank]), None);
    // No stamp: nothing can be judged to fit.
    assert_eq!(due(&s, &offers, running(0), None, false, &[]), None);
}

/// [`due`] against this build at release 0 and this build's stamp, nothing staged or given up.
fn due_of(s: &InstallerStatus, offers: &[Offer]) -> Option<u64> {
    due_with(s, offers, 0, false, &[])
}

/// [`due`] on this build's stamp, running `running`, with `staged` saying whether anything is
/// at `incoming/complete`.
fn due_with(
    s: &InstallerStatus,
    offers: &[Offer],
    running_release: u64,
    staged: bool,
    given_up: &[u64],
) -> Option<u64> {
    let stamp = our_stamp();
    due(
        s,
        offers,
        running(running_release),
        Some(&stamp),
        staged,
        given_up,
    )
    .map(|o| o.release_version)
}

#[test]
fn due_skips_a_release_already_installed() {
    let g = embedded_schema_version();
    assert_eq!(due_of(&status("installed", 6, None), &[offer(6, g)]), None);
    // Not newer than the program running now either (a terminal updated by hand).
    assert_eq!(
        due_with(&status("idle", 0, None), &[offer(6, g)], 6, false, &[]),
        None
    );
    assert_eq!(due_of(&status("idle", 0, None), &[offer(6, g)]), Some(6));
}

/// Rulings R17/R30: while anything is at `incoming/complete`, nothing is fetched, not even a
/// newer release: the installer may be reading those files, and a new fetch would truncate them
/// under it. That holds for a stale or unreadable `complete` too (one at or below what is
/// installed): it re-triggers the path unit, whose run clears `incoming/` on every outcome.
#[test]
fn an_untaken_release_waits() {
    let g = embedded_schema_version();
    let idle = status("idle", 5, None);
    assert_eq!(due_with(&idle, &[offer(6, g)], 0, true, &[]), None);
    assert_eq!(
        due_with(&idle, &[offer(7, g), offer(6, g)], 0, true, &[]),
        None
    );
    // Nothing staged: the newer offer is due.
    assert_eq!(due_with(&idle, &[offer(7, g)], 0, false, &[]), Some(7));
}

/// Ruling R30: any `complete` holds the window back, also one that is stale or no number (the
/// old rule waited only for a number newer than the installed release, and so fetched over it).
#[test]
fn a_stale_or_unreadable_complete_still_counts_as_staged() {
    let dir = scratch();
    std::fs::create_dir_all(dir.join("incoming")).unwrap();
    assert!(!is_staged(&dir));
    for junk in ["3", "not a number"] {
        std::fs::write(dir.join("incoming").join(COMPLETE), junk).unwrap();
        assert!(is_staged(&dir), "{junk}");
    }
}

/// Ruling R30: the window fetches the very entry the installer will take. A release may carry
/// two programs for this terminal; the installer takes the FIRST (lowest `idx`), so the window
/// stages that one, and a release whose first program does not fit is not fetched at all
/// (staging its second would end in a refusal for a file the signed list does not name there).
#[test]
fn due_picks_the_lowest_idx_of_the_newest_release() {
    let g = embedded_schema_version();
    let entry = |idx: u32, generation: i32| Offer {
        idx,
        digest: vec![idx as u8 + 1; 32],
        ..offer(7, generation)
    };
    let s = status("idle", 0, None);
    let stamp = our_stamp();
    let pick = |offers: &[Offer]| due(&s, offers, running(0), Some(&stamp), false, &[]);
    assert_eq!(pick(&[entry(0, g), entry(1, g)]).map(|o| o.idx), Some(0));
    assert_eq!(pick(&[entry(1, g), entry(0, g)]).map(|o| o.idx), Some(0));
    // The first program needs another schema: not the second instead, but the older release.
    let older = offer(6, g);
    let got = pick(&[entry(0, g + 1), entry(1, g), older.clone()]);
    assert_eq!(got, Some(older));
}

/// Ruling R18: a release the installer refused for good (`refused_release`) is never fetched
/// again; one refused as damaged in transit (no `refused_release`, `incoming/` cleared) is, on a
/// later poll; and a newer release after a refused one is due.
#[test]
fn a_refused_release_is_fetched_again_only_if_a_fetch_can_fix_it() {
    let g = embedded_schema_version();
    let mut for_good = status("refused", 4, Some("not signed by enough signers"));
    for_good.refused_release = Some(6);
    assert_eq!(due_of(&for_good, &[offer(6, g)]), None);
    assert_eq!(due_of(&for_good, &[offer(7, g), offer(6, g)]), Some(7));
    // A release refused for good does not hold back an older one the installer has not seen.
    assert_eq!(due_of(&for_good, &[offer(6, g), offer(5, g)]), Some(5));
    let damaged = status("refused", 4, Some("damaged; fetch it again"));
    assert_eq!(due_of(&damaged, &[offer(6, g)]), Some(6));
}

/// Ruling R19: after [`MAX_ATTEMPTS`] fetches of one release this session, failed or not, the
/// window gives that release up; a newer release is still due.
#[test]
fn fetches_stop_after_three() {
    let courier = Courier::new(None);
    for _ in 0..MAX_ATTEMPTS - 1 {
        courier.attempted(6);
    }
    assert!(courier.given_up().is_empty());
    courier.attempted(6);
    assert_eq!(courier.given_up(), vec![6]);
    let g = embedded_schema_version();
    let idle = status("idle", 0, None);
    assert_eq!(due_with(&idle, &[offer(6, g)], 0, false, &[6]), None);
    assert_eq!(
        due_with(&idle, &[offer(7, g), offer(6, g)], 0, false, &[6]),
        Some(7)
    );
}

/// One poll after another against `offers`, the installer refusing every staged release with
/// `refuse` (its `refused_release` for that release); how many times each release was fetched.
fn fetched_while_refused(
    offers: &[Offer],
    refuse: impl Fn(u64) -> Option<u64>,
) -> std::collections::BTreeMap<u64, u32> {
    let (courier, mut fetched) = (Courier::new(None), std::collections::BTreeMap::new());
    let mut s = status("idle", 0, None);
    for _poll in 0..50 {
        let Some(r) = due_with(&s, offers, 0, false, &courier.given_up()) else {
            break;
        };
        // What `fetch` does first; the installer then refuses it and clears `incoming/`.
        courier.attempted(r);
        *fetched.entry(r).or_insert(0) += 1;
        s = status("refused", 0, Some("refused"));
        s.refused_release = refuse(r);
    }
    fetched
}

/// Ruling R19 (a): a node whose stored file is damaged the same way every time stages fine and
/// is refused as damaged (`refused_release` none) each time: fetched three times, then not again.
#[test]
fn deterministic_damage_stops_after_three_fetches() {
    let fetched = fetched_while_refused(&[offer(6, embedded_schema_version())], |_| None);
    assert_eq!(
        fetched.into_iter().collect::<Vec<_>>(),
        vec![(6, MAX_ATTEMPTS)]
    );
}

/// Ruling R19 (b): two releases refused for good (before the genesis ceremony, say) would
/// alternate, since `refused_release` holds one: each is fetched three times, then neither again.
#[test]
fn two_refused_releases_do_not_alternate_forever() {
    let g = embedded_schema_version();
    let fetched = fetched_while_refused(&[offer(6, g), offer(5, g)], Some);
    let expected = vec![(5, MAX_ATTEMPTS), (6, MAX_ATTEMPTS)];
    assert_eq!(fetched.into_iter().collect::<Vec<_>>(), expected);
}

/// A genuine one-off transit error still gets its re-fetch: one fetch, refused as damaged, and
/// the release is due again.
#[test]
fn a_one_off_transit_error_is_fetched_again() {
    let courier = Courier::new(None);
    courier.attempted(6);
    let damaged = status("refused", 0, Some("damaged; fetch it again"));
    let offers = [offer(6, embedded_schema_version())];
    assert_eq!(
        due_with(&damaged, &offers, 0, false, &courier.given_up()),
        Some(6)
    );
}

/// No Cairn program is over 64 MiB (the installer refuses one as `TooLarge`): never fetched.
#[test]
fn an_oversized_offer_is_never_due() {
    let g = embedded_schema_version();
    let mut big = offer(6, g);
    big.byte_len = MAX_ARTIFACT + 1;
    assert_eq!(due_of(&status("idle", 0, None), &[big.clone()]), None);
    big.byte_len = MAX_ARTIFACT;
    assert_eq!(due_of(&status("idle", 0, None), &[big]), Some(6));
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
