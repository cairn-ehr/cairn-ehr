//! The window as courier (#909 slice C2, design §4): it carries a steward-signed release from its
//! node to this terminal's staging folder, and says what the terminal's installer made of it.
//!
//! # Why the window, and why it trusts nothing
//!
//! A terminal's installer runs as root and has no database login; the window already has the node's
//! one connection. So on its 60 s update poll (`crate::update::update_poll`) the window asks the
//! node, through three read-only doors (db/086), whether a newer release fits this machine, and
//! downloads it into `<release dir>/incoming/`. It verifies NOTHING: the installer re-verifies
//! every byte against its own pin (`cairn-terminal-install`), so a window or node that lies can at
//! worst stage something the installer refuses.
//!
//! # Flow
//!
//! 1. [`ready`]: a release dir was given (`--release-dir`), the installer has written
//!    `state/status.json` (else there is no installer here), and `/etc/os-release` names this
//!    machine's platform. Fixture mode never gets past this.
//! 2. [`offers`] asks the node (`cairn_release_offer`); [`due`] picks, of each release's first
//!    program for this terminal (lowest `idx`, the one the installer takes), the newest that is
//!    newer than what is installed and running, fits the node's stamp, is at most 64 MiB, was
//!    not refused for good, and was not given up after [`MAX_ATTEMPTS`] fetches this session;
//!    and nothing at all while anything is at `incoming/complete`.
//! 3. [`fetch`] (spawned, at most one at a time: [`Courier::claim`]) writes `meta.cbor`, then
//!    `artifact.bin` by 256 KiB chunks, each under its own lock of the shared connection so a
//!    clinical press waits behind at most one chunk, then `complete` LAST: the installer acts on
//!    `complete` alone.
//! 4. [`installer_view`] adds what the installer said to the update line: an installed release
//!    newer than this program is an update `waiting` (a hint only: the restart still asks the
//!    installed program first, C1's rule), and a refusal shows its sentence.
//!
//! # The contract with the installer (`cairn-terminal-install`)
//!
//! The installer writes `status.json` FIRST, then clears `incoming/`, on EVERY outcome, installed
//! or refused (the other order would let a poll see no `complete` beside a stale status). So
//! while ANYTHING is at `complete` the window fetches nothing at all: the installer may be
//! reading those files (ruling R17), and even a stale or unreadable `complete` re-triggers the
//! path unit, whose run clears it (R30). A refusal a fresh fetch cannot fix names its release in
//! `refused_release`, which the window then never fetches again; one damaged in transit leaves
//! it `None`, and the window fetches that release again on a later poll.
use crate::state::AppState;
use crate::update::UpdateView;
use cairn_node::db::SchemaStamp;
use cairn_node::window_floor::{install_fit, InstallFit, InstalledBuild};
use cairn_release::{encode, ReleaseMeta, SignedRoot};
pub use cairn_release::{InstallerState, InstallerStatus};
// The staging folder's names, the 64 MiB limit and this machine's target are the installer's
// contract too, so they are defined once, in cairn-release (ruling R20).
pub use cairn_release::staging::{this_target, Target, ARTIFACT, COMPLETE, MAX_ARTIFACT, META};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{atomic::AtomicBool, atomic::Ordering, Mutex};

/// The chunk door's cap (db/086): the longest a fetch holds the window's one connection at once.
pub const CHUNK: u64 = 262_144;

/// Said while this window fetches a release (the plan's copy, verbatim).
pub const DOWNLOADING: &str = "This terminal is fetching its update from the node.";

/// Said when the installer refused the staged release: its own sentence (already sanitised by
/// the verifier; the webview sets it as text, never HTML).
pub fn refused_note(sentence: &str) -> String {
    format!("This terminal did not install the update: {sentence}")
}

/// One `window` program the node offers for this target (a row of `cairn_release_offer`).
#[derive(Debug, Clone, PartialEq)]
pub struct Offer {
    pub release_version: u64,
    /// The program's place in the manifest's artifact list: the installer takes a release's
    /// FIRST program for this terminal, so the window fetches only the lowest `idx` (R30).
    pub idx: u32,
    /// The signed manifest's bytes and its signatures, staged as they came.
    pub manifest: Vec<u8>,
    pub release_signatures: Vec<Vec<u8>>,
    /// The program's content address: what the chunk door is asked for.
    pub digest: Vec<u8>,
    pub byte_len: u64,
    /// The program's `--print-schema-generation` line, as the manifest names it.
    pub generation_line: Option<String>,
}

/// Fetches of one release per session, failed OR staged, before the window gives it up (R19): a
/// release that keeps failing, or keeps being refused once staged, must not re-download forever.
pub const MAX_ATTEMPTS: u32 = 3;

/// The window's fetch bookkeeping, in `AppState`: where to stage, whether a fetch is running,
/// and how many times each release was fetched this session.
#[derive(Default)]
pub struct Courier {
    /// `--release-dir`; `None` never fetches.
    pub dir: Option<PathBuf>,
    fetching: AtomicBool,
    attempts: Mutex<std::collections::BTreeMap<u64, u32>>,
}

/// A running fetch's hold on [`Courier`]; dropping it lets the next fetch start.
pub struct Claim<'a>(&'a Courier);

impl Drop for Claim<'_> {
    fn drop(&mut self) {
        self.0.fetching.store(false, Ordering::SeqCst);
    }
}

impl Courier {
    pub fn new(dir: Option<PathBuf>) -> Self {
        Self {
            dir,
            ..Default::default()
        }
    }

    /// Start a fetch, unless one is running (`None`): two would write one folder at once.
    pub fn claim(&self) -> Option<Claim<'_>> {
        let free = self
            .fetching
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok();
        free.then_some(Claim(self))
    }

    pub fn is_fetching(&self) -> bool {
        self.fetching.load(Ordering::SeqCst)
    }

    /// Count one fetch of `release`, however it ends.
    pub fn attempted(&self, release: u64) {
        let mut attempts = self.attempts.lock().unwrap_or_else(|p| p.into_inner());
        *attempts.entry(release).or_default() += 1;
    }

    /// The releases fetched [`MAX_ATTEMPTS`] times this session.
    pub fn given_up(&self) -> Vec<u64> {
        let attempts = self.attempts.lock().unwrap_or_else(|p| p.into_inner());
        let spent = attempts.iter().filter(|(_, n)| **n >= MAX_ATTEMPTS);
        spent.map(|(release, _)| *release).collect()
    }
}

/// What a poll needs before it may ask the node anything.
#[derive(Debug)]
pub struct Ready {
    pub dir: PathBuf,
    pub status: InstallerStatus,
    pub target: Target,
}

/// May this window fetch at all? Not in fixture mode, not without a release dir, not without
/// the installer's status file (no installer here, or one that wrote half a file), and not
/// without a target. Reads only `status.json`.
pub fn ready(mock: bool, dir: Option<&Path>, os_release: Option<&str>) -> Option<Ready> {
    if mock {
        return None;
    }
    let dir = dir?;
    Some(Ready {
        status: read_status(dir)?,
        target: this_target(os_release)?,
        dir: dir.to_path_buf(),
    })
}

/// `<dir>/state/status.json`, if it is there and whole.
pub fn read_status(dir: &Path) -> Option<InstallerStatus> {
    InstallerStatus::from_json(&std::fs::read(dir.join("state").join("status.json")).ok()?)
}

/// Is anything at `<dir>/incoming/complete`? A link, a FIFO or a file that is no number counts
/// too, as it does for the installer: the path unit runs on it, and that run clears it (R30).
pub fn is_staged(dir: &Path) -> bool {
    let found = std::fs::symlink_metadata(dir.join("incoming").join(COMPLETE));
    !matches!(found, Err(e) if e.kind() == std::io::ErrorKind::NotFound)
}

/// The release number staged in `<dir>/incoming/`, if any (the tests' view of what a fetch
/// staged; the poll asks only [`is_staged`]).
#[cfg(test)]
pub fn staged_release(dir: &Path) -> Option<u64> {
    let text = std::fs::read_to_string(dir.join("incoming").join(COMPLETE)).ok()?;
    text.trim().parse().ok()
}

/// The offer to fetch now, if any. **Pure.** None at all while anything is `staged` at
/// `incoming/complete` ([`is_staged`]): the installer has not taken it yet, and a fetch would
/// truncate files it may be reading. Otherwise the newest offer that is its release's first
/// program for this terminal (lowest `idx`, the one the installer takes), is newer than the
/// installed and `running` releases, is at most [`MAX_ARTIFACT`] bytes, is neither the
/// installer's `refused_release` nor `given_up` this session, and whose program this node's
/// `stamp` would serve ([`install_fit`] says `Fits`): one it would refuse is never fetched. An
/// offer whose generation line cannot be read cannot be judged, so it is skipped.
pub fn due(
    status: &InstallerStatus,
    offers: &[Offer],
    running: InstalledBuild,
    stamp: Option<&SchemaStamp>,
    staged: bool,
    given_up: &[u64],
) -> Option<Offer> {
    if staged {
        return None;
    }
    let floor = status.installed_release.max(running.release);
    let wanted = |o: &&Offer| {
        o.release_version > floor
            && o.byte_len <= MAX_ARTIFACT
            && status.refused_release != Some(o.release_version)
            && !given_up.contains(&o.release_version)
    };
    let fits = |offer: &Offer| {
        let line = offer.generation_line.as_deref();
        let Some((generation, hash)) = line.and_then(crate::update::parse_generation_line) else {
            return false;
        };
        let build = InstalledBuild {
            generation,
            hash,
            release: offer.release_version,
        };
        install_fit(Some(build), running, stamp) == InstallFit::Fits
    };
    // The installer takes each release's FIRST program for this terminal (lowest `idx`), so only
    // that entry may be fetched; a release whose first program is unwanted is skipped whole.
    let first = |o: &&Offer| {
        let same = offers
            .iter()
            .filter(|p| p.release_version == o.release_version);
        same.map(|p| p.idx).min() == Some(o.idx)
    };
    offers
        .iter()
        .filter(|o| first(o) && wanted(o) && fits(o))
        .max_by_key(|o| o.release_version)
        .cloned()
}

/// The update line, told what the installer said. **Pure.** An installed release newer than
/// this program turns `current` into `waiting` (below the floor stays below); the note is
/// [`DOWNLOADING`] while a fetch runs, else a refusal's sentence, else nothing.
pub fn installer_view(
    mut view: UpdateView,
    status: Option<&InstallerStatus>,
    running: u64,
    fetching: bool,
) -> UpdateView {
    let Some(status) = status else {
        return view;
    };
    if view.kind == "current" && status.installed_release > running {
        view.kind = "waiting";
    }
    view.note = match (&status.sentence, status.state) {
        _ if fetching => Some(DOWNLOADING.to_string()),
        (Some(sentence), InstallerState::Refused) => Some(refused_note(sentence)),
        _ => None,
    };
    view
}

/// Everything [`fetch`] needs, decided by a poll and carried into the spawned task.
#[derive(Debug)]
pub struct Job {
    pub offer: Offer,
    pub dir: PathBuf,
    pub pinned_root: u64,
}

/// The poll's courier half (`crate::update::update_poll`): add the installer's word to `view`,
/// and decide whether a fetch is due. A fetch is decided only on a stamp read just now (`fresh`)
/// and never while one runs.
pub async fn after_poll(
    state: &AppState,
    view: UpdateView,
    fresh: Option<&Option<SchemaStamp>>,
) -> (UpdateView, Option<Job>) {
    let os_release = std::fs::read_to_string("/etc/os-release").ok();
    let dir = state.release.dir.as_deref();
    let Some(ready) = ready(state.is_mock(), dir, os_release.as_deref()) else {
        // A release folder means a terminal (the kiosk unit always passes one), so not being
        // ready there is a broken installer contract: say it once in the journal, not silently.
        static SAID: std::sync::Once = std::sync::Once::new();
        if dir.is_some() && !state.is_mock() {
            SAID.call_once(|| {
                eprintln!(
                    "this window will fetch no update: no readable installer status under \
                     --release-dir, or no ID and VERSION_ID in /etc/os-release"
                )
            });
        }
        return (view, None);
    };
    let job = match fresh {
        Some(stamp) if !state.release.is_fetching() => {
            let offers = offers(state, &ready.target).await.unwrap_or_default();
            let staged = is_staged(&ready.dir);
            let (running, given_up) = (crate::update::running_build(), state.release.given_up());
            let due = due(
                &ready.status,
                &offers,
                running,
                stamp.as_ref(),
                staged,
                &given_up,
            );
            due.map(|offer| Job {
                offer,
                dir: ready.dir.clone(),
                pinned_root: ready.status.pinned_root_version,
            })
        }
        _ => None,
    };
    let fetching = state.release.is_fetching() || job.is_some();
    let running = crate::update::release_version();
    (
        installer_view(view, Some(&ready.status), running, fetching),
        job,
    )
}

/// The node's offers for `target`, newest first. `None` when the node did not answer (said on
/// stderr); fetching then simply waits for the next poll.
pub async fn offers(state: &AppState, target: &Target) -> Option<Vec<Offer>> {
    let conn = match state.db.as_ref()?.lock().await {
        Ok(conn) => conn,
        Err(lost) => {
            eprintln!("the update check could not reach the node for releases: {lost}");
            return None;
        }
    };
    let rows = conn
        .query(
            "SELECT release_version, manifest, release_signatures, digest, byte_len, \
                    generation_line, idx FROM cairn_release_offer($1, $2, $3)",
            &[&target.os, &target.arch, &target.platform],
        )
        .await;
    let rows = match rows {
        Ok(rows) => rows,
        Err(e) => {
            eprintln!(
                "the update check could not ask for releases: {:#}",
                conn.explain(e.into())
            );
            return None;
        }
    };
    // Negative numbers cannot be stored by the intake; one would read as "nothing offered".
    let rows = rows.iter().filter_map(|r| {
        Some(Offer {
            release_version: u64::try_from(r.get::<_, i64>(0)).ok()?,
            manifest: r.get(1),
            release_signatures: r.get(2),
            digest: r.get(3),
            byte_len: u64::try_from(r.get::<_, i64>(4)).ok()?,
            generation_line: r.get(5),
            idx: u32::try_from(r.get::<_, i32>(6)).ok()?,
        })
    });
    Some(rows.collect())
}

/// Fetch `offer` into `<dir>/incoming/` (see the module doc's step 3), unless a fetch is
/// already running. Every fetch counts ([`Courier::attempted`], at most [`MAX_ATTEMPTS`]); a
/// failure is said on stderr and leaves no `complete`, and the next poll starts again.
pub async fn fetch(state: &AppState, offer: Offer, dir: &Path, pinned_root: u64) {
    let Some(_claim) = state.release.claim() else {
        return;
    };
    state.release.attempted(offer.release_version);
    if let Err(e) = fetch_into(state, &offer, dir, pinned_root).await {
        eprintln!("the update could not be fetched from the node: {e:#}");
    }
}

/// [`fetch`]'s work: the roots after the pin, the meta, then the artifact by chunks.
async fn fetch_into(
    state: &AppState,
    offer: &Offer,
    dir: &Path,
    pinned_root: u64,
) -> anyhow::Result<()> {
    let meta = ReleaseMeta {
        root_chain: roots_after(state, pinned_root).await?,
        manifest: offer.manifest.clone(),
        release_signatures: offer.release_signatures.clone(),
        // Ruling R2: no transparency log yet.
        inclusion_proof: None,
    };
    let read = |offset, len| chunk(state, &offer.digest, offset, len);
    stage(
        &dir.join("incoming"),
        offer.release_version,
        &encode(&meta),
        offer.byte_len,
        read,
    )
    .await
}

/// The trust roots after `pinned` (`cairn_release_roots`), ascending: the chain the installer
/// climbs from its pin.
async fn roots_after(state: &AppState, pinned: u64) -> anyhow::Result<Vec<SignedRoot>> {
    let db = state
        .db
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("no node"))?;
    let conn = db.lock().await?;
    let after = i64::try_from(pinned)?;
    let rows = conn
        .query(
            "SELECT payload, signatures FROM cairn_release_roots($1)",
            &[&after],
        )
        .await
        .map_err(|e| conn.explain(e.into()))?;
    let roots = rows.iter().map(|r| SignedRoot {
        payload: r.get(0),
        signatures: r.get(1),
    });
    Ok(roots.collect())
}

/// One chunk of the program, under its own lock of the window's one connection: the guard is
/// dropped when this returns, so a clinical press waits behind at most this one read.
async fn chunk(state: &AppState, digest: &[u8], offset: u64, len: i32) -> anyhow::Result<Vec<u8>> {
    let db = state
        .db
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("no node"))?;
    let conn = db.lock().await?;
    let row = conn
        .query_one(
            "SELECT cairn_release_chunk($1, $2, $3)",
            &[&digest, &i64::try_from(offset)?, &len],
        )
        .await
        .map_err(|e| conn.explain(e.into()))?;
    Ok(row.get(0))
}

/// Write a release into `incoming`: `meta`, then `byte_len` bytes read by `read(offset, len)` in
/// slices of at most [`CHUNK`], then `complete` holding `release`, LAST and by rename, so the
/// installer never sees `complete` beside a partial file. A stale `complete` is removed FIRST: it
/// would otherwise pair an old release's number with the new files. A slice that is empty or
/// longer than asked (the node's file differs from its offer) fails the stage.
pub async fn stage<F, Fut>(
    incoming: &Path,
    release: u64,
    meta: &[u8],
    byte_len: u64,
    mut read: F,
) -> anyhow::Result<()>
where
    F: FnMut(u64, i32) -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<Vec<u8>>>,
{
    // shortcut: std::fs on the async runtime (this crate's tokio has no `fs`); a 256 KiB write
    // per chunk is brief, and the fetch runs on its own spawned task.
    std::fs::create_dir_all(incoming)?;
    match std::fs::remove_file(incoming.join(COMPLETE)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
        _ => {}
    }
    write_synced(&incoming.join(META), meta)?;
    let mut out = std::fs::File::create(incoming.join(ARTIFACT))?;
    let mut offset = 0;
    while offset < byte_len {
        let want = CHUNK.min(byte_len - offset);
        let bytes = read(offset, want as i32).await?;
        if bytes.is_empty() || bytes.len() as u64 > want {
            anyhow::bail!("the node's program is not the length its offer names");
        }
        out.write_all(&bytes)?;
        offset += bytes.len() as u64;
    }
    out.sync_all()?;
    let pending = incoming.join("complete.partial");
    write_synced(&pending, release.to_string().as_bytes())?;
    // No fsync of the directory after the rename: a power cut may lose `complete`, which is
    // safe (no `complete`, nothing installs, and the next poll fetches again).
    std::fs::rename(pending, incoming.join(COMPLETE))?;
    Ok(())
}

/// Write `bytes` to `path` and flush them to disk.
fn write_synced(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = std::fs::File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

#[cfg(test)]
#[path = "release_fetch_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "release_fetch_db_tests.rs"]
mod db_tests;
