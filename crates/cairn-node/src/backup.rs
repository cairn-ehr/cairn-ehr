//! ADR-0026 slice B — backup-as-cold-peer: the DB/IO orchestration of an export plus
//! backup-health surfacing. The PURE medium container format (framing, the CAIRNB2
//! self-marker, signature verification) lives in [`crate::medium`]; this module is the thin
//! DB-touching layer on top.
//!
//! WHY THIS EXISTS: the spec designed *intentional* key-death in detail (crypto-shred) but
//! left *accidental* data-death — a node's disk simply dying — undesigned. For a genuinely-solo
//! clinic (no parent to re-provision from) replication provides zero durability, so a backup is
//! the only safety net. ADR-0026's insight: a backup is just another replication peer — the
//! medium holds a NORMAL Cairn event set and restore is set-union apply through the existing
//! verify-on-apply path. This module drives the capture of BOTH signed event planes
//! (`node_event` and, since #500 slice 2c, `event_log` with its per-record custody) onto a
//! local append-only medium, reads a medium of either revision back for `restore` and
//! `verify-backup`, and surfaces backup health (point 7: a node running without a net must
//! say so). The capture LOOP itself — which events reach the medium and which are skipped —
//! lives in [`crate::capture`]; this module owns the durability and the health record around
//! it.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

// The medium format is defined once in `crate::medium`. Re-export the surface other modules
// and tests already reach for via `backup::…`, so the format move stays source-compatible.
pub use crate::medium::{
    parse_medium, verify_event, verify_events, verify_medium_bytes, BackupError, SelfMarker,
    VerifyReport,
};

use crate::capture;
use crate::medium::{MediumImage, Plane};

// ---------------------------------------------------------------------------
// Reading a medium of EITHER revision (Erratum E2, #500 slice 2c design doc §4).
//
// WHY THIS EXISTS. `restore` and `verify-backup` both used to read a medium through the
// LEGACY parser (`parse_container`, which refuses CAIRNB3 outright). `backup_to` now WRITES
// CAIRNB3, at which point that refusal would have stopped being a safety net and become the
// defect: an operator's nightly backup verifying RED and their only restore path refusing to
// read it — the working half of disaster recovery, broken by the slice that is fixing the
// broken half. This section landed one commit BEFORE the writer switched, so no commit in
// this branch's history is ever unable to read a medium it can write.
// ---------------------------------------------------------------------------

/// Which events the RESTORE path actually applies, for either medium revision — the ONE
/// place that answers that question, so `restore` and `verify-backup` (both callers) can
/// never silently drift onto two different answers.
///
/// - **Legacy (CAIRNB1/CAIRNB2):** every event in the container, unchanged. A legacy medium
///   predates the plane split entirely — every event on it IS the federation plane — so this
///   is exactly what `parse_container` always handed back. Media already in the field are
///   unaffected, forever.
/// - **CAIRNB3:** only the records carried by `Plane::Node` segments, in file order. A
///   CAIRNB3 medium written by this build carries the CLINICAL plane too, but restoring it
///   is slice 2d's job — returning it here would silently let 2c be read as having closed
///   #500's restore half, which it has deliberately not (design doc §8).
///
/// **Design choice, made explicit because the alternative is tempting and wrong: every
/// Node-plane segment is returned, never only the prefix `chain::chain_report` could
/// verify.** A legacy medium has no chain concept at all — `parse_container` hands back
/// every event it holds, gated only by the FLAT per-event signature check both callers run
/// immediately after this function returns (`verify_events`, unchanged). Filtering a CAIRNB3
/// medium down to `ChainReport::verified_through` would make a federation event's fate
/// depend on whether some OTHER, unrelated segment earlier in the medium's single global
/// chain happens to verify — including a CLINICAL segment, a plane this call site does not
/// even restore. That would silently drop a federation record an operator can see plainly
/// in the file, over a fault in content nobody here is trying to recover: the opposite
/// failure from the one "verification is the boundary of trust" guards against elsewhere in
/// this slice (which is a WRITER deciding what it may safely call "already captured", not a
/// reader deciding what to surface from bytes already on disk). The per-record signature
/// check downstream still refuses the whole restore/verify if any returned record's own
/// signature fails, so this choice costs nothing on that front. What it does NOT catch:
/// segment-level chain tampering that leaves every individual record signature intact (a
/// spliced or reordered segment) — that is `cairn_medium::health::assess`'s job, and it is
/// not wired into either call site in this slice.
///
/// Returns `Result` for symmetry with this crate's other readers (`parse_medium`,
/// `verify_medium_bytes`) and so a future revision that CAN fail need not change this
/// function's callers. Given an already-parsed `MediumImage`, today it cannot: both match
/// arms only rearrange bytes already validated by `parse_any`. Review finding: harmless,
/// noted rather than "fixed" — narrowing the signature would break the interface this
/// task was specified against for no behavioural gain (#500 slice 2c review).
pub fn node_plane_events(image: &MediumImage) -> Result<Vec<Vec<u8>>, BackupError> {
    Ok(match image {
        MediumImage::Legacy(container) => container.events.clone(),
        MediumImage::V3(medium) => medium
            .segments
            .iter()
            .filter(|segment| segment.plane == Plane::Node)
            .flat_map(|segment| segment.records.iter().map(|r| r.signed_bytes.clone()))
            .collect(),
    })
}

/// How many records a medium carries in each plane — the shared arithmetic behind every
/// operator-facing scope message in `verify-backup`/`restore` (#500 slice 2c review,
/// Important 3 & 4). One function so the three call sites (the empty-medium check, the
/// unknown-plane warning, and the "clinical events were not restored" note) can never
/// silently disagree about how many records a medium actually holds.
///
/// A legacy medium predates the plane split entirely: every event on it IS the federation
/// plane, so it reports as all-`node`, zero `clinical`, zero `unknown`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PlaneCounts {
    pub node: usize,
    pub clinical: usize,
    /// Records under a plane tag this build does not recognise — written by a NEWER
    /// Cairn. Deliberately NOT folded into `clinical`/`node`, and deliberately not
    /// silently dropped: a caller that ignores this can report a medium "OK" while an
    /// entire plane it cannot read sits unmentioned — precisely the
    /// `BackupError::UnsupportedByThisBuild` case the error taxonomy exists to name, one
    /// layer up (`MediumV3::segments` keeps an unknown-plane segment in the list for
    /// exactly this reason — see its doc).
    pub unknown: usize,
}

/// Count every record on `image`, grouped by plane. Pure; no verification performed (an
/// unsigned or tampered segment's records are counted exactly like any other — this
/// answers "how much is here", not "how much can be trusted").
pub fn plane_counts(image: &MediumImage) -> PlaneCounts {
    match image {
        MediumImage::Legacy(container) => PlaneCounts {
            node: container.events.len(),
            clinical: 0,
            unknown: 0,
        },
        MediumImage::V3(medium) => {
            let mut counts = PlaneCounts::default();
            for segment in &medium.segments {
                let n = segment.records.len();
                match segment.plane {
                    Plane::Node => counts.node += n,
                    Plane::Clinical => counts.clinical += n,
                    Plane::Unknown(_) => counts.unknown += n,
                }
            }
            counts
        }
    }
}

/// If `image` is a TORN CAIRNB3 medium (its last section is short), a human-readable
/// description of what that does and does not tell us — `None` when the medium is
/// complete.
///
/// WHY THIS EXISTS (#500 slice 2c review). `parse_any` reports a torn tail via
/// `MediumV3::truncated_tail` rather than an `Err`, because everything before the tear is
/// genuinely intact and a torn medium is not damage (see that field's doc). A caller that
/// only checks the returned `Result` — which is exactly what both `verify-backup` and
/// `restore` did before this existed — sees `Ok` and reports the medium sound, missing its
/// tail with no warning at all. A legacy medium can never reach this state silently:
/// `parse_container` already fails a truncated frame with `Damaged`, AT PARSE TIME, before
/// returning at all.
///
/// **Principle 4, read carefully (#500 slice 2c review round 3): `truncated_tail` is an
/// OBSERVATION, not a diagnosis, and the text below is a BRACKET, never a point claim.**
/// A short last section is consistent with an interrupted capture-loop append — in which
/// case exactly ONE increment is missing, because the watermark never advances past an
/// unverifiable tail — but it is INDISTINGUISHABLE, at this layer, from a partial or
/// truncated COPY of an otherwise-complete medium (a failed `cp`/`dd`, a dying USB drive,
/// a cut-off network transfer), where arbitrarily many increments — up to the entire tail
/// — could be missing. An earlier version of this text asserted "at most one increment"
/// unconditionally; that was true of the first cause and false, possibly badly so, of the
/// second, and round 2 made this text load-bearing for a DEFAULT that proceeds without
/// refusing (see `restore`'s caller, below) — an imprecise near-truth stated as a bracket
/// beats a precise-sounding untruth stated as a point (principle 4). The one thing this
/// layer CAN say without guessing: comparing this medium's size/event counts against the
/// SOURCE node's `backup-status.json`, if it is still reachable, tells the two apart.
///
/// **Named a "notice", not a "refusal" — read this before wiring in a third caller.** The
/// two existing callers use this fact for OPPOSITE purposes (#500 slice 2c review round
/// 2): `verify-backup` is the cron health check — its job is to say "this is not a
/// complete backup", so it turns this into a hard bail with its own remedy text ("run
/// `backup` again", which IS actionable there). `restore` must NOT refuse: whatever the
/// true extent of the loss, a torn tail can only ever cost what comes AFTER the intact
/// prefix — which is all `parse_any` even keeps in `MediumV3::segments`, the torn remnant
/// is discarded before this function ever sees it — never what is IN it, so restoring the
/// prefix is strictly better than refusing outright (which would cost the whole thing).
/// That reasoning holds regardless of how much is missing, which is exactly why it does
/// not need the retracted "at most one" certainty to justify proceeding by default.
///
/// A legacy medium therefore always returns `None` here (its own parse already refused a
/// torn frame, so a `Container` this function sees is never torn), and a complete CAIRNB3
/// medium returns `None` too.
pub fn torn_tail_notice(image: &MediumImage, path: &std::path::Path) -> Option<String> {
    match image {
        MediumImage::Legacy(_) => None,
        MediumImage::V3(medium) if medium.truncated_tail => Some(format!(
            "{} was cut short after {} byte(s). Everything before that point is intact and \
             fully verified. What is missing after it is NOT determinable from the bytes \
             alone: this is consistent with an interrupted backup append (in which case \
             exactly ONE increment is missing — a capture never advances past an \
             unverifiable tail) but is indistinguishable, at this layer, from a \
             partial/truncated COPY of a complete medium (a failed `cp`/`dd`, a dying USB \
             drive, a cut-off network transfer) — where arbitrarily many increments, up to \
             the whole tail, could be missing. If the source node is still reachable, \
             compare this medium's size and event counts against its `backup-status.json` \
             to tell the two apart before assuming only one increment is gone.",
            path.display(),
            medium.complete_bytes
        )),
        MediumImage::V3(_) => None,
    }
}

/// Derive the self-marker `resolve_dead_node` should use, for either medium revision.
///
/// WHY A SHARED FUNCTION, NOT INLINE MATCH LOGIC (#500 slice 2c review round 2). This is
/// the exact construction `main.rs`'s restore arm needs — extracted so main.rs and its
/// tests call the SAME code, rather than a test replicating the logic beside it. A prior
/// version of this fix inlined the match directly in `main.rs` and tested a hand-copied
/// re-implementation in `tests/restore.rs`; that test could not have caught a regression
/// of `main.rs` itself back to `self_marker: None` (issue #53's exact footgun) — only
/// calling the real function can.
///
/// - **Legacy:** the container's own marker, unchanged.
/// - **CAIRNB3:** derived from `chain::self_id_from_chain` — the ATTESTED id (two binds
///   already checked there: the segment attestation verifies, and the named node has a
///   genesis on THIS medium signed by the same key), never the untrusted plaintext
///   `Segment::self_node_id_hex`. Wrapped as `SelfMarker::Unsigned`, not `Signed`:
///   `SelfMarker::Signed`'s verifier (`verify_self_attestation`) expects a CAIRNB2
///   whole-set-committing `node.self_attested` blob, a different wire shape from a
///   segment attestation, and would wrongly raise `InvalidSelfMarker` on a perfectly good
///   V3 medium. Wrapping it at all is what preserves `resolve_dead_node`'s
///   `confirm_explicit` cross-check — with a bare `None`, `resolve_without_marker` accepts
///   ANY `--superseded-node` present on the medium, silently reopening issue #53. `None`
///   only when no segment attestation on the medium binds to a genesis also on it (e.g. an
///   entirely unsigned capture); `resolve_dead_node`'s existing marker-less fallback then
///   applies (sole enroll, or an explicit, unchecked `--superseded-node`).
pub fn self_marker_for(image: &MediumImage) -> Option<SelfMarker> {
    match image {
        MediumImage::Legacy(container) => container.self_marker.clone(),
        MediumImage::V3(medium) => {
            let report = crate::medium::chain_report(medium);
            crate::medium::self_id_from_chain(medium, &report).map(SelfMarker::Unsigned)
        }
    }
}

// ---------------------------------------------------------------------------
// Backup health (node-local operational state — NOT a clinical event, never signed,
// never replicated). Lives in a local sidecar JSON, not the DB: see the slice-B design
// note (smaller safety-critical surface — no SECURITY DEFINER door — and it fails SAFE,
// degrading to "never / running without a net" when absent or unreadable).
// ---------------------------------------------------------------------------

/// A record of the last successful backup. Written only AFTER the medium is durable and
/// self-verified, so it can never over-claim a backup the node does not actually hold.
///
/// **v2 (#500 slice 2c Task 10) replaces the single `event_count` with per-plane SCOPE.**
/// v1 recorded one true count of what the medium held, with nothing to say that what it
/// held was the federation plane alone and no clinical record at all — a count without a
/// scope is the honest-surface half of a dishonest composite, and it is exactly how #500
/// stayed invisible for months even though `status`/`describe_health` never lied about the
/// number itself.
///
/// `#[serde(default)]` on every field new in v2: a v1 sidecar on disk (written by
/// yesterday's binary) must still READ after an upgrade, with the missing fields becoming
/// `None`/0 rather than a parse failure. A parse failure here reads as "no backup ever
/// ran" — the reassuring-direction lie this project keeps hunting — so the fail-safe
/// direction is to under-claim scope, never to refuse the whole record over one absent field.
///
/// **What is deliberately NOT here yet.** `capture_plane`'s `PlaneCapture::unfilled_gaps`
/// and `probed_empty` (Task 7) have no operator surface in this struct. That is not an
/// oversight: `unfilled_gaps` is NOT normally empty on a federating node — routine
/// `ON CONFLICT` IDENTITY burns look identical to lost events — so a naive count surfaced
/// here would cry wolf on every healthy node. The durable record and the honest surface for
/// those two fields are [#549](https://github.com/cairn-ehr/cairn-ehr/issues/549), not this
/// task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupHealth {
    pub version: u8,
    /// Unix seconds at which the backup completed (operational wall-clock, not the HLC).
    pub last_backup_unix: i64,
    /// Where the medium was written (for the operator to locate it).
    pub medium_path: String,
    /// Size of the medium image in bytes.
    pub medium_bytes: u64,
    /// How many federation-plane (`node_event`) records the medium holds. Renamed from v1's
    /// `event_count` — kept alone, a count says nothing about SCOPE (see the struct doc).
    #[serde(default)]
    pub node_events: u64,
    /// How many clinical-plane (`event_log`) records the medium holds. `0` on a v1 sidecar
    /// or a build that has not yet started capturing the clinical plane — the honest "zero
    /// known", never a parse failure standing in for it.
    #[serde(default)]
    pub clinical_events: u64,
    /// The medium's newest clinical `seq` (`cairn_medium::watermark` over `Plane::Clinical`,
    /// derived from verified segments only — see `PlaneCapture::watermark`). `None` means no
    /// verified clinical segment exists yet, which is NOT the same claim as `Some(0)` would
    /// be (principle 4: absence is the honest answer, zero is a claim).
    #[serde(default)]
    pub clinical_watermark: Option<i64>,
    /// `max(event_log.seq)` at the moment the local-state export (`CAIRNL1`, the artifact
    /// carrying this node's custody key off the machine) was last WRITTEN — deliberately NOT
    /// at the moment this sidecar itself was written, and deliberately NOT advanced by a
    /// skipped export (see [`export_coverage_after`]). `None` means no export has ever
    /// succeeded here. A coverage figure no export actually achieved would be worse than
    /// none: it would make `verify-backup`'s staleness check pass over a kit that cannot
    /// restore.
    #[serde(default)]
    pub export_covers_seq: Option<i64>,
}

/// What the export attempt this backup run did — the only input `export_coverage_after`
/// needs, because that is the only distinction that changes whether coverage may advance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportOutcome {
    /// Not attempted, or attempted and not durably written (no passphrase, an unusable
    /// escrow, a load failure — every one of `backup`'s deliberate warn-and-continue paths,
    /// see `backup_to`'s doc). Coverage must not move: nothing new actually landed.
    Skipped,
    /// The export was written AND verified (read-after-write, Task 12). Carries
    /// `max(event_log.seq)` as it stood at that moment.
    Written(i64),
}

/// PURE. Which `export_covers_seq` the next sidecar carries, given the value the sidecar
/// already had and what this run's export attempt did.
///
/// This is a one-line `match`, split out as its own function anyway because it is the ENTIRE
/// safety property that makes `export_covers_seq` trustworthy, and a property this narrow
/// is worth being able to test without a database, a medium, or even `backup_to` — see
/// `a_skipped_export_leaves_the_previous_coverage_untouched` in `tests/backup_health_v2.rs`.
/// Advancing on a `Skipped` outcome would let `verify-backup` compare the medium against a
/// coverage figure no export ever achieved — worse than no figure at all, because it would
/// make a stale, unrestorable kit report GREEN.
pub fn export_coverage_after(previous: Option<i64>, outcome: ExportOutcome) -> Option<i64> {
    match outcome {
        ExportOutcome::Skipped => previous,
        ExportOutcome::Written(seq) => Some(seq),
    }
}

/// The sidecar path for backup health: a sibling of the key file named
/// `backup-status.json`. Node-local, discoverable from what `status` already has (the
/// key path). Pure.
pub fn health_path_for(key_path: &Path) -> PathBuf {
    key_path.with_file_name("backup-status.json")
}

/// Render a coarse "time ago" for a `status` line. Pure. Negative input (a clock that
/// went backwards between backup and read) is reported as "just now" rather than a
/// nonsense negative age — honest degradation, never a confusing display.
pub fn humanize_ago(secs: i64) -> String {
    if secs <= 0 {
        return "just now".to_string();
    }
    let s = secs as u64;
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m", s / 60)
    } else if s < 86_400 {
        format!("{}h", s / 3600)
    } else {
        format!("{}d", s / 86_400)
    }
}

/// The `status` backup-health line. Pure (time injected). Absent health → the honest
/// "running without a net" warning; present → freshness + per-plane SCOPE + size + location.
///
/// v1 printed one bare "N events" — precisely the composite #500 hid behind (see
/// `BackupHealth`'s doc). Naming the plane in the text itself, not just in a struct field
/// nobody reads directly, is the point: an operator staring at `cairn-node status` must see
/// "0 clinical events" as a fact about their backup, not have to already know to go looking
/// for it.
pub fn describe_health(now_unix: i64, health: &Option<BackupHealth>) -> String {
    match health {
        None => "never — running without a net".to_string(),
        Some(h) => format!(
            "{} ago ({} node event(s), {} clinical event(s), {} bytes -> {})",
            humanize_ago(now_unix - h.last_backup_unix),
            h.node_events,
            h.clinical_events,
            h.medium_bytes,
            h.medium_path,
        ),
    }
}

/// Read the backup-health sidecar. Returns `None` on ANY error (absent, unreadable,
/// malformed) — the fail-safe reading, so `status` degrades to "never / running without
/// a net" rather than asserting a freshness it cannot vouch for. A lying or missing
/// sidecar can only UNDER-claim; it can never cause data loss, because the load-bearing
/// guarantee lives in the self-verifying medium, not here.
pub fn read_health(path: &Path) -> Option<BackupHealth> {
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Atomically write the backup-health sidecar (owner-only). Atomic so a torn write can
/// never corrupt the freshness reading; the flip from old→new is a single rename.
pub fn write_health(path: &Path, health: &BackupHealth) -> Result<(), BackupError> {
    let json = serde_json::to_vec_pretty(health)
        // `Encode`, not a decode fault: this is a failure to SERIALIZE our own data, not a
        // property of any medium on disk (the fault taxonomy is `cairn_medium::BackupError`).
        .map_err(|e| BackupError::Encode(format!("serializing backup health: {e}")))?;
    crate::fsio::atomic_write(path, &json, Some(0o600))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// DB / IO glue (thin — the only DB-touching part of this module).
// ---------------------------------------------------------------------------

/// How strongly the medium `backup_to` left on disk identifies the node it belongs to, so
/// the caller can warn an operator when a medium is weaker than tamper-evident.
///
/// **Derived from the medium AFTER the write, never from what this run intended.** That
/// distinction became load-bearing when `backup_to` started APPENDING (#500 slice 2c): a run
/// over an unchanged log appends no segment at all, so "we had a key, therefore the medium is
/// signed" would describe an intention rather than a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WrittenMarker {
    /// No identity on the medium at all — the node is not yet enrolled, so there is nothing
    /// to attest and nothing to name.
    None,
    /// The medium names this node only in the UNTRUSTED plaintext `Segment::self_node_id_hex`
    /// (the signing key was not available at capture, so no segment carries an attestation).
    ///
    /// ⚠️ **Weaker on CAIRNB3 than the CAIRNB2 unsigned head marker it replaces**, and the
    /// difference is real rather than cosmetic: [`self_marker_for`] derives a marker from the
    /// ATTESTED id only, so an entirely-unsigned CAIRNB3 medium yields `None` and
    /// `restore`'s `confirm_explicit` cross-check (issue #53's footgun) does not run on it.
    /// A CAIRNB2 unsigned marker, equally forgeable, still ran that check. Filed as
    /// [#550](https://github.com/cairn-ehr/cairn-ehr/issues/550) rather than fixed here: what
    /// a V3 medium's untrusted plaintext `Segment::self_node_id_hex` may be used for is a
    /// decision about `self_marker_for` and `restore`, not about the writer that started
    /// producing the revision.
    Unsigned,
    /// The medium carries an ATTESTED identity: at least one signed segment whose attestation
    /// verifies AND whose claimed node has a genesis on this same medium signed by the same
    /// key. Unforgeable (the private key never leaves the node); tampering can only WITHHOLD
    /// it, never misdirect (see [`crate::medium`]).
    Signed,
}

/// Why this backup is writing the bytes it is writing — whether it CONTINUED the medium it
/// found at the target path or started a new one in its place.
///
/// An operator has to be told, because the file at `--to` may no longer be the artifact they
/// backed up to yesterday: CAIRNB3 segments cannot be appended to a CAIRNB1/CAIRNB2
/// container (there is no chain to hang them from), so the first backup after this slice
/// lands necessarily starts a NEW medium.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediumOrigin {
    /// A CAIRNB3 medium was already at the path; this run appended to it. The normal case
    /// from the second backup onward.
    Continued,
    /// Nothing was at the path. A first backup.
    FirstEver,
    /// A CAIRNB1/CAIRNB2 medium was at the path and has been SUCCEEDED by a new CAIRNB3 one,
    /// replacing the file.
    ///
    /// Safe **for a medium of THIS node's own event set**, and only because the first capture
    /// of a fresh medium resumes from an ABSENT watermark, which is the same instruction as
    /// "sweep from the beginning": `node_event` is append-only, so the successor holds
    /// everything that legacy medium held, plus the clinical plane the legacy revision could
    /// never carry. Pinned by
    /// `tests/backup_carries_both_planes.rs::a_legacy_medium_is_succeeded_by_a_cairnb3_medium_holding_at_least_as_much`.
    ///
    /// ⚠️ **The precondition is real: a legacy medium belonging to a DIFFERENT node — a peer's,
    /// or this node's own from before a restore minted it a new identity — is replaced, not
    /// merged, and its events are not in this database to be re-swept.** That hazard is not
    /// introduced here (today's whole-set writer already overwrote whatever was at `--to`, and
    /// a CAIRNB3 medium is now APPENDED to rather than replaced, so this path is the only one
    /// left that destroys), but it is not closed here either: nothing checks whose medium this
    /// was. An operator reusing a backup volume across nodes must copy the old file aside
    /// first, and the CLI note says so.
    SucceededLegacy,
}

/// What one backup did. **Per plane, never as one total** — a single count with no scope is
/// precisely the shape that let #500 hide for months while every surface reported it truly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupReport {
    /// Federation-plane (`node_event`) records the medium NOW HOLDS — not what this run
    /// appended. Counted by [`plane_counts`] over the re-read medium.
    pub node_events: usize,
    /// Clinical-plane (`event_log`) records the medium NOW HOLDS, same derivation.
    pub clinical_events: usize,
    /// Federation-plane records THIS RUN appended. `0` on an unchanged log — the property
    /// CAIRNB3 exists for.
    pub node_appended: usize,
    /// Clinical-plane records THIS RUN appended, same meaning.
    pub clinical_appended: usize,
    pub medium_bytes: usize,
    pub marker: WrittenMarker,
    pub origin: MediumOrigin,
}

/// Read this node's signed `node_event` set — the whole FEDERATION plane, in local `seq`
/// order. A plain `SELECT`: any role with read access works (the runtime `cairn_node` role
/// has `GRANT SELECT ON node_event`), and no signing key and no validated door are needed.
///
/// **This is no longer the backup writer, and has not been since #500 slice 2c Task 9.** The
/// `⚠️ #500` warning that stood here — *"`node_event` IS THE WHOLE MEDIUM, and that is a live
/// defect"* — described a real defect that is now closed at this layer: [`backup_to`] captures
/// BOTH planes through [`crate::capture::capture_plane`], reading `node_event` through
/// `capture::read_node_page` and `event_log` through db/051's `cairn_clinical_page`, paging
/// from each plane's own watermark rather than sweeping a whole table. So a solo clinic's
/// medium now carries its clinical, demographic, identity, registration and erasure streams,
/// with per-record custody, and `BackupHealth` v2 reports the two planes separately instead
/// of one scope-free total.
///
/// **What is still open, and must not be read off this comment as fixed.** #500 itself stays
/// open: the medium HOLDS the clinical record, and nothing yet RESTORES it —
/// [`node_plane_events`] hands `restore`/`verify-backup` the federation plane alone, on
/// purpose, and slice 2d owns the other half. `tests/dr_clinical_guarantee_gap.rs` pins both
/// halves: that the medium carries both planes, and that nothing reads the clinical one back.
///
/// **What this function is FOR now.** It answers the narrower question its name asks — "what
/// is this node's federation event set?" — for callers that want the events themselves rather
/// than a medium: the DR guarantee suite reads it to compare the database against the medium,
/// and it is the one-line reference spelling of that set. It is deliberately NOT wired back
/// into the write path; a capture must page from a watermark, not re-read a whole table,
/// or a nightly backup re-records the clinic's whole history (see `capture_plane`'s property 2).
pub async fn read_event_set(db: &tokio_postgres::Client) -> anyhow::Result<Vec<Vec<u8>>> {
    use anyhow::Context;
    let rows = db
        .query("SELECT signed_bytes FROM node_event ORDER BY seq", &[])
        .await
        .context("reading node_event set for backup")?;
    Ok(rows.iter().map(|r| r.get::<_, Vec<u8>>(0)).collect())
}

/// This node's own genesis node-id (hex), from `local_node`, or `None` if not yet enrolled.
/// The authoritative answer to "whose backup is this?" — read while we are still live, and
/// written into every segment this capture appends (set-union sync cannot erase what we put
/// into the container).
async fn read_self_node_id(db: &tokio_postgres::Client) -> anyhow::Result<Option<String>> {
    use anyhow::Context;
    let row = db
        .query_opt(
            "SELECT encode(node_id,'hex') AS id FROM local_node WHERE id",
            &[],
        )
        .await
        .context("reading local_node id for the backup self-marker")?;
    Ok(row.map(|r| r.get::<_, String>("id")))
}

/// How many events one capture page reads.
///
/// **The same number as `cairn_wire::DEFAULT_PAGE_EVENTS`, spelled here rather than imported.**
/// ADR-0026 decision 2 makes a backup medium *a cold peer*, so a capture page and a slice-2b
/// PULL page are the same operation seen from two sides — same ≈2 MiB working set, same bound
/// on how much work one interruption discards. `cairn-node` deliberately does not depend on
/// `cairn-wire` (`capture_plane` takes the page size as a PARAMETER for exactly that reason,
/// see its doc), so the two constants live apart.
///
/// A drift between them would cost nothing but a differently-sized page: this is a work-batch
/// size, not a wire constant — no reader of a medium can tell what page size wrote it, because
/// a segment boundary carries no meaning beyond "one append increment". That is why a shared
/// definition is not worth a crate dependency here.
const CAPTURE_PAGE_EVENTS: i64 = 500;

/// The bytes this backup will append to, plus WHY they are those bytes.
///
/// The three outcomes are deliberately not collapsed (`MediumOrigin`'s doc has the operator
/// consequence). The fourth possible state of the target path — a file that exists and is
/// NOT a readable Cairn medium — is a REFUSAL rather than a fourth variant; see below.
fn open_or_start_medium(path: &Path) -> anyhow::Result<(Vec<u8>, MediumOrigin)> {
    use anyhow::Context;

    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // A first backup. `serialize_v3(&[])` is an 8-byte magic header and nothing
            // else; the capture below appends every segment.
            return Ok((crate::medium::serialize_v3(&[])?, MediumOrigin::FirstEver));
        }
        // Present but UNREADABLE (permissions, an I/O error, a mount that went away). We
        // must not overwrite what we could not read: a good medium behind a transient read
        // error would be destroyed by a "start fresh" fallback, and the fallback would look
        // like a successful backup while doing it.
        Err(e) => {
            return Err(anyhow::Error::new(e)).with_context(|| {
                format!(
                    "reading the existing backup medium {} (refusing to overwrite a medium \
                     this run could not read — fix the read error, or point --to at a \
                     different path to start a new medium)",
                    path.display()
                )
            })
        }
    };

    match crate::medium::parse_any(&bytes) {
        // The normal case from the second backup onward: append to what is already there.
        // A TORN tail is not handled here on purpose — `capture_plane` truncates to
        // `MediumV3::complete_bytes` before it appends, which is the only place that
        // recovery may happen (see its doc; appending after a torn remnant orphans every
        // later backup forever).
        Ok(MediumImage::V3(_)) => Ok((bytes, MediumOrigin::Continued)),

        // A CAIRNB1/CAIRNB2 medium. A CAIRNB3 segment has nowhere to attach in a legacy
        // container — there is no chain — so the only options are "refuse forever" or "start
        // a new medium here". Starting a new one is safe because the first capture of a
        // fresh medium sweeps from an absent watermark, i.e. from the beginning of both
        // planes: the successor is a strict superset of what the legacy medium carried.
        // The operator is TOLD (`MediumOrigin::SucceededLegacy` → the CLI line), because the
        // file at this path is no longer the artifact they backed up to yesterday.
        Ok(MediumImage::Legacy(_)) => Ok((
            crate::medium::serialize_v3(&[])?,
            MediumOrigin::SucceededLegacy,
        )),

        // Present, readable, and NOT a medium this build can parse. Refused, never replaced,
        // and this is the one place `backup` can now fail where it previously always
        // succeeded. Three reasons it is the right direction:
        //
        //  1. It may not be a medium at all — an operator typo pointing `--to` at a keystore,
        //     an export, or a patient file. Silently overwriting it (today's behaviour) is a
        //     data-destroying operator-error footgun with no undo.
        //  2. If it IS a damaged medium, #523 says a corrupt section length UNDER the cap is
        //     indistinguishable from a torn tail and the two remedies are OPPOSITE. Replacing
        //     it destroys the only copy a future, repaired parser could read.
        //  3. Nothing is lost by refusing: the events are still in the database, and the
        //     remedy is one flag (`--to` a new path), which then writes a complete medium.
        //
        // It fails LOUDLY (non-zero exit) rather than warning, because ADR-0026 decision 7 is
        // that a node which cannot currently back up must say so.
        Err(e) => anyhow::bail!(
            "{} exists but is not a backup medium this build can read ({e}). Refusing to \
             overwrite it: if this path is a typo it may be a file you need, and if it is a \
             DAMAGED medium then replacing it destroys the only copy — a corrupt section \
             length is indistinguishable from an interrupted append (#523) and the two \
             remedies are opposite. Point --to at a NEW path to write a complete medium (the \
             first capture sweeps both planes from the beginning, so nothing is lost), and \
             keep this file for diagnosis.",
            path.display()
        ),
    }
}

/// How strongly the medium on disk identifies its node. PURE — the classification alone, so
/// it can be exercised without a database or a signing key.
///
/// Derived from the FINAL medium rather than from this run's intent: a backup over an
/// unchanged log appends no segment, so "a key was available, therefore the medium is signed"
/// would be a claim about the process rather than about the artifact.
///
/// `enrolled` is the database's answer ("does `local_node` name us?"), not the medium's. It
/// is what separates the two weak cases: a node with no identity has nothing to attest
/// ([`WrittenMarker::None`]), whereas an enrolled node whose capture ran without a key has an
/// identity that simply did not get signed onto the medium ([`WrittenMarker::Unsigned`]) —
/// different situations with different remedies, and folding them together would tell an
/// operator with a passphrase problem that their node is not enrolled.
fn written_marker(
    m: &crate::medium::MediumV3,
    report: &crate::medium::ChainReport,
    enrolled: bool,
) -> WrittenMarker {
    if crate::medium::self_id_from_chain(m, report).is_some() {
        WrittenMarker::Signed
    } else if enrolled {
        WrittenMarker::Unsigned
    } else {
        WrittenMarker::None
    }
}

/// Borrow the CAIRNB3 image out of an already-parsed `image`, naming `what` in the failure.
///
/// `backup_to` builds its buffer with `serialize_v3` and only ever appends CAIRNB3 segments
/// to it, so a legacy image here is a broken writer rather than a case to handle — but it is
/// written as a refusal, not an `expect`, so a future format change cannot turn it into a
/// panic in an unattended nightly job.
///
/// It BORROWS rather than returning an owned `MediumV3` for one reason worth stating: a
/// medium is the size of a clinic's whole event log, and cloning it once per backup to satisfy
/// the borrow checker would silently double the peak memory of the one operation that already
/// holds the entire log in RAM.
fn v3_of<'a>(image: &'a MediumImage, what: &str) -> anyhow::Result<&'a crate::medium::MediumV3> {
    match image {
        MediumImage::V3(m) => Ok(m),
        MediumImage::Legacy(_) => anyhow::bail!(
            "{what} parsed as a CAIRNB1/CAIRNB2 container; a capture only ever writes \
             CAIRNB3, so this build's writer and reader disagree — refusing rather than \
             guessing"
        ),
    }
}

/// Refuse a medium image that is not SOUND, quoting what `cairn-medium` found.
///
/// One helper for BOTH the pre-write and the post-write check, so the two can never drift
/// into different ideas of "good enough" — the same reason [`plane_counts`] exists once.
/// [`crate::medium::assess`] is the composed verdict (chain + every record's signature +
/// the tail); every narrower predicate in that crate returns `true` for some medium that is
/// not sound, which is the composite untruth this whole slice is about.
fn refuse_unsound(
    health: &crate::medium::MediumHealth,
    what: &str,
    remedy: &str,
) -> anyhow::Result<()> {
    if health.sound() {
        return Ok(());
    }
    anyhow::bail!(
        "{what} is not sound: chain_intact={}, {} of {} record signature(s) intact (first \
         bad at {:?}), truncated_tail={}, faults={:?}. {remedy}",
        health.chain.chain_intact(),
        health.records.intact,
        health.records.total,
        health.first_bad_record,
        health.truncated_tail,
        health.chain.faults
    )
}

/// Back up BOTH event planes to `medium_path`, then record health at `health_path`.
///
/// **This is where #500's payload lands (slice 2c Task 9).** Before it, this function wrote
/// `SELECT signed_bytes FROM node_event` and nothing else: a solo clinic backed up nightly,
/// `verify-backup` passed, the disk died, and `restore` recovered who it had peered with and
/// zero patients. It now captures the FEDERATION plane and then the CLINICAL plane — the
/// latter carrying per-record custody — onto one CAIRNB3 medium.
///
/// (#500 itself is NOT closed by this function. The medium holds the clinical record; nothing
/// yet restores it. See [`node_plane_events`], and `tests/dr_clinical_guarantee_gap.rs`, which
/// pins both halves.)
///
/// `marker_key` is the node's signing key (+ key-id). When present, every segment this run
/// appends carries a signed attestation, and the medium can identify its own node
/// unforgeably. When `None` the segments are written UNSIGNED — they still NAME the node in
/// plaintext, but nothing binds the claim. **A missing key never blocks a backup**, and that
/// is a §1.2 paper-parity requirement rather than a convenience: an unattended cron run has no
/// passphrase and therefore no key, so refusing would turn the operator's one nightly act into
/// two (`M > N`, an architecture defect under house rule 7).
///
/// # The order of operations, and why each step is where it is
///
/// 1. **Capture `Plane::Node` FIRST, then `Plane::Clinical`, into ONE buffer.** The CAIRNB3
///    chain is a single global chain in FILE order across both planes (that is what lets it
///    detect a reordering or a splice ACROSS planes, which two independent chains could not),
///    so the two passes must share one buffer and one cursor. Node first is not arbitrary: a
///    restore needs a federation identity before anything else is meaningful, so if a medium
///    is ever read only in part, the half that arrives first is the half that establishes
///    whose backup it is.
/// 2. **On ANY capture error the buffer is DISCARDED, unwritten.** `capture_plane` can
///    backfill a gap, append it, and only THEN fail on the tail — returning `Err` with our
///    buffer already mutated. Byte-identity on refusal holds only before its first append. The
///    `?` operators below are therefore load-bearing: they drop the buffer on the way out and
///    the previous good medium on disk is never touched. Do not "helpfully" write a partial
///    capture here.
/// 3. **Verify BEFORE the bytes can reach the medium.** Two layers, and both are needed:
///    `capture_plane` refuses any single record whose signature does not verify at the moment
///    it appends it (a segment attestation commits to the CONTENT ADDRESS of whatever it is
///    handed, so a corrupt read would otherwise be signed into a genuinely VALID attestation
///    over corruption); then the whole staged image is put through
///    [`crate::medium::assess`] — the composed verdict — before the write. A staged image that
///    is not sound BAILS with the previous medium completely untouched.
/// 4. **Write atomically** (`fsio::atomic_write`: `sync_all` on a temp sibling, then rename,
///    then a parent-directory fsync on unix). This is the durability half of `capture_plane`'s
///    contract, which that function deliberately does not do — it is pure of I/O by design.
///    A crash here never destroys the previous medium: the rename lands whole or not at all.
///    The write happens even when nothing was appended, so a nightly run still proves the
///    backup volume is mounted and writable rather than reporting success against a vanished
///    mount.
/// 5. **Re-read and re-assess the ON-DISK bytes** — a defence-in-depth tripwire for a
///    filesystem bug between write and rename. Still BAILS without touching health.
/// 6. **Only then update the health sidecar**, from the re-read medium. A crash between (5)
///    and (6) leaves health UNDER-reporting (older, or "never"), which is the correct
///    direction for a safety-net indicator: it must never over-claim.
///
/// # What a failure costs, and why refusing is the safe direction
///
/// Every bail above leaves the previous medium and the previous health sidecar exactly as
/// they were, so a refused backup loses nothing that was already safe — the events are still
/// in the database. What it does cost is TONIGHT's backup, and the process exits non-zero, so
/// an operator is paged. That is ADR-0026 decision 7 working as intended: a node that cannot
/// currently back up is running without a net and must say so.
///
/// `now_unix` is injected (operational wall-clock) so the function stays deterministic and
/// testable; the CLI passes `SystemTime::now()`.
pub async fn backup_to(
    db: &tokio_postgres::Client,
    medium_path: &Path,
    health_path: &Path,
    now_unix: i64,
    marker_key: Option<(&cairn_event::SigningKey, &str)>,
) -> anyhow::Result<BackupReport> {
    use crate::medium::Plane;
    use anyhow::Context;

    // Whose backup is this? Read while the node is still live; `local_node` is the authority.
    // `None` = not yet enrolled: the segments below then name themselves with the empty
    // string, which `Segment::self_node_id_hex` documents as exactly that state ("empty
    // before enrolment, when there is no identity to name yet"). We do NOT skip the capture
    // in that case — a database holding events but no identity must still be backed up, and
    // silently writing nothing would be a fresh instance of #500's own shape.
    let self_id = read_self_node_id(db).await?;
    let enrolled = self_id.is_some();
    let self_id_hex = self_id.unwrap_or_default();

    let (mut buffer, origin) = open_or_start_medium(medium_path)?;

    // STEP 1 + 2. Node, then Clinical, into the one buffer. Each `?` DISCARDS `buffer`
    // unwritten — see the doc's step 2; that absence of a write is the safety property, and
    // it is invisible in the code, which is why it is written down.
    let node = capture::capture_plane(
        db,
        &mut buffer,
        Plane::Node,
        marker_key,
        &self_id_hex,
        CAPTURE_PAGE_EVENTS,
    )
    .await
    .with_context(|| {
        format!(
            "capturing the federation plane onto {} (nothing was written; the previous \
             medium is untouched)",
            medium_path.display()
        )
    })?;
    let clinical = capture::capture_plane(
        db,
        &mut buffer,
        Plane::Clinical,
        marker_key,
        &self_id_hex,
        CAPTURE_PAGE_EVENTS,
    )
    .await
    .with_context(|| {
        format!(
            "capturing the clinical plane onto {} (nothing was written; the previous medium \
             is untouched)",
            medium_path.display()
        )
    })?;

    // STEP 3. Verify-before-write over the WHOLE staged image, not only over what this run
    // appended. That is deliberate and it is the one place `backup` can refuse over damage it
    // did not cause: continuing to append to a medium that cannot fully restore, while
    // reporting a fresh healthy backup, is exactly the composite untruth this slice exists to
    // end. The remedy is one flag and the next run writes a complete medium.
    const STAGED: &str = "the image this capture just built";
    let staged_image = crate::medium::parse_any(&buffer)
        .context("re-parsing the image this capture just built, to assess it before writing")?;
    refuse_unsound(
        &crate::medium::assess(v3_of(&staged_image, STAGED)?),
        STAGED,
        "NOTHING was written and the previous medium is untouched. Point --to at a NEW path: \
         the first capture of a fresh medium sweeps both planes from the beginning, so the \
         successor holds everything this one did.",
    )?;

    // STEP 4.
    crate::fsio::atomic_write(medium_path, &buffer, Some(0o600))
        .with_context(|| format!("writing backup medium to {}", medium_path.display()))?;

    // STEP 5. Read-after-write.
    let readback = std::fs::read(medium_path)
        .with_context(|| format!("re-reading backup medium {}", medium_path.display()))?;
    let written_image = crate::medium::parse_any(&readback).with_context(|| {
        format!(
            "re-parsing the medium just written to {}",
            medium_path.display()
        )
    })?;
    let durable_name = format!("the freshly-written medium {}", medium_path.display());
    let durable = v3_of(&written_image, &durable_name)?;
    let durable_health = crate::medium::assess(durable);
    refuse_unsound(
        &durable_health,
        &durable_name,
        "Health was NOT advanced, so `status` and `verify-backup` keep reporting the last \
         backup that genuinely succeeded. The bytes verified in memory immediately before \
         the write, so suspect the filesystem or the device.",
    )?;

    // Per-plane counts recorded into health come from `plane_counts` over this SAME
    // verified, already-re-read medium image — never re-derived by hand from what the capture
    // said it appended — so this call site can never silently disagree with
    // `verify-backup`/`restore` about how many records a medium holds (Task 8's whole reason
    // for `plane_counts` existing as one function). It is also why Task 10's per-plane health
    // fields needed no edit when this task started appending `Plane::Clinical` segments: the
    // line asks the medium, not a vector that only ever held one plane.
    let counts = plane_counts(&written_image);

    // The medium's newest CLINICAL seq, re-derived from the durable bytes and their chain
    // report rather than taken from `clinical.watermark`. The two agree today (that field is
    // itself re-derived from the buffer), and deriving it here anyway keeps the rule that
    // every number in the sidecar describes the artifact on disk — the only thing a restore
    // can actually use. `None` is the honest absence, never a guessed `Some(0)`.
    let clinical_watermark =
        crate::medium::watermark(durable, &durable_health.chain, Plane::Clinical);

    let written = written_marker(durable, &durable_health.chain, enrolled);

    // `export_covers_seq` belongs to a DIFFERENT artifact than anything this function
    // touches — the CAIRNL1 local-state export, sealed and written later in the `backup`
    // command (see `main.rs`'s `Cmd::Backup` arm), never here. Resetting it to `None` on
    // every call would silently forget a coverage figure a PRIOR run actually earned, the
    // moment tonight's export is (deliberately, safely) skipped — exactly backwards for a
    // value `verify-backup`'s staleness check depends on. So, from THIS function's own point
    // of view, no export ran at all this call: `ExportOutcome::Skipped` over whatever the
    // existing sidecar already recorded, via the same pure rule Task 12's export site will
    // use when it actually writes one.
    let previous_export_covers_seq = read_health(health_path).and_then(|h| h.export_covers_seq);

    let health = BackupHealth {
        version: 2,
        last_backup_unix: now_unix,
        medium_path: medium_path.display().to_string(),
        medium_bytes: readback.len() as u64,
        node_events: counts.node as u64,
        clinical_events: counts.clinical as u64,
        clinical_watermark,
        export_covers_seq: export_coverage_after(
            previous_export_covers_seq,
            ExportOutcome::Skipped,
        ),
    };
    write_health(health_path, &health).context("writing backup-health sidecar")?;

    Ok(BackupReport {
        node_events: counts.node,
        clinical_events: counts.clinical,
        node_appended: node.records_appended,
        clinical_appended: clinical.records_appended,
        medium_bytes: readback.len(),
        marker: written,
        origin,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn humanize_ago_buckets_are_coarse_and_safe() {
        assert_eq!(
            humanize_ago(-5),
            "just now",
            "a backwards clock must not show a negative age"
        );
        assert_eq!(humanize_ago(0), "just now");
        assert_eq!(humanize_ago(42), "42s");
        assert_eq!(humanize_ago(120), "2m");
        assert_eq!(humanize_ago(7200), "2h");
        assert_eq!(humanize_ago(172_800), "2d");
    }

    #[test]
    fn describe_health_warns_when_absent_and_summarizes_when_present() {
        assert_eq!(
            describe_health(1000, &None),
            "never — running without a net"
        );
        let h = BackupHealth {
            version: 2,
            last_backup_unix: 1000,
            medium_path: "/mnt/backup/cairn.medium".into(),
            medium_bytes: 2048,
            node_events: 7,
            clinical_events: 3,
            clinical_watermark: Some(41),
            export_covers_seq: None,
        };
        let line = describe_health(1000 + 3600, &Some(h));
        assert!(line.starts_with("1h ago"), "freshness first: got {line:?}");
        assert!(
            line.contains("7 node event(s)"),
            "must name the node plane: {line:?}"
        );
        assert!(
            line.contains("3 clinical event(s)"),
            "must name the clinical plane, not fold it into one count: {line:?}"
        );
        assert!(line.contains("/mnt/backup/cairn.medium"));
    }

    #[test]
    fn health_sidecar_roundtrips_and_absent_reads_as_none() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("backup-status.json");
        assert_eq!(
            read_health(&p),
            None,
            "a missing sidecar reads as None (fail-safe)"
        );
        let h = BackupHealth {
            version: 2,
            last_backup_unix: 12_345,
            medium_path: "/mnt/x".into(),
            medium_bytes: 999,
            node_events: 3,
            clinical_events: 0,
            clinical_watermark: None,
            export_covers_seq: None,
        };
        write_health(&p, &h).unwrap();
        assert_eq!(
            read_health(&p),
            Some(h),
            "a written sidecar reads back exactly"
        );
    }

    #[test]
    fn malformed_sidecar_reads_as_none_not_a_crash() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("backup-status.json");
        std::fs::write(&p, b"{ this is not valid json").unwrap();
        assert_eq!(
            read_health(&p),
            None,
            "a corrupt sidecar must fail safe to None"
        );
    }

    #[test]
    fn health_path_is_a_sibling_of_the_key() {
        let p = health_path_for(Path::new("/var/lib/cairn/node.key"));
        assert_eq!(p, Path::new("/var/lib/cairn/backup-status.json"));
    }

    /// The two WEAK arms of `written_marker`, which are the ones an operator acts on and the
    /// two a naive implementation folds together. Both are reachable with no signing key and
    /// no database, because the distinction they carry is not about the medium's contents at
    /// all — it is about whether `local_node` names us.
    ///
    /// The `Signed` arm needs a real attested segment bound to a genesis on the same medium,
    /// so it is pinned end-to-end instead, against a live key and a real capture, by
    /// `tests/backup_carries_both_planes.rs` (and its negative twin, the unsigned capture).
    #[test]
    fn written_marker_separates_not_enrolled_from_enrolled_but_unsigned() {
        // An empty CAIRNB3 medium: no segment, therefore no attestation, therefore no
        // attested id for `self_id_from_chain` to return.
        let bytes = crate::medium::serialize_v3(&[]).unwrap();
        let m = match crate::medium::parse_any(&bytes).unwrap() {
            MediumImage::V3(m) => m,
            MediumImage::Legacy(_) => panic!("serialize_v3 must produce a CAIRNB3 image"),
        };
        let report = crate::medium::chain_report(&m);

        assert_eq!(
            written_marker(&m, &report, false),
            WrittenMarker::None,
            "a node with no `local_node` row has no identity to attest — that is not the \
             same situation as a passphrase that was unavailable"
        );
        assert_eq!(
            written_marker(&m, &report, true),
            WrittenMarker::Unsigned,
            "an enrolled node whose capture ran without a key HAS an identity; it just did \
             not get signed onto the medium. Reporting `None` here would send an operator \
             to `provision` instead of to their passphrase."
        );
    }
}
