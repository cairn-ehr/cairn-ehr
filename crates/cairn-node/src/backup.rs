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

/// Which CLINICAL records the RESTORE path applies, for either medium revision — the
/// sibling of [`node_plane_events`] and, since #554 slice 2d, the reader that finally makes
/// a restored node have patients.
///
/// **It is a thin adapter over [`cairn_medium::plane_records`] and must stay one.** That
/// function gates on `verified_through`, sorts by `source_seq` and collapses byte-identical
/// re-capture duplicates; re-implementing any of it here would be a second derivation of a
/// trust boundary, which is what an earlier draft of this slice did and what its review
/// caught. `cairn_wire::MediumTransport` serves from the same function, so the serving path
/// and the disaster-recovery path cannot drift onto two different answers about what a
/// medium may be trusted for.
///
/// **Why whole [`cairn_medium::MediumRecord`]s and not bare bytes**, where `node_plane_events` returns
/// `Vec<Vec<u8>>`: the clinical plane carries three things the federation plane does not —
/// the attestation pair (which the apply door re-verifies for a suppressing event) and the
/// **wrapped DEK**, without which a restored sealed body is permanently unreadable. Handing
/// back only `signed_bytes` here would have made design §2.2's dropped-custody failure
/// unreachable to fix at the call site.
///
/// **A legacy medium returns EMPTY**, and that is the truthful answer: CAIRNB1/CAIRNB2
/// predate the plane split, so no clinical record exists on one to fail to read. It is not
/// a truthful thing to show an operator on its own — *"restored, 0 clinical events"* is
/// #500's exact signature reproduced inside the machinery built to close it — so naming
/// that outcome is the CALLER's job (design §5.2), not this function's.
///
/// Returns `Result` for symmetry with [`node_plane_events`] and this module's other readers.
/// Given an already-parsed image it cannot fail today; the signature leaves room for a
/// revision that can, without changing every caller.
pub fn clinical_plane_records(
    image: &MediumImage,
) -> Result<Vec<cairn_medium::MediumRecord>, BackupError> {
    Ok(match image {
        MediumImage::Legacy(_) => Vec::new(),
        MediumImage::V3(m) => {
            let report = cairn_medium::chain_report(m);
            cairn_medium::plane_records(m, &report, Plane::Clinical)
        }
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

/// How a medium's self-marker was derived. The marker's WIRE SHAPE cannot say this on a
/// CAIRNB3 medium — both derivations below produce a `SelfMarker::Unsigned` — so the trust
/// level travels beside it rather than being guessed from the variant.
///
/// WHY IT HAS TO (#500 slice 2c review): without it, a fully tamper-evident sole-enroll V3
/// medium was described to the operator as "UNSIGNED (not tamper-evident)". Under-claiming is
/// the safe direction, but it is still a false statement to a human at the moment they are
/// deciding whether to trust a restore.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerSource {
    /// A CAIRNB1/CAIRNB2 container's own head marker, passed through unchanged. Its own
    /// `SelfMarker` variant already says whether it was signed.
    LegacyContainer,
    /// CAIRNB3: the ATTESTED node id from `chain::self_id_from_chain` — a verified segment
    /// attestation, bound to a genesis on THIS medium signed by the SAME key. **Unforgeable**
    /// (the private key never leaves the node), and therefore the CAIRNB3 equivalent of a
    /// CAIRNB2 *signed* head marker despite being carried in the `Unsigned` variant.
    V3Attested,
    /// CAIRNB3: the untrusted plaintext `Segment::self_node_id_hex`, used only because no
    /// attestation on this medium could supply an id. **Exactly the trust level a CAIRNB2
    /// unsigned head marker had** — forgeable by anyone who can write the file, still
    /// cross-checked by `resolve_dead_node`'s `Unsigned` arm against the medium's own enrolls.
    V3Plaintext,
}

/// Derive the self-marker `resolve_dead_node` should use, for either medium revision, AND how
/// it was derived. ONE derivation, so the marker and the trust statement made about it to an
/// operator can never disagree.
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
/// - **CAIRNB3, first choice:** `chain::self_id_from_chain` — the ATTESTED id (two binds
///   already checked there: the segment attestation verifies, and the named node has a
///   genesis on THIS medium signed by the same key). Wrapped as `SelfMarker::Unsigned`, not
///   `Signed`: `SelfMarker::Signed`'s verifier (`verify_self_attestation`) expects a CAIRNB2
///   whole-set-committing `node.self_attested` blob, a different wire shape from a segment
///   attestation, and would wrongly raise `InvalidSelfMarker` on a perfectly good V3 medium.
///   `MarkerSource::V3Attested` is what carries the real trust level out.
/// - **CAIRNB3, fallback:** the plaintext `Segment::self_node_id_hex` of the last segment
///   that carries a non-empty one.
///
/// # Why the plaintext fallback exists, and why it invents no trust (#550)
///
/// An unattended cron backup has no passphrase, therefore no key, therefore writes UNSIGNED
/// segments — and §1.2 requires that never to block a backup. Without this fallback such a
/// medium yields `None`, and `resolve_dead_node` then takes its marker-less path. That was a
/// REGRESSION against the CAIRNB2 medium it replaced, in two directions at once:
///
///   1. `confirm_explicit` never runs, so ANY `--superseded-node` naming an enroll on the
///      medium is accepted UNCHECKED — issue #53's original footgun, where an operator typo
///      becomes an immutable supersede edge against the wrong node;
///   2. with NO `--superseded-node`, a multi-enroll medium becomes `RestoreError::Ambiguous`
///      — so a restore that worked on CAIRNB2 now REFUSES, for exactly the passphrase-less
///      case that cannot avoid it.
///
/// The fallback restores parity and nothing more. `Segment::self_node_id_hex` is untrusted
/// plaintext — its own doc says so — and so was a CAIRNB2 unsigned marker: both are forgeable
/// by anyone who can write the file, and `resolve_dead_node`'s `Unsigned` arm re-checks either
/// one against the enrolls actually present on the medium before honouring it, yielding
/// `InvalidSelfMarker` for an off-medium id and `NotSelf` for a named peer. The caller is told
/// which derivation it got via [`MarkerSource`] and must say so; **never present a
/// `V3Plaintext` marker as tamper-evident.**
///
/// `None` now only when a V3 medium carries no attestation AND no non-empty plaintext id
/// anywhere — a capture by a node that was not yet enrolled. `resolve_dead_node`'s marker-less
/// fallback then applies, which is correct: there is genuinely no identity claim to check.
pub fn self_marker_source(image: &MediumImage) -> Option<(SelfMarker, MarkerSource)> {
    match image {
        MediumImage::Legacy(container) => container
            .self_marker
            .clone()
            .map(|m| (m, MarkerSource::LegacyContainer)),
        MediumImage::V3(medium) => {
            let report = crate::medium::chain_report(medium);
            if let Some(attested) = crate::medium::self_id_from_chain(medium, &report) {
                return Some((SelfMarker::Unsigned(attested), MarkerSource::V3Attested));
            }
            // The LAST segment that names itself, mirroring the attested path's "last
            // verified signed segment": the most recent writer is the one whose backup this
            // is. Empty ids are skipped — `Segment::self_node_id_hex` uses the empty string
            // for "captured before enrolment", which is an absence, not a claim.
            medium
                .segments
                .iter()
                .rev()
                .map(|s| s.self_node_id_hex.as_str())
                .find(|id| !id.is_empty())
                .map(|id| {
                    (
                        SelfMarker::Unsigned(id.to_string()),
                        MarkerSource::V3Plaintext,
                    )
                })
        }
    }
}

/// The marker alone, for callers that do not surface a trust statement.
///
/// A thin wrapper over [`self_marker_source`] rather than a second derivation — the #522
/// lesson: two places deriving one answer is how they come to disagree.
pub fn self_marker_for(image: &MediumImage) -> Option<SelfMarker> {
    self_marker_source(image).map(|(marker, _)| marker)
}

/// The node id an already-written **CAIRNB3** medium names, with the provenance of that id —
/// the input to [`refuse_foreign_continuation`]'s identity guard.
///
/// Built on [`self_marker_source`] rather than re-deriving it (#522): the nightly identity
/// guard and the restore surface must never come to disagree about whose medium this is.
///
/// A V3 medium's marker is always [`SelfMarker::Unsigned`] — the ATTESTED id is carried in
/// that variant too, with `MarkerSource::V3Attested` beside it to say so, see
/// `self_marker_source` — so the `Signed` arm is unreachable here and answers `None` rather
/// than being guessed at.
///
/// Legacy containers answer `None` deliberately. They are guarded on the other arm, by
/// `refuse_unsafe_legacy_succession` via `OpenedMedium::superseded`, and their marker already
/// has its own reader in `legacy_claimed_node`; answering here would put two derivations on
/// one question, which is the defect #522 was filed about.
fn v3_claimed_node(image: &MediumImage) -> Option<(String, MarkerSource)> {
    if matches!(image, MediumImage::Legacy(_)) {
        return None;
    }
    match self_marker_source(image)? {
        (SelfMarker::Unsigned(id), source) => Some((id, source)),
        (SelfMarker::Signed(_), _) => None,
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
/// The sidecar shape this build WRITES, and the floor at which it trusts the per-plane
/// counts. A sidecar below this records no plane scope at all (v1 had a single
/// `event_count`), and `describe_health` must say so rather than render the serde defaults
/// as facts — see [`describe_health`].
pub const SUPPORTED_HEALTH_VERSION: u8 = 2;

// `Eq` is deliberately absent: `extra` holds `serde_json::Value`, which is `PartialEq` but
// not `Eq` (floats). Nothing needs a total equality here, and preserving a newer build's
// fields is worth more than the marker trait.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackupHealth {
    /// Which sidecar shape wrote this. READ, not decorative: `describe_health` refuses to
    /// present v1's absent per-plane counts as zeros. Compare against
    /// [`SUPPORTED_HEALTH_VERSION`].
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
    /// Every field this build does not know, carried through a read-modify-write untouched
    /// (final review, I14).
    ///
    /// `main.rs`'s export ceremony reads this sidecar, advances `export_covers_seq`, and
    /// writes it back. Plain serde DROPS what it does not recognise, so an older binary run
    /// once against a newer sidecar — a rollback, a rescue USB, a second node sharing the key
    /// directory — silently erased whatever the newer build had recorded. Principle 11
    /// (additive evolution across a mixed-version fleet) applied to the one file
    /// `verify-backup` reads to decide whether a kit is restorable.
    ///
    /// `deny_unknown_fields` is deliberately NOT used here, unlike [`crate::localstate`]'s
    /// `LocalState` one layer over: v1's `event_count` is an unknown field to this build, so
    /// refusing would break the v1 sidecar this struct still reads on purpose. Preserving is
    /// backward AND forward compatible; refusing is only forward.
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
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

/// PURE. What a completed export write actually ACHIEVED — which is not the same question as
/// whether the file landed (#500 slice 2c final review, Critical 2).
///
/// `backup` can seal, write and read-back-verify an export that carries the custody rows and
/// **no unwrap key**. It reaches that state deliberately: `<key>.unwrap` may be absent (a node
/// provisioned before [ADR-0066] decision 5), bit-rotted, or sealed under the other operator
/// secret, and `seal_and_write_local_state_export` warns and carries on rather than aborting,
/// because the medium is the load-bearing copy and the export is optional. The artifact that
/// results is durable, readable, well-formed — and opens nothing, because ADR-0066's whole
/// point is that the key and the bytes are useless apart.
///
/// So the FILE landing must not be mistaken for the export achieving something.
/// [`ExportOutcome::Skipped`]'s doc already names "a load failure" as one of `backup`'s
/// warn-and-continue paths; this function is what makes the call site honour it. Recording
/// `Written` there was the exact false green [`export_coverage_after`] exists to prevent:
/// `kit_verdict(Some(N), Some(N))` returns `Restorable`, `verify-backup` exits 0, and every
/// sealed body on the kit restores as ciphertext, permanently — while `status`, reading the
/// same node, shouts about the missing key.
///
/// Split out as its own pure function for the same reason `export_coverage_after` is: it is
/// a whole safety property, and it should be testable without a database, a keystore or a
/// medium — see `tests/backup_health_v2.rs`.
///
/// [ADR-0066]: https://github.com/cairn-ehr/cairn-ehr/blob/main/docs/spec/decisions/0066-identity-dies-with-the-disk-custody-must-not.md
pub fn export_outcome_for_write(custody_key_carried: bool, covers_seq: i64) -> ExportOutcome {
    if custody_key_carried {
        ExportOutcome::Written(covers_seq)
    } else {
        ExportOutcome::Skipped
    }
}

/// The medium's own newest CLINICAL seq, read straight off `image` — no key, no database,
/// no sidecar. `verify-backup`'s kit-staleness check ([`kit_verdict`]) uses this rather than
/// a figure carried in `backup-status.json`, because a number derived from the bytes
/// `--from` actually names can never describe a DIFFERENT backup run than the one being
/// checked — the same "every number describes the artifact on disk" rule `backup_to`
/// already follows for this same field (see `clinical_watermark`'s doc on [`BackupHealth`]).
///
/// `None` for a legacy (CAIRNB1/CAIRNB2) medium — the clinical plane did not exist when that
/// format was frozen, so there is nothing to have a watermark over — and for a CAIRNB3
/// medium with no verified clinical segment at all (a fresh node, or one backed up before
/// #500 slice 2c started capturing this plane). Both are the honest absence, never a
/// guessed `Some(0)` (principle 4).
pub fn clinical_watermark_of(image: &MediumImage) -> Option<i64> {
    match image {
        MediumImage::Legacy(_) => None,
        MediumImage::V3(m) => {
            let health = crate::medium::assess(m);
            crate::medium::watermark(m, &health.chain, Plane::Clinical)
        }
    }
}

/// What `verify-backup` concluded about the DR kit as a WHOLE — the medium plus the sealed
/// local-state export sitting beside it — rather than about the medium alone (review
/// finding I5's shape, carried forward: the federation-plane events being intact is not the
/// same claim as the kit being restorable). See [`kit_verdict`].
#[derive(Debug, PartialEq, Eq)]
pub enum KitVerdict {
    /// Medium and export agree: everything on the medium has custody coverage.
    Restorable,
    /// The medium holds clinical events written after the export last covered anything.
    /// Those bodies restore as ciphertext unless their medium-borne DEKs open them.
    ExportStale {
        medium_seq: i64,
        /// The seq the export last actually achieved. NOT an `Option`: `kit_verdict` reaches
        /// this variant only through `Some(export_seq) if export_seq < medium_seq`, so a
        /// `None` here was a state the constructor could not produce — and the caller paid
        /// for it with a dead branch that printed the literal words "none (unexpected)" to an
        /// operator mid-disaster. `ExportMissing` is where an absent figure lives.
        export_seq: i64,
    },
    /// No export beside the medium ever recorded coverage. Carries the operator-facing
    /// remedy directly, because unlike `ExportStale` there are no two numbers left for a
    /// caller to compose a message from — there is no coverage figure at all.
    ExportMissing(String),
    /// The health sidecar's coverage figure describes a DIFFERENT medium than the one under
    /// test (#500 slice 2c Task 12 fix round 1, Important 1) — a two-drive rotation, or any
    /// cron pointed at a fresh path, while `backup-status.json` stays one file per signing
    /// key. Never constructed by [`kit_verdict`] itself (which knows only two seq numbers,
    /// not paths); the caller builds this variant directly, BEFORE calling `kit_verdict` at
    /// all, once it has established the mismatch — see the `verify-backup` arm in
    /// `main.rs`. Kept as a `KitVerdict` variant rather than a separate early return so every
    /// non-`Restorable` outcome is handled by ONE match, with ONE exit-code policy.
    CoverageUnknown(String),
}

/// PURE. Decide whether a DR kit is actually restorable, from nothing but the two seq
/// numbers this module already tracks: `medium_seq` (the medium's own newest clinical seq —
/// [`clinical_watermark_of`] offline, or `BackupHealth::clinical_watermark` from the
/// sidecar) and `export_seq` (`BackupHealth::export_covers_seq`, the seq the export last
/// actually achieved, per [`export_coverage_after`]'s ratchet).
///
/// Pure and total, so the exit-code policy it drives is testable with no database, no
/// medium and no CLI — see `tests/verify_backup_scope.rs`.
///
/// - `medium_seq` is `None` exactly when the medium holds no clinical events at all (a
///   fresh node that has never written one). Nothing is uncovered, so this is
///   `Restorable` — never `ExportStale`, never `ExportMissing`. Calling a genuinely-empty
///   medium "stale" would train an operator to ignore the one signal this function exists
///   to raise.
/// - Otherwise `export_seq` decides it: `None` means no export has EVER recorded coverage
///   for this node (`ExportMissing` — the #502 lesson: a different remedy from a
///   merely-behind export, never conflated with it, because "run `backup` again" and
///   "the escrow needs recovering" are not the same instruction); `Some(x)` behind
///   `medium_seq` means the export is out of date (`ExportStale`); anything else — equal,
///   or AHEAD because this run's export landed after this run's medium capture — is
///   `Restorable`.
pub fn kit_verdict(medium_seq: Option<i64>, export_seq: Option<i64>) -> KitVerdict {
    let Some(medium_seq) = medium_seq else {
        return KitVerdict::Restorable;
    };
    match export_seq {
        None => KitVerdict::ExportMissing(format!(
            "the medium holds clinical events up to seq {medium_seq}, but no local-state \
             export has EVER recorded coverage for them — a restore would open none of \
             their sealed bodies as anything but ciphertext (ADR-0066). Remedy: recover \
             the export — confirm a local-state escrow exists (run \
             `cairn-node establish-local-state-key` if it does not) and then run `backup` \
             again with CAIRN_KEY_PASSPHRASE set (or --passphrase)."
        )),
        Some(export_seq) if export_seq < medium_seq => KitVerdict::ExportStale {
            medium_seq,
            export_seq,
        },
        Some(_) => KitVerdict::Restorable,
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
        // A sidecar older than v2 carries NO per-plane scope: `node_events`/`clinical_events`
        // are `#[serde(default)]` and read as zero, which is a serde artefact, not a fact the
        // sidecar ever stated. Rendering it unqualified told an operator that a
        // multi-megabyte medium holds nothing — and it appeared exactly in the window between
        // upgrading the binary and the first successful new `backup`, i.e. the window where
        // `backup` is most likely to be failing and the line most likely to be read.
        Some(h) if h.version < SUPPORTED_HEALTH_VERSION => format!(
            "{} ago (per-plane counts not recorded by the `backup` that wrote this sidecar — \
             run `backup` to refresh, {} bytes -> {})",
            humanize_ago(now_unix - h.last_backup_unix),
            h.medium_bytes,
            h.medium_path,
        ),
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

/// Does the `medium_path` a health sidecar recorded actually name `from` — the medium
/// `verify-backup` was asked about? (#500 slice 2c Task 12 fix round 1, Important 1.)
///
/// **Why this has to be checked at all.** `backup-status.json` lives beside the SIGNING
/// KEY (`health_path_for`), one file per node; `export_covers_seq` is a claim about ONE
/// specific export, written for whichever medium the most recent successful export
/// targeted. A node backing up to more than one medium/drive under the same key — the
/// ordinary two-drive rotation, or a cron job pointed at a fresh path — can have a sidecar
/// whose coverage figure describes a DIFFERENT artifact than the one named by `--from`. A
/// mismatch here is not "unknown" the way an absent sidecar is: it is a coverage figure
/// that positively describes something else, and reading it against the wrong medium would
/// let an easily-reachable rotation manufacture a false green on a kit that cannot actually
/// open its bodies — the exact failure this whole task exists to catch, reintroduced one
/// layer up. So the caller must refuse to reach `Restorable` on a mismatch, never merely
/// warn (see the `verify-backup` arm in `main.rs`).
///
/// Canonicalizes both sides where the named file exists — a relative `--from` and an
/// absolute recorded path can legitimately name the same medium — and falls back to a
/// literal path comparison when canonicalization fails on either side (e.g. the recorded
/// medium has since been moved or deleted): an honest "cannot confirm sameness by resolving
/// the filesystem", never a panic and never a silent pass.
pub fn health_describes_medium(recorded_medium_path: &str, from: &Path) -> bool {
    let recorded = Path::new(recorded_medium_path);
    match (std::fs::canonicalize(recorded), std::fs::canonicalize(from)) {
        (Ok(a), Ok(b)) => a == b,
        _ => recorded == from,
    }
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
    /// **Operator-error-safe, not tamper-evident** — exactly what a CAIRNB2 unsigned head
    /// marker was, and deliberately so: [`self_marker_source`] falls back to that plaintext id
    /// when no attestation can supply one, so `restore`'s `confirm_explicit` cross-check
    /// (issue #53's footgun) still runs and a multi-enroll medium still resolves self rather
    /// than going `Ambiguous`. Without that fallback this variant would have been strictly
    /// weaker than the format it replaced ([#550](https://github.com/cairn-ehr/cairn-ehr/issues/550));
    /// parity is restored, and it is parity, not an upgrade — anyone who can write the file
    /// can write the id.
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
    /// ⚠️ **The precondition is real, and it is now ENFORCED** (#500 slice 2c final review,
    /// Critical 1). A legacy medium belonging to a DIFFERENT node — a peer's on a shared
    /// backup volume, or this node's own from before a restore minted it a new identity —
    /// would be replaced, not merged, and its events are not in this database to be re-swept.
    /// The same shape swallowed a clinic's only medium after a disk failure: re-`init`, then
    /// `backup --to` before `restore`, and an empty-but-SOUND successor overwrote it.
    /// `refuse_unsafe_legacy_succession` now refuses the write when the legacy medium names
    /// another node or when the successor would carry fewer federation events than it did,
    /// leaving the old file untouched; pinned by the two `…_refuses_…` tests in
    /// `tests/backup_carries_both_planes.rs`.
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
    /// The medium found at the path had a TORN tail, which this run repaired by cutting back
    /// to `MediumV3::complete_bytes` before appending.
    ///
    /// Surfaced because the repair is otherwise INVISIBLE and looks alarming: `capture_plane`
    /// truncates whether or not it then appends anything, so a nightly run over an unchanged
    /// log can print `+0 / +0 appended` while the file on disk silently changes size. An
    /// operator watching byte counts would have no way to tell that from corruption.
    pub repaired_torn_tail: bool,
    /// The medium this run wrote would restore NOTHING — no record in any plane
    /// (`MediumHealth::carries_nothing`).
    ///
    /// Not an error: a genuinely new node must be able to write its first medium, and that
    /// rule is recorded in `verify-backup` (#502 item 2). But reporting plain success over an
    /// artifact that restores nothing is a green light on an empty file whose only other
    /// refusal comes at the disaster, so the caller is handed the fact and must say it.
    pub carries_nothing: bool,
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

/// What the LEGACY medium at the target path was carrying, remembered ONLY for as long as
/// it takes [`backup_to`] to decide whether succeeding it is safe.
///
/// WHY IT HAS TO TRAVEL (#500 slice 2c final review, Critical 1). Succeeding a CAIRNB1/B2
/// medium REPLACES the file: the successor is safe only under the precondition
/// [`MediumOrigin::SucceededLegacy`] names — that it is a medium of *this* node's own event
/// set, which the fresh capture then re-sweeps in full. Nothing checked that precondition,
/// and `read_self_node_id` returns `None` (not an error) on an initialised-but-not-enrolled
/// database, so an operator whose disk had just died could re-`init`, point `backup --to` at
/// their only medium, and have it replaced by an 8-byte empty one — sound, internally
/// consistent, and carrying nothing. Checking needs two facts the legacy image holds and
/// nothing downstream would otherwise see, so they are carried forward rather than re-read.
struct SupersededLegacy {
    /// The node the legacy medium claims for itself, lowercased — `None` when it makes no
    /// claim at all.
    ///
    /// `None` covers three honest cases and one deliberate degradation: a CAIRNB1 medium
    /// (predates the marker entirely), a CAIRNB2 medium captured before enrolment, and a
    /// SIGNED marker whose attestation does not verify against the events beside it. That
    /// last one is treated as "no claim" rather than as "a claim we distrust", because the
    /// only way to read an id out of an unverified attestation is to skip the signature
    /// check — which this codebase does nowhere — and because the count check below still
    /// covers the case. It can only ever WITHHOLD a refusal, never invent one.
    claimed_node_hex: Option<String>,
    /// How many events it carried. A legacy container has no planes: every event on it is a
    /// federation-plane event, which is why the successor's NODE count is what it compares to.
    node_events: usize,
}

/// The bytes this backup will append to, plus WHY they are those bytes.
///
/// The three outcomes are deliberately not collapsed (`MediumOrigin`'s doc has the operator
/// consequence). The fourth possible state of the target path — a file that exists and is
/// NOT a readable Cairn medium — is a REFUSAL rather than a fourth variant; see below.
struct OpenedMedium {
    /// The bytes the capture appends to. Empty-but-framed for the two "start fresh" outcomes.
    buffer: Vec<u8>,
    origin: MediumOrigin,
    /// The medium found on disk had a torn tail. Reported, not acted on here — the repair is
    /// `capture_plane`'s.
    torn_tail_repaired: bool,
    /// `Some` exactly when `origin == MediumOrigin::SucceededLegacy`: what the medium about
    /// to be REPLACED was carrying, for [`refuse_unsafe_legacy_succession`].
    superseded: Option<SupersededLegacy>,
    /// Whose CAIRNB3 medium this ALREADY is, when it says so — `(node id, how the id was
    /// derived)`, from the one derivation in [`self_marker_source`]. `Some` only on the
    /// `Continued` arm; `None` for a fresh medium, a legacy succession (guarded by
    /// `superseded` instead), and for a medium that names nobody.
    ///
    /// One field carrying both values rather than two Options that must agree: the id is
    /// meaningless without the provenance, because the message an operator reads has to say
    /// whether the id is attested or forgeable. See [`refuse_foreign_continuation`].
    continued_claim: Option<(String, MarkerSource)>,
}

/// Read the target path and classify it. See [`OpenedMedium`].
fn open_or_start_medium(path: &Path) -> anyhow::Result<OpenedMedium> {
    use anyhow::Context;

    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // A first backup. `serialize_v3(&[])` is an 8-byte magic header and nothing
            // else; the capture below appends every segment.
            return Ok(OpenedMedium {
                buffer: crate::medium::serialize_v3(&[])?,
                origin: MediumOrigin::FirstEver,
                torn_tail_repaired: false,
                superseded: None,
                // Nothing to be foreign to: this path CREATES the medium.
                continued_claim: None,
            });
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

    let parsed = crate::medium::parse_any(&bytes);
    // ONE derivation of "whose medium is this", taken BEFORE the match: the `Continued` arm's
    // pattern destructures the `MediumImage` away, and `self_marker_source` needs the whole
    // thing. Answering here also means the identity guard reads the medium exactly as it was
    // found, before a single record is captured onto it.
    let claim = parsed.as_ref().ok().and_then(v3_claimed_node);

    match parsed {
        // The normal case from the second backup onward: append to what is already there.
        // A TORN tail is not handled here on purpose — `capture_plane` truncates to
        // `MediumV3::complete_bytes` before it appends, which is the only place that
        // recovery may happen (see its doc; appending after a torn remnant leaves every
        // later backup unreachable behind it — loudly since #523, but still unreachable).
        Ok(MediumImage::V3(ref m)) => {
            // Reported, not acted on, here: the actual repair is `capture_plane`'s (it must
            // truncate to `complete_bytes` immediately before it appends, or a torn remnant
            // sits where the next section header belongs and strands every later backup
            // behind it). This only remembers that it is about to happen, so the caller can
            // say so.
            let torn = m.truncated_tail;
            Ok(OpenedMedium {
                buffer: bytes,
                origin: MediumOrigin::Continued,
                torn_tail_repaired: torn,
                superseded: None,
                // Read from the medium we are about to append to, BEFORE anything is
                // captured — `backup_to` refuses on a mismatch (Critical 1).
                continued_claim: claim,
            })
        }

        // A CAIRNB1/CAIRNB2 medium. A CAIRNB3 segment has nowhere to attach in a legacy
        // container — there is no chain — so the only options are "refuse forever" or "start
        // a new medium here". Starting a new one is safe because the first capture of a
        // fresh medium sweeps from an absent watermark, i.e. from the beginning of both
        // planes: the successor is a strict superset of what the legacy medium carried.
        // The operator is TOLD (`MediumOrigin::SucceededLegacy` → the CLI line), because the
        // file at this path is no longer the artifact they backed up to yesterday.
        //
        // THAT SAFETY ARGUMENT HAS A PRECONDITION, AND THIS IS WHERE ITS INPUTS ARE TAKEN
        // (#500 slice 2c final review, Critical 1). "The successor is a strict superset" holds
        // only if the legacy medium is of THIS node's own event set. It is not enforceable
        // here — `open_or_start_medium` is pure of the database, and the successor's contents
        // do not exist yet — so the two facts needed to enforce it are carried out to
        // `backup_to`, which checks them against the staged image immediately before the
        // write (`refuse_unsafe_legacy_succession`). Reading the marker costs one signature
        // scan of the legacy medium, once, on the single run that supersedes it.
        Ok(MediumImage::Legacy(container)) => Ok(OpenedMedium {
            buffer: crate::medium::serialize_v3(&[])?,
            origin: MediumOrigin::SucceededLegacy,
            torn_tail_repaired: false,
            superseded: Some(SupersededLegacy {
                claimed_node_hex: legacy_claimed_node(&container),
                node_events: container.events.len(),
            }),
            // The legacy arm carries its identity in `superseded` and is guarded by
            // `refuse_unsafe_legacy_succession`; two homes for one fact is the #522 defect.
            continued_claim: None,
        }),

        // Present, readable, and NOT a medium this build can parse. Refused, never replaced,
        // and this is the one place `backup` can now fail where it previously always
        // succeeded. Three reasons it is the right direction:
        //
        //  1. It may not be a medium at all — an operator typo pointing `--to` at a keystore,
        //     an export, or a patient file. Silently overwriting it (today's behaviour) is a
        //     data-destroying operator-error footgun with no undo.
        //  2. If it IS a damaged medium, replacing it destroys the only copy a future,
        //     repaired parser could read. (#523 has since made the DIAGNOSIS reliable — a
        //     corrupt length is now named as damage rather than mistaken for a torn tail —
        //     but that changes what we can TELL the operator, not whether their bytes are
        //     worth keeping.)
        //  3. Nothing is lost by refusing: the events are still in the database, and the
        //     remedy is one flag (`--to` a new path), which then writes a complete medium.
        //
        // It fails LOUDLY (non-zero exit) rather than warning, because ADR-0026 decision 7 is
        // that a node which cannot currently back up must say so.
        //
        // THE THREE VARIANTS ARE SPLIT, NOT FOLDED (#500 slice 2c final review, Important 3).
        // `error.rs`'s header is explicit that these situations have OPPOSITE remedies, and a
        // single catch-all arm here reintroduced the exact defect that taxonomy was created to
        // end: it told an operator holding a perfectly good medium written by a NEWER Cairn
        // that it "is not a backup medium this build can read" and to abandon it for a new
        // path — while `UnsupportedByThisBuild`'s own doc says *"Never treat this as damage.
        // Do not re-run a backup over this medium… Upgrade the node."* One message for three
        // diagnoses is how a good medium gets discarded mid-disaster.
        Err(e @ crate::medium::BackupError::NotAMedium(_)) => anyhow::bail!(
            "{} is not a Cairn backup medium at all ({e}). Nothing is damaged and nothing \
             was written — this looks like the wrong path. If `--to` is a typo, this may be \
             a file you still need, so it is left exactly as it was: check the path, or \
             point --to at a NEW one to write a complete medium (the first capture sweeps \
             both planes from the beginning, so nothing is lost).",
            path.display()
        ),
        Err(e @ crate::medium::BackupError::UnsupportedByThisBuild(_)) => anyhow::bail!(
            "{} is a VALID backup medium that THIS BUILD cannot fully read ({e}) — it was \
             written by a NEWER Cairn. The medium is fine; this node is behind it. The \
             remedy is to UPGRADE THIS NODE. Do NOT re-run a backup over this medium and do \
             not append to it: this build cannot see everything already on it, so an append \
             would write against an incomplete picture. Nothing was written and the medium \
             is untouched. If a backup must be taken before the upgrade, point --to at a \
             DIFFERENT path and leave this file alone.",
            path.display()
        ),
        Err(e @ crate::medium::BackupError::Damaged(_)) => anyhow::bail!(
            "{} is a DAMAGED backup medium ({e}). This is damage, NOT an interrupted \
             append — those are told apart at the section header since #523, and an \
             interrupted append would have left a short tail that `backup` repairs and \
             reports. \
             Refusing to overwrite it: replacing it destroys the only copy a future, \
             repaired parser could read. Keep this file for diagnosis, look for another \
             copy, and point --to at a NEW path to write a complete medium (the first \
             capture sweeps both planes from the beginning, so nothing is lost).",
            path.display()
        ),
        // `Io` and `Encode` cannot come out of `parse_any` (it reads a slice and writes
        // nothing), so this arm exists to keep the match total rather than to describe a
        // reachable state — and it says so, instead of guessing a remedy for a situation
        // nobody has diagnosed.
        Err(e) => anyhow::bail!(
            "reading the backup medium {} failed in a way this build does not expect from a \
             parse ({e}). Nothing was written and the medium is untouched. Please report \
             this with the message above; meanwhile, point --to at a NEW path to take \
             tonight's backup.",
            path.display()
        ),
    }
}

/// Which node a LEGACY (CAIRNB1/CAIRNB2) medium claims for itself, lowercased — or `None`
/// when it makes no claim this function is willing to read. PURE.
///
/// See [`SupersededLegacy::claimed_node_hex`] for what each `None` means and why an
/// unverifiable signed marker is folded into it. Split out as its own function so the
/// "what does the old medium say it is?" question has one spelling and can be reasoned
/// about without the surrounding I/O.
fn legacy_claimed_node(container: &crate::medium::Container) -> Option<String> {
    match container.self_marker.as_ref()? {
        // The untrusted plaintext id. Operator-error-safe, not tamper-evident — exactly the
        // trust level this check needs, because the failure it guards against is an operator
        // pointing `--to` at the wrong volume, not an attacker.
        SelfMarker::Unsigned(id) => Some(id.to_ascii_lowercase()),
        // The signed marker: verified against the events it sits beside, so a genuine peer's
        // medium names the peer unforgeably. `verify_self_attestation` already lowercases.
        SelfMarker::Signed(attestation) => {
            crate::medium::verify_self_attestation(attestation, &container.events)
        }
    }
}

/// Refuse to CONTINUE a CAIRNB3 medium that belongs to another node
/// (#500 slice 2c final review, Critical 1).
///
/// # The disaster this exists to prevent, and why it is worse than the legacy one
///
/// [`refuse_unsafe_legacy_succession`] below guards the arm that DESTROYS a file, and that
/// arm is loud: the operator is told the medium was superseded. This guards the arm that runs
/// every night, and it fails **silently**.
///
/// `capture_plane` resumes from `cairn_medium::watermark`, which filters segments by PLANE and
/// takes the `max` of `source_seq` — and `source_seq` is a node-local `GENERATED ALWAYS AS
/// IDENTITY` value. It never looks at `Segment::self_node_id_hex`. So the seq space of a
/// FOREIGN medium is read as if it were ours, and the arithmetic works out to silent loss:
///
/// > A peer's medium holds clinical seqs 1..100. This node holds 1..300. The capture resumes
/// > at `seq > 100` and writes our 101..300. **Our events 1..100 are never captured** — the
/// > medium already "has" those seq numbers, they are simply the peer's. `seq_gaps` reports
/// > no hole, because the seq space looks complete. `plane_counts` records 300 clinical
/// > events. `export_covers_seq` is 300. `kit_verdict(Some(300), Some(300))` returns
/// > `Restorable`, and `verify-backup` exits 0.
///
/// A medium missing a hundred of this clinic's earliest patient events, holding another
/// clinic's in their place, reported fully restorable — every surface honest, the composite
/// false. That is #500's own shape, reproduced on the path #500's fix runs down. Two
/// realistic ways to arrive there: a shared backup volume rotated between two clinics, and
/// this node's own pre-restore medium re-used after `restore` re-sequenced `event_log`.
///
/// # Why it refuses on an UNVERIFIED claim too
///
/// `claimed` carries [`MarkerSource`] so the message can be honest about provenance, but both
/// derivations refuse. Refusing on a forgeable plaintext id is the fail-safe direction: the
/// worst a forged foreign id achieves is costing tonight's backup, loudly, with a remedy the
/// operator can act on — whereas TRUSTING one is precisely how a foreign medium gets appended
/// to. The asymmetry is the whole reason the check is worth having on an untrusted field.
///
/// # What it does NOT catch, stated so nobody reads it as more than it is
///
/// A medium that names NOBODY claims nothing and is allowed through — silence is not
/// disagreement (principle 4), and refusing it would block the legitimate first backup of a
/// node enrolled after its first capture. Since #550 an enrolled node always names itself in
/// plaintext even with no key, so this is narrow: a genuinely pre-enrolment capture. It is
/// the same residual shape as the unmarked-legacy medium of
/// [#553](https://github.com/cairn-ehr/cairn-ehr/issues/553), on the other arm.
///
/// It also cannot un-mix a medium two nodes have ALREADY both written to: `self_marker_source`
/// answers with the most recent writer, so if we wrote last, we look like the owner. What this
/// closes is the FIRST contact, which is the event that creates the mix.
fn refuse_foreign_continuation(
    claimed: Option<&(String, MarkerSource)>,
    self_id_hex: &str,
    path: &Path,
) -> anyhow::Result<()> {
    // Silence is not a mismatch. See the doc's "What it does NOT catch".
    let Some((claimed_hex, source)) = claimed else {
        return Ok(());
    };
    if claimed_hex.eq_ignore_ascii_case(self_id_hex) {
        return Ok(());
    }

    // The provenance sentence, so an operator is never told a forgeable id is proof. This is
    // the same honesty `MarkerSource` was introduced for on the restore surface.
    let provenance = match source {
        MarkerSource::V3Attested => {
            "That id is ATTESTED — it comes from a verified segment \
             attestation bound to a genesis enroll on this same medium, so it is not a guess"
        }
        MarkerSource::V3Plaintext => {
            "That id is UNVERIFIED plaintext (no attestation on this \
             medium could supply one), so treat it as a strong hint rather than proof — but \
             the refusal stands either way, because appending to a medium that might be \
             another node's is the one direction that loses data silently"
        }
        // A legacy container never reaches this function: `open_or_start_medium` routes it to
        // `MediumOrigin::SucceededLegacy`, which `refuse_unsafe_legacy_succession` guards.
        MarkerSource::LegacyContainer => "That id came from a legacy container head marker",
    };

    let whose = if self_id_hex.is_empty() {
        "this database is NOT ENROLLED, so none of that node's events are here to append — \
         and none of any node's. If the old node's disk died, run `restore` FROM this medium \
         first: backing up onto it instead leaves everything it already holds permanently \
         uncaptured while the file grows and reports success"
            .to_string()
    } else {
        format!(
            "this node is {self_id_hex}, so that node's events are not in this database. \
             Appending here would resume from ITS sequence numbers and silently skip every \
             event of ours below them"
        )
    };

    anyhow::bail!(
        "refusing to continue the backup medium {}: it is a CAIRNB3 medium belonging to node \
         {claimed_hex}, and {whose}. {provenance}. NOTHING was written — the medium at this \
         path is exactly as it was. Point --to at a NEW path: that writes a complete medium \
         of THIS node (the first capture sweeps both planes from the beginning), and leaves \
         this one intact to be read, copied aside, or restored from.",
        path.display()
    );
}

/// Refuse to REPLACE a legacy medium whose successor would not hold everything it held.
/// PURE — no database, no filesystem — so both refusal arms are exercisable directly.
///
/// # The disaster this exists to prevent (#500 slice 2c final review, Critical 1)
///
/// [`MediumOrigin::SucceededLegacy`] destroys the file at `--to` and puts a fresh CAIRNB3
/// medium in its place. That is safe under ONE precondition — the legacy medium is of this
/// node's own event set, so the fresh capture's full sweep re-records everything it held —
/// and until this function nothing checked it. Two realistic ways the precondition fails:
///
///  1. **The medium belongs to another node.** A peer's medium on a shared backup volume, or
///     this node's own from before a restore minted it a new identity. Those events are not
///     in this database, so no sweep can recover them.
///  2. **The database no longer holds what the medium does.** The disk died, the operator
///     re-`init`ed a node, and — *before* running `restore` — ran `backup --to` at their only
///     medium. `read_self_node_id` answers `None` (not an error) on an un-enrolled database,
///     the capture reads zero rows, and the staged image is genuinely SOUND: `chain_intact`
///     holds, `all_intact()` is vacuously true at 0 of 0, and there is no torn tail. So
///     `refuse_unsound` passes it, the write lands, and the clinic's only backup becomes an
///     8-byte header — with `carries_nothing` warned to stderr *after* the file is gone.
///
/// Arm 2 is what makes this a hard refusal rather than a warning: by the time anything else
/// could notice, the artifact it would have warned about no longer exists.
///
/// # Why COUNTS, and why that is enough here
///
/// A count cannot prove set inclusion, and this codebase's own rule is *name, never count*.
/// It does not need to prove it: the two planes are compared for the only property that can
/// be violated by a full sweep of an append-only table, namely that the sweep found FEWER
/// events than the medium already held. `node_event` is append-only, so for this node's own
/// medium the successor's count is monotone — a shortfall is therefore proof of a different
/// event set, never a false alarm about ordering or content. (The strict superset itself is
/// pinned event-by-event by
/// `tests/backup_carries_both_planes.rs::a_legacy_medium_is_succeeded_by_a_cairnb3_medium_holding_at_least_as_much`.)
fn refuse_unsafe_legacy_succession(
    legacy: &SupersededLegacy,
    staged_node_records: usize,
    self_id_hex: &str,
    path: &Path,
) -> anyhow::Result<()> {
    // The remedy is the same for both arms and is the whole point of refusing: the operator
    // still gets tonight's backup, and the artifact they cannot rebuild stays on disk.
    const REMEDY: &str = "NOTHING was written — the medium at this path is exactly as it \
                          was. Point --to at a NEW path: that writes a complete medium of \
                          THIS node (the first capture sweeps both planes from the \
                          beginning), and leaves the old one intact to be read, copied \
                          aside, or restored from.";

    // ARM 1 — it is somebody else's medium. Only checked when the medium actually names a
    // node: a CAIRNB1 medium or a pre-enrolment capture claims nothing, and silence is not a
    // mismatch (principle 4 — no data is never disagreement).
    if let Some(claimed) = &legacy.claimed_node_hex {
        if !claimed.eq_ignore_ascii_case(self_id_hex) {
            let whose = if self_id_hex.is_empty() {
                "this database is NOT ENROLLED, so it has no events of that node — or of any \
                 node — to sweep back onto a successor. If the old node's disk died, run \
                 `restore` FROM this medium first; running `backup` first replaces the only \
                 copy"
                    .to_string()
            } else {
                format!(
                    "this node is {self_id_hex}, so that node's events are not in this \
                     database and no capture here can put them back"
                )
            };
            anyhow::bail!(
                "refusing to replace the backup medium {}: it is a CAIRNB1/CAIRNB2 medium \
                 belonging to node {claimed}, and {whose}. Succeeding a legacy medium \
                 REPLACES the file, and that is only safe for a medium of this node's own \
                 event set. {REMEDY}",
                path.display()
            );
        }
    }

    // ARM 2 — it is (or may be) our medium, but the successor would hold LESS. This is the
    // arm that catches the un-enrolled/empty database even when the old medium named nobody.
    if staged_node_records < legacy.node_events {
        anyhow::bail!(
            "refusing to replace the backup medium {}: it carries {} federation event(s) and \
             the medium that would replace it carries only {}. Succeeding a legacy medium \
             REPLACES the file, and is only safe when the fresh capture sweeps back at least \
             everything the old medium held. It did not, so this database is not the one that \
             wrote that medium (a restore that has not run yet, a re-`init`ed node, or \
             another node's volume). {REMEDY}",
            path.display(),
            legacy.node_events,
            staged_node_records
        );
    }
    Ok(())
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

/// Put a medium IMAGE through the composed [`crate::medium::assess`] verdict and refuse it if
/// it does not hold — the whole-medium check, for callers that hold an image rather than a
/// `MediumHealth`.
///
/// # Why this is public, and what went wrong without it (#500 slice 2c final review, Important 2)
///
/// `backup_to` has run this verdict since Task 9, and `verify-backup` had not: it computed
/// `assess()` (inside [`clinical_watermark_of`]), threw the verdict away, and rested its "OK"
/// on `verify_events` over the FEDERATION plane alone. So one spliced or reordered segment, or
/// one corrupt clinical record, printed `federation-plane events OK: N/N verified` and exited
/// 0 while `backup` over the very same bytes refused. Two commands, opposite verdicts, one
/// file — and `cairn-medium`'s own `health` module says `assess` exists precisely so that no
/// caller concludes anything from a narrower predicate. Sharing ONE function is what keeps the
/// cron health check and the writer from drifting into different ideas of "good enough".
///
/// The UNKNOWN-PLANE case is checked FIRST and separately, exactly as in `backup_to`, because
/// its remedy is the opposite of every other unsound medium's: *upgrade this node*, never
/// *fetch another copy* and never *start a new medium* ([`crate::medium::MediumHealth::needs_a_newer_build`]).
/// An unknown plane also makes `chain_intact()` false, so without this arm a perfectly good
/// medium written by a newer Cairn would fall through to the generic refusal and be given
/// precisely the advice `BackupError::UnsupportedByThisBuild` forbids. The WORDING differs
/// from `backup_to`'s deliberately, and only the wording: that one is about to APPEND, so its
/// refusal says why appending in particular is wrong; this one only ever reads.
///
/// A CAIRNB1/CAIRNB2 image returns `Ok` and nothing is skipped: a legacy container has no
/// segments and no chain, so `assess` — a verdict about a CAIRNB3 chain — has nothing to say
/// about one, and on such a medium `node_plane_events` already IS every event the file holds,
/// so the caller's own flat signature pass covers the whole artifact.
pub fn refuse_unsound_medium(image: &MediumImage, what: &str, remedy: &str) -> anyhow::Result<()> {
    let MediumImage::V3(m) = image else {
        return Ok(());
    };
    let health = crate::medium::assess(m);
    if health.needs_a_newer_build() {
        anyhow::bail!(
            "{what} carries {} record(s) in a plane this build does not recognise, so it was \
             written by a NEWER Cairn and this build cannot see all of it. The medium is \
             fine; this node is behind it. The remedy is to UPGRADE THIS NODE — not to fetch \
             another copy, and not to start a new medium.",
            health.records_in_unknown_planes
        );
    }
    refuse_unsound(&health, what, remedy)
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

    let OpenedMedium {
        mut buffer,
        origin,
        torn_tail_repaired: repaired_torn_tail,
        superseded,
        continued_claim,
    } = open_or_start_medium(medium_path)?;

    // THE CONTINUATION IDENTITY GUARD (#500 slice 2c final review, Critical 1), and why it
    // sits BEFORE the captures rather than beside `refuse_unsafe_legacy_succession` below.
    //
    // The legacy guard has to run late: its safety argument is about the STAGED IMAGE, which
    // does not exist until the captures have run. This one is the opposite. Its input is the
    // medium exactly as it was found, and continuing a foreign medium is wrong before a
    // single row is read — the capture would resume from ANOTHER node's sequence numbers and
    // silently skip every event of ours below them. Refusing first also means the refusal
    // costs no database work and can say, truthfully, that nothing was even read.
    refuse_foreign_continuation(continued_claim.as_ref(), &self_id_hex, medium_path)?;

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
    let staged_health = crate::medium::assess(v3_of(&staged_image, STAGED)?);
    // The UNKNOWN-PLANE case gets its own refusal, ahead of the general one, because its
    // REMEDY is the opposite of every other unsound medium's. A plane tag this build does not
    // recognise was written by a NEWER Cairn; `MediumHealth::needs_a_newer_build`'s own doc
    // says it plainly — *"the remedy is upgrade this node, never fetch another copy, and
    // never run the backup again: appending to it would write against an incomplete picture."*
    // The generic message below would send an operator to `--to` a new path, abandoning a
    // perfectly good medium over a plane that is only unreadable HERE.
    if staged_health.needs_a_newer_build() {
        anyhow::bail!(
            "refusing to append to {}: it carries {} record(s) in a plane this build does not \
             recognise, so it was written by a NEWER Cairn and this build cannot see all of \
             it. Appending here would write against an incomplete picture. NOTHING was \
             written and the medium is untouched. The remedy is to UPGRADE THIS NODE — not to \
             start a new medium, and not to fetch another copy: the medium is fine, this \
             build is behind it.",
            medium_path.display(),
            staged_health.records_in_unknown_planes
        );
    }
    refuse_unsound(
        &staged_health,
        STAGED,
        "NOTHING was written and the previous medium is untouched. Point --to at a NEW path: \
         the first capture of a fresh medium sweeps both planes from the beginning, so the \
         successor holds everything this one did.",
    )?;
    // THE LEGACY-SUCCESSION GUARD, and why it sits HERE rather than in
    // `open_or_start_medium` (#500 slice 2c final review, Critical 1). Succeeding a legacy
    // medium is the one path in this function that DESTROYS an artifact, and its safety
    // argument — "the successor is a strict superset" — is a claim about the staged image,
    // which does not exist until the two captures above have run. So the check cannot live
    // where the decision to supersede is made; it lives at the last moment before the write,
    // with both inputs in hand. Note that soundness above does NOT subsume it: an empty
    // successor is perfectly sound (0 of 0 signatures intact, an intact chain over no
    // segments, no torn tail), which is exactly how this could destroy a clinic's only medium
    // and report success.
    if let Some(legacy) = &superseded {
        refuse_unsafe_legacy_succession(
            legacy,
            plane_counts(&staged_image).node,
            &self_id_hex,
            medium_path,
        )?;
    }
    // PEAK MEMORY. `parse_any` copies every record's `signed_bytes` into an owned `Vec`, so a
    // parsed image costs roughly what the file costs. Nothing below reads `staged_image`, and
    // holding it across the write and the read-back would put four medium-sized allocations
    // (`buffer`, `staged_image`, `readback`, `written_image`) alive at once — on a Pi-class
    // node (8 GB, the Bet B target) a 2 GB medium then OOM-kills the nightly backup. The
    // previous medium survives that, but the clinic silently stops backing up and only
    // `describe_health`'s staleness ever says so. Two explicit drops keep the peak at two.
    drop(staged_image);

    // STEP 4.
    crate::fsio::atomic_write(medium_path, &buffer, Some(0o600))
        .with_context(|| format!("writing backup medium to {}", medium_path.display()))?;
    drop(buffer); // see the peak-memory note above: nothing below reads it.

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
    //
    // The SAME read also carries forward `extra` — any field a NEWER build recorded here.
    // `backup_to` rewrites the sidecar whole, so without this an older binary run once would
    // erase a newer build's field even though the export ceremony downstream preserves it.
    // One read, both facts, so the two can never come to disagree (#522's lesson).
    let previous = read_health(health_path);
    let previous_export_covers_seq = previous.as_ref().and_then(|h| h.export_covers_seq);
    let carried_extra = previous.map(|h| h.extra).unwrap_or_default();

    let health = BackupHealth {
        version: SUPPORTED_HEALTH_VERSION,
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
        extra: carried_extra,
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
        repaired_torn_tail,
        // From the DURABLE medium, like every other number here — never from "we appended
        // nothing", which would also be true of a perfectly good unchanged medium.
        carries_nothing: durable_health.carries_nothing(),
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
            extra: Default::default(),
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
            extra: Default::default(),
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

    // -----------------------------------------------------------------------
    // `clinical_plane_records` — the restore path's reader (#554 slice 2d, design §5.1).
    //
    // These are ADAPTER tests, deliberately. The gate, the sort and the duplicate collapse
    // are `cairn_medium::plane_records`' contract and are tested there, once. What is
    // testable HERE is the only thing this function adds: that each medium revision maps to
    // the right call, and — critically — that the V3 arm delegates rather than re-deriving.
    // A second derivation that sorted but did not gate on `verified_through` is exactly what
    // the design review caught, and `equals_the_shared_derivation` below is what stops one
    // growing back.
    // -----------------------------------------------------------------------

    /// A CAIRNB3 image holding one UNSIGNED clinical segment.
    ///
    /// Unsigned is deliberate and is what makes this fixture usable without a signing key:
    /// `chain_report` treats an unsigned segment as chain-verified (an unavailable key must
    /// never block a backup — it is simply not tamper-evident), so `verified_through` covers
    /// it and the records are servable. Soundness is a different question and not one these
    /// adapter tests ask.
    fn clinical_image(seqs: &[i64]) -> MediumImage {
        let seg = crate::medium::Segment {
            plane: Plane::Clinical,
            index: 0,
            prev_commitment: String::new(),
            self_node_id_hex: String::new(),
            attestation: None,
            records: seqs
                .iter()
                .map(|&source_seq| crate::medium::MediumRecord {
                    // Not a real signed event: nothing in this function verifies a
                    // signature. Derived rather than written out, per house rule 6.
                    signed_bytes: (0..24u8)
                        .map(|i| i.wrapping_add(source_seq as u8))
                        .collect(),
                    attestation: None,
                    attester_key: None,
                    dek_wrapped: None,
                    source_seq,
                })
                .collect(),
        };
        let bytes = crate::medium::serialize_v3(std::slice::from_ref(&seg)).unwrap();
        crate::medium::parse_any(&bytes).unwrap()
    }

    /// The V3 arm IS the shared derivation, not a lookalike of it.
    ///
    /// Asserting equality against `chain::plane_records` for the same image is what makes
    /// this an adapter rather than a second reader: a re-implementation that dropped the
    /// `verified_through` gate would still return "the clinical records, sorted" and would
    /// pass any test written in terms of the records alone.
    #[test]
    fn clinical_plane_records_equals_the_shared_derivation() {
        let image = clinical_image(&[70, 7, 40]);
        let MediumImage::V3(ref m) = image else {
            panic!("serialize_v3 must produce a CAIRNB3 image");
        };
        let report = crate::medium::chain_report(m);
        assert_eq!(
            clinical_plane_records(&image).unwrap(),
            crate::medium::plane_records(m, &report, Plane::Clinical),
            "the adapter must delegate; a second derivation loses the trust gate"
        );
        assert_eq!(
            clinical_plane_records(&image)
                .unwrap()
                .iter()
                .map(|r| r.source_seq)
                .collect::<Vec<_>>(),
            vec![7, 40, 70],
            "anti-vacuity: the delegation must actually be producing the sorted set"
        );
    }

    /// A legacy medium has no clinical plane AT ALL, and this returns empty for it.
    ///
    /// Empty is the truthful answer here — CAIRNB1/B2 predate the plane split, so there is
    /// no clinical record on such a medium to fail to read. It is NOT a truthful thing to
    /// show an operator on its own: "restored, 0 clinical events" is #500's exact signature
    /// reproduced inside the machinery built to close it. Naming that outcome is the CALLER's
    /// job (design §5.2), which is why this function is allowed to be silent about it.
    #[test]
    fn clinical_plane_records_of_a_legacy_medium_is_empty() {
        let container = crate::medium::Container {
            self_marker: None,
            events: vec![vec![1, 2, 3]],
        };
        let image = MediumImage::Legacy(container);
        assert!(clinical_plane_records(&image).unwrap().is_empty());
        assert_eq!(
            node_plane_events(&image).unwrap().len(),
            1,
            "and the federation plane is untouched by this slice: every legacy event IS \
             the federation plane"
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

    // -----------------------------------------------------------------------
    // The legacy-succession guard (#500 slice 2c final review, Critical 1). PURE, so these
    // run with no Postgres at all — the end-to-end pins in
    // `tests/backup_carries_both_planes.rs` self-skip without a database, and the one path
    // in this file that DESTROYS an artifact should not be unguarded on a developer machine
    // that has not started one.
    // -----------------------------------------------------------------------

    /// A node-id-shaped value, derived rather than written out — the real thing is the
    /// 32-byte content-address of a genesis, and a short word could never occur.
    fn node_id_hex(lineage: u8) -> String {
        hex::encode(std::array::from_fn::<u8, 32, _>(|i| {
            lineage.wrapping_add(i as u8)
        }))
    }

    // ---------------------------------------------------------------------------
    // The CAIRNB3 CONTINUATION guard (#500 slice 2c final review, Critical 1). The legacy
    // arm above has had an identity check since round 1; the arm that runs EVERY NIGHT had
    // none, and it is the more dangerous of the two because it fails SILENTLY rather than
    // destructively — see `refuse_foreign_continuation`'s doc for the worked seq arithmetic.

    /// The normal case, and the anti-vacuity guard for every refusal below: our own medium,
    /// by either derivation, is continued without complaint.
    #[test]
    fn continuing_our_own_cairnb3_medium_is_allowed() {
        let us = node_id_hex(1);
        for source in [MarkerSource::V3Attested, MarkerSource::V3Plaintext] {
            refuse_foreign_continuation(Some(&(us.clone(), source)), &us, Path::new("/mnt/usb/m"))
                .unwrap_or_else(|e| {
                    panic!("the nightly path must not be refused ({source:?}): {e:#}")
                });
        }
    }

    /// A medium naming ANOTHER node is refused before a single record is captured — because
    /// continuing it silently omits exactly the events whose seq numbers the other node
    /// already spent, and every downstream surface then agrees the backup is complete.
    #[test]
    fn a_cairnb3_medium_naming_another_node_is_refused_with_a_remedy() {
        let err = refuse_foreign_continuation(
            Some(&(node_id_hex(2), MarkerSource::V3Attested)),
            &node_id_hex(1),
            Path::new("/mnt/usb/m"),
        )
        .expect_err("another node's medium must never be appended to");
        let msg = format!("{err:#}");
        assert!(
            msg.contains(&node_id_hex(2)),
            "the refusal must name whose medium it is: {msg}"
        );
        assert!(
            msg.contains("--to at a NEW path"),
            "and a remedy the operator can act on tonight: {msg}"
        );
        assert!(
            msg.contains("NOTHING was written"),
            "and it must say the medium is untouched — an operator who thinks a partial \
             backup landed will reach for the wrong recovery: {msg}"
        );
    }

    /// A FORGEABLE claim still refuses. Refusing on an untrusted id is the fail-safe
    /// direction: the worst a forged foreign id can do is cost tonight's backup, loudly and
    /// with a remedy, whereas TRUSTING one is how a foreign medium gets appended to. The
    /// message must not describe a plaintext id as proof, though — that is the `V3Plaintext`
    /// half of the trust statement `MarkerSource` exists to carry.
    #[test]
    fn a_forgeable_plaintext_claim_still_refuses_but_says_it_is_unverified() {
        let err = refuse_foreign_continuation(
            Some(&(node_id_hex(2), MarkerSource::V3Plaintext)),
            &node_id_hex(1),
            Path::new("/mnt/usb/m"),
        )
        .expect_err("a mismatch must refuse whatever its provenance");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("unverified") || msg.contains("UNVERIFIED"),
            "a plaintext id must never be presented as proof: {msg}"
        );
    }

    /// Silence is not a mismatch (principle 4). A medium that names nobody — a capture taken
    /// before this node was enrolled — claims nothing, and refusing on an absent claim would
    /// block the legitimate first backup of a node that enrolled after its first capture.
    #[test]
    fn a_medium_that_names_nobody_is_not_refused() {
        refuse_foreign_continuation(None, &node_id_hex(1), Path::new("/mnt/usb/m"))
            .expect("an absent claim is not a foreign claim");
    }

    /// An UNENROLLED database continuing a medium that names a node gets the specific advice,
    /// not the generic mismatch text: the shape here is a died disk, a re-`init`, and
    /// `backup` run before `restore`. Appending would leave the medium's own events
    /// permanently uncaptured while the file grew and reported success.
    #[test]
    fn an_unenrolled_database_is_told_to_restore_before_it_continues() {
        let err = refuse_foreign_continuation(
            Some(&(node_id_hex(3), MarkerSource::V3Attested)),
            "",
            Path::new("/mnt/usb/m"),
        )
        .expect_err("an unenrolled node must not append to an identified medium");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("NOT ENROLLED"),
            "the unenrolled case needs its own diagnosis: {msg}"
        );
        assert!(
            msg.contains("restore"),
            "and the remedy is to restore FROM this medium, not to back up onto it: {msg}"
        );
    }

    /// Case-insensitivity, matching the legacy arm: hex ids differing only in case name the
    /// SAME node, and refusing them would break the nightly path on a cosmetic difference.
    #[test]
    fn a_claim_differing_only_in_hex_case_is_us() {
        let us = node_id_hex(1);
        refuse_foreign_continuation(
            Some(&(us.to_uppercase(), MarkerSource::V3Attested)),
            &us,
            Path::new("/mnt/usb/m"),
        )
        .expect("hex case must not decide identity");
    }

    #[test]
    fn succeeding_our_own_legacy_medium_is_allowed_when_the_successor_holds_at_least_as_much() {
        let us = node_id_hex(1);
        let legacy = SupersededLegacy {
            claimed_node_hex: Some(us.clone()),
            node_events: 7,
        };
        // Equal is fine (an unchanged log), and more is fine (the log grew since).
        for staged in [7, 9] {
            refuse_unsafe_legacy_succession(&legacy, staged, &us, Path::new("/mnt/usb/m"))
                .unwrap_or_else(|e| {
                    panic!("the normal upgrade path must not be refused (staged={staged}): {e:#}")
                });
        }
    }

    #[test]
    fn a_legacy_medium_naming_another_node_is_refused_with_a_remedy() {
        let err = refuse_unsafe_legacy_succession(
            &SupersededLegacy {
                claimed_node_hex: Some(node_id_hex(2)),
                node_events: 3,
            },
            // Deliberately NOT short: the count arm must not be what refuses this.
            99,
            &node_id_hex(1),
            Path::new("/mnt/usb/m"),
        )
        .expect_err("another node's medium must never be replaced");
        let msg = format!("{err:#}");
        assert!(
            msg.contains(&node_id_hex(2)),
            "the refusal must name whose medium it is: {msg}"
        );
        assert!(
            msg.contains("--to at a NEW path"),
            "and a remedy the operator can act on tonight: {msg}"
        );
    }

    #[test]
    fn an_unenrolled_database_is_told_to_restore_before_it_backs_up() {
        // The disaster shape: `read_self_node_id` answers `None`, so `self_id_hex` is empty.
        // Getting this wrong destroys the only copy, so the advice has to be specific.
        let err = refuse_unsafe_legacy_succession(
            &SupersededLegacy {
                claimed_node_hex: Some(node_id_hex(3)),
                node_events: 3,
            },
            0,
            "",
            Path::new("/mnt/usb/m"),
        )
        .expect_err("an un-enrolled node must not replace a medium it cannot re-sweep");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("NOT ENROLLED") && msg.contains("`restore`"),
            "an operator mid-disaster must be told to restore FIRST, not merely that this \
             failed: {msg}"
        );
    }

    #[test]
    fn a_successor_holding_fewer_events_is_refused_even_when_nobody_is_named() {
        // A CAIRNB1 medium claims no node at all, so arm 1 cannot fire — this is the only
        // thing standing between an emptied database and the clinic's last copy.
        let err = refuse_unsafe_legacy_succession(
            &SupersededLegacy {
                claimed_node_hex: None,
                node_events: 12,
            },
            0,
            "",
            Path::new("/mnt/usb/m"),
        )
        .expect_err("a successor holding less must never replace its predecessor");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("12 federation event(s)") && msg.contains("only 0"),
            "the refusal must quote both counts so the shortfall is visible: {msg}"
        );
    }

    #[test]
    fn a_marker_naming_us_in_a_different_case_is_not_a_mismatch() {
        // `local_node` renders hex lowercase and a marker carries whatever was written into
        // it. Refusing over letter case alone would block a legitimate upgrade at the worst
        // possible moment, so the compare is case-insensitive.
        let us = node_id_hex(4);
        refuse_unsafe_legacy_succession(
            &SupersededLegacy {
                claimed_node_hex: Some(us.to_ascii_uppercase()),
                node_events: 1,
            },
            1,
            &us,
            Path::new("/mnt/usb/m"),
        )
        .expect("case is not identity");
    }

    #[test]
    fn legacy_claimed_node_reads_an_unsigned_marker_and_stays_silent_without_one() {
        let id = node_id_hex(5);
        let named = crate::medium::Container {
            self_marker: Some(SelfMarker::Unsigned(id.to_ascii_uppercase())),
            events: vec![],
        };
        assert_eq!(
            legacy_claimed_node(&named),
            Some(id),
            "an unsigned marker is the claim, normalised to lowercase"
        );
        let anonymous = crate::medium::Container {
            self_marker: None,
            events: vec![],
        };
        assert_eq!(
            legacy_claimed_node(&anonymous),
            None,
            "a CAIRNB1 medium claims nothing, and silence is not a mismatch (principle 4)"
        );
    }
}
