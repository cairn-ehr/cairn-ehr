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
//! verify-on-apply path. This module reads the signed `node_event` set, writes it to a local
//! medium (with a self-marker so restore can tell which node it belongs to — see `medium`),
//! and surfaces backup health (point 7: a node running without a net must say so).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

// The medium format is defined once in `crate::medium`. Re-export the surface other modules
// and tests already reach for via `backup::…`, so the format move stays source-compatible.
pub use crate::medium::{
    parse_medium, verify_event, verify_events, verify_medium_bytes, BackupError, SelfMarker,
    VerifyReport,
};

use crate::medium::{MediumImage, Plane};

// ---------------------------------------------------------------------------
// Reading a medium of EITHER revision (Erratum E2, #500 slice 2c design doc §4).
//
// WHY THIS EXISTS. `restore` and `verify-backup` both still read a medium through the
// LEGACY parser (`parse_container`, which refuses CAIRNB3 outright). The moment `backup_to`
// starts writing CAIRNB3 (the very next task in this slice), that refusal stops being a
// safety net and becomes the defect: an operator's nightly backup would verify RED and
// their only restore path would refuse to read it — the working half of disaster recovery,
// broken by the slice that is fixing the broken half. This section lands FIRST so no commit
// in this branch's history is ever unable to read a medium it can write.
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
///   CAIRNB3 medium carries the CLINICAL plane too (from the next task onward), but
///   restoring it is slice 2d's job — returning it here would silently let 2c be read as
///   having closed #500's restore half, which it has deliberately not (design doc §8).
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

/// Which self-marker `backup_to` actually wrote into the medium, so the caller can warn an
/// operator when a medium is only operator-error-safe (unsigned) rather than tamper-evident.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WrittenMarker {
    /// No marker — the node is not yet enrolled, so there is no identity to attest.
    None,
    /// The self node-id without a signature (the signing key was not available at backup).
    Unsigned,
    /// A signed self-attestation — tampering can only withhold it, never misdirect (medium docs).
    Signed,
}

/// What one backup did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupReport {
    pub event_count: usize,
    pub medium_bytes: usize,
    pub marker: WrittenMarker,
}

/// Read this node's signed `node_event` set, in local `seq` order. A plain `SELECT` —
/// any role with read access works (the runtime `cairn_node` role has `GRANT SELECT ON
/// node_event`); no signing key and no validated door are needed to back up.
///
/// ⚠️ **#500 — `node_event` IS THE WHOLE MEDIUM, and that is a live defect.** This is the
/// federation plane: enroll, peer, revoke, supersede. `event_log` is NOT read here or
/// anywhere else on the backup path, so the medium carries **no `event_log` row at all** —
/// not clinical, not demographic, not identity, not registration, not erasure — while
/// ADR-0026 decision 1 promises *"the clinical event log survives"* a restore and decision 2
/// says *"clinical events back up as a cold peer"*. For the solo clinic that ADR opens by
/// naming as first-class — the one for which *"replication provides zero durability"* — a
/// dead disk is total record loss, not merely loss of clinical content.
///
/// **And the operator is never told.** ADR-0026 decision 7: *"Backup health is a first-class
/// honest-assembly fact."* — *"A node that cannot currently back up is running without a net
/// and must say so."* `backup-status.json` and `status` report freshness
/// truly, and `verify-backup` reports the medium's INTEGRITY truly (not its health, and not
/// its scope); none of them can see that the scope is wrong, because nothing on this path
/// distinguishes the two planes. Every surface is honest and the composite is a precise
/// untruth — decision 7 defeated by a system in which no single component lies.
///
/// Its sibling is #495 (a restored solo node cannot unwrap inherited custody); fixing
/// either alone is useless. Pinned by
/// `tests/dr_clinical_guarantee_gap.rs::medium_carries_the_federation_plane_and_no_clinical_event`,
/// which checks both this function's result AND the medium file `backup_to` writes.
pub async fn read_event_set(db: &tokio_postgres::Client) -> anyhow::Result<Vec<Vec<u8>>> {
    use anyhow::Context;
    let rows = db
        .query("SELECT signed_bytes FROM node_event ORDER BY seq", &[])
        .await
        .context("reading node_event set for backup")?;
    Ok(rows.iter().map(|r| r.get::<_, Vec<u8>>(0)).collect())
}

/// This node's own genesis node-id (hex), from `local_node`, or `None` if not yet enrolled.
/// The authoritative answer to "whose backup is this?" — recorded into the medium's marker
/// while we are still live (set-union sync cannot erase what we write into the container).
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

/// Choose the self-marker to embed: signed when the node's key is supplied AND the node is
/// enrolled; unsigned when enrolled but no key was available; none when not yet enrolled. The
/// signed attestation is bound to `events` (the exact set being backed up) so it cannot be
/// replayed onto another medium.
fn choose_marker(
    self_id: Option<String>,
    marker_key: Option<(&cairn_event::SigningKey, &str)>,
    events: &[Vec<u8>],
) -> Option<SelfMarker> {
    let id = self_id?;
    match marker_key {
        Some((sk, key_id)) => Some(SelfMarker::Signed(crate::medium::build_self_attestation(
            sk, key_id, &id, events,
        ))),
        None => Some(SelfMarker::Unsigned(id)),
    }
}

/// Back up the node's event set to `medium_path`, then record health at `health_path`.
///
/// `marker_key` is the node's signing key (+ key-id): when present, the medium carries a
/// SIGNED self-attestation (tamper can only withhold it on restore, never misdirect — see
/// [`crate::medium`]); when `None`, an UNSIGNED self-marker is written instead (still closes
/// the operator-typo footgun, just not tamper-evident). An unsigned marker NEVER blocks a
/// backup — the caller decides whether the key is available and warns accordingly.
///
/// Ordering is deliberately fail-safe and verify-BEFORE-write:
///   1. serialize the medium and self-verify the image IN MEMORY — if the event set fails its
///      own signature check we BAIL before touching disk, so the previous good medium at
///      `medium_path` is left completely untouched;
///   2. write the verified image atomically (a crash here never destroys the previous good
///      medium either — the rename either lands whole or not at all);
///   3. re-read and self-verify the on-disk bytes — a defence-in-depth tripwire for an fs bug
///      between write and rename; still BAIL WITHOUT touching health if it fails;
///   4. only then update the health sidecar.
///
/// A crash between (3) and (4) leaves health UNDER-reporting (older / "never"), which is the
/// correct direction for a safety-net indicator — it must never over-claim.
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
    use anyhow::Context;
    let events = read_event_set(db).await?;
    let self_id = read_self_node_id(db).await?;
    let marker = choose_marker(self_id, marker_key, &events);
    let written = match &marker {
        None => WrittenMarker::None,
        Some(SelfMarker::Unsigned(_)) => WrittenMarker::Unsigned,
        Some(SelfMarker::Signed(_)) => WrittenMarker::Signed,
    };

    // Verify the event set BEFORE it can overwrite the live medium: a set that fails its own
    // signature check is rejected here, with the previous good medium still intact on disk.
    let medium = crate::medium::serialize_and_verify_container(marker.as_ref(), &events)
        .context("self-verifying the backup image before writing (previous medium untouched)")?;

    crate::fsio::atomic_write(medium_path, &medium, Some(0o600))
        .with_context(|| format!("writing backup medium to {}", medium_path.display()))?;

    // Read-after-write: the on-disk bytes must still parse AND verify (catches an fs/rename
    // bug), or we refuse to advance health (never tell the operator a broken backup is good).
    let readback = std::fs::read(medium_path)
        .with_context(|| format!("re-reading backup medium {}", medium_path.display()))?;
    let report = verify_medium_bytes(&readback)
        .with_context(|| format!("verifying freshly-written medium {}", medium_path.display()))?;
    if !report.all_intact() {
        anyhow::bail!(
            "backup medium failed self-verification after write ({} of {} events intact, \
             first bad at index {:?}); health NOT advanced",
            report.intact,
            report.total,
            report.first_bad
        );
    }

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
        medium_bytes: medium.len() as u64,
        // #500 slice 2c Task 9 (not yet landed at this line's authorship): this function
        // still reads ONLY `node_event`, so every event backed up today is federation-plane
        // by construction. `clinical_events`/`clinical_watermark` stay at the honest
        // "nothing captured yet" until the clinical capture pass lands — never a guess.
        node_events: events.len() as u64,
        clinical_events: 0,
        clinical_watermark: None,
        export_covers_seq: export_coverage_after(
            previous_export_covers_seq,
            ExportOutcome::Skipped,
        ),
    };
    write_health(health_path, &health).context("writing backup-health sidecar")?;

    Ok(BackupReport {
        event_count: events.len(),
        medium_bytes: medium.len(),
        marker: written,
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

    #[test]
    fn choose_marker_picks_signed_unsigned_or_none() {
        let events: Vec<Vec<u8>> = vec![];
        // No identity yet → no marker (nothing to attest).
        assert_eq!(choose_marker(None, None, &events), None);
        // Enrolled but no key available → unsigned marker carrying the self id.
        assert_eq!(
            choose_marker(Some("abcd".into()), None, &events),
            Some(SelfMarker::Unsigned("abcd".into()))
        );
        // Enrolled + key → a signed attestation (a Signed variant; its bytes are exercised in
        // the medium module's verification tests).
        let (sk, _) = cairn_event::generate_key().unwrap();
        let kid = hex::encode(sk.verifying_key().to_bytes());
        assert!(matches!(
            choose_marker(Some("abcd".into()), Some((&sk, &kid)), &events),
            Some(SelfMarker::Signed(_))
        ));
    }
}
