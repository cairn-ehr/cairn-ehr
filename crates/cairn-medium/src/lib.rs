//! The backup-medium container format (ADR-0026 slice B, issue #53).
//!
//! WHY A CRATE: `cairn-node` writes the federation plane and orchestrates restore, and
//! from slice 2b `cairn-sync` writes the clinical plane — it owns the clinical event
//! log, the wire protocol and the transport seam. Both need this format, and a
//! production dependency from `cairn-sync` onto `cairn-node` (an application crate
//! carrying clap, rustls, rcgen and tokio-postgres) is the wrong direction. Same shape
//! as `cairn-keystore`, extracted for the same reason in #503.
//!
//! Pure by construction: no database, no I/O, no async. Serialization, parsing and
//! signature checks only, so every property below is unit-testable without a fixture
//! larger than a byte slice.
//!
//! SCOPE TODAY: this crate carries the format. It does NOT read a database and does not
//! decide what goes on a medium — `cairn-node`'s `backup.rs` still reads `node_event`
//! and nothing else, which is issue #500 and is NOT fixed by this crate existing.
//!
//! The backup-medium container format and its self-marker (ADR-0026 slice B + issue #53).
//!
//! WHY A SELF-MARKER: a backup medium is a node's `node_event` set. By set-union sync that
//! set CONVERGES with every peer's — two fully-synced mutual peers hold byte-identical event
//! sets. So nothing *in the events* can say which node a given backup belongs to; on restore
//! we could not tell "self" from a peer, and would record a wrong, immutable supersede edge
//! and adopt a peer's name (issue #53). The fix is a marker written into the CONTAINER (not
//! the synced event stream) at backup time, when `local_node` still names self authoritatively.
//!
//! The marker is SIGNED when the node's key is available at backup, UNSIGNED otherwise — an
//! unsigned marker never blocks a backup, it just travels flagged for caution. The safety
//! asymmetry we want (mirrors "uncertainty can only withhold an auto-link"): tampering with a
//! SIGNED marker can only WITHHOLD (delete/corrupt → restore fails closed to a manual choice),
//! and an attacker holds no private key (the signing key is never backed up) so a *wrong*
//! self-attestation cannot be FORGED.
//!
//! KNOWN LIMITATION — the converged-peer splice (issue #53 follow-up). The "never misdirect"
//! property is NOT absolute. The medium bind ([`event_set_commitment`]) ties a marker to the
//! exact event SET it sits beside, which rejects a marker lifted from a backup with a *different*
//! set. But two fully-converged mutual peers hold BYTE-IDENTICAL event sets (that is the very
//! premise of this marker), so their commitments are identical too. An attacker who physically
//! holds a PEER's genuine cold medium can therefore splice that peer's valid signed marker onto
//! this one and `verify_self_attestation` cannot tell them apart — there is no signal in the
//! shared bytes that distinguishes the two media. The splice is IMPOSSIBLE on a sole-enroll
//! medium (a foreign marker would name an absent enroll → fail closed), so the residual risk is
//! exactly the multi-enroll / federated case. Its defences are not in this module: restore-time
//! provenance (`cairn_node::restore::Provenance::SignedFederated` → confirm the echoed name/address)
//! plus physical custody of the medium. So: forgery-proof always; misdirect-proof for sole-enroll
//! media and for splices from a *different* set; a peer-medium splice between converged peers is a
//! confirm-on-restore residual, not a silent misdirect.
//!
//! This module does no DB and no I/O (serialization, parsing, and signature checks only), so it
//! is trivially unit-testable and reusable by both the backup and restore paths. It is *mostly*
//! pure — [`build_self_attestation`] is the one exception (it mints a fresh UUID, see its docs).
//!
//! # Module map
//!
//! - `chunk` — the `[u32 BE len][bytes]` primitive every other module frames with.
//! - `marker` — **CAIRNB2 only, and frozen.** The head self-marker and its
//!   whole-set commitment. It serves media that already exist and gains nothing:
//!   CAIRNB3's equivalent is `segment` + `attest`, because a whole-set commitment cannot
//!   survive an append (see the crate docs). Do not extend this module.
//! - `record` — one event as the medium carries it. Tracks the sync WIRE shape; changes
//!   when `cairn-sync`'s `EventsResponse` shape changes.
//! - `segment` — the append-only, plane-tagged, chained GROUP of records. Tracks the
//!   append/chain durability design, a separate concern from `record` with a separate
//!   reason to change (see its module doc for why this, not `marker`, is CAIRNB3's
//!   appendable unit).
//! - `attest` — the signed, per-segment attestation: names the node and binds a
//!   segment's contents, plane, position and predecessor, so appending costs one
//!   signature instead of a whole-file rewrite. CAIRNB3's counterpart to `marker`'s
//!   whole-set self-attestation.
//! - `container` — magic dispatch and the on-disk framing of every revision.
//! - `verify` — flat, whole-set signature verification: "do these bytes verify".
//! - `chain` — (from slice 2a, task 8) "what can this whole medium be trusted for": the
//!   chain pass over CAIRNB3 segments, the per-plane watermark, and self-identification.
//!   Split out of `verify` in task 8 review (#500) — same seam as `attest`/`segment`: a
//!   responsibility boundary, not a line-count cut.
//!
//! # The invariants, in one place
//!
//! A reader arriving at this crate needs the rules, not a tour of the modules above.
//!
//! 1. **CAIRNB1 and CAIRNB2 are frozen.** They parse today exactly as they did before this
//!    crate existed, through untouched code (`container`, `marker`). Media in the field are
//!    unaffected, forever.
//! 2. **CAIRNB3 is append-only.** `append_segment` writes bytes and reads none. Nothing
//!    already on a medium is ever rewritten.
//! 3. **Every segment is chained.** Its attestation binds its contents, its plane, its
//!    position and its predecessor's commitment. A genuine segment replayed elsewhere in a
//!    chain, or spliced from another medium, fails.
//! 4. **A torn tail is not corruption.** Fewer bytes than a section claims means an
//!    interrupted append: keep the complete prefix, flag the tail, re-capture. An over-cap
//!    length prefix IS corruption. The two verdicts never collapse, because they send an
//!    operator to different places.
//! 5. **Trust stops at `verified_through`.** The watermark is derived from it, never from
//!    the file's tail, which bounds the loss from any tail damage to one increment.
//! 6. **Nothing unrecognised is skipped in silence.** An unknown plane tag is reported with
//!    its index and record count; an unknown record flag bit is REFUSED. A medium that
//!    parses cleanly while missing a plane is the exact failure shape #500 is about.
//! 7. **Unsigned is a declared limitation, not a fault.** An unavailable signing key never
//!    blocks a backup. It travels flagged, and no caller may treat it as tamper-evident.
//! 8. **`None` is not zero.** A plane with no verified segment has no watermark. Zero is a
//!    claim; absence is the honest answer.
//! 9. **A fault is located, never merely counted.** Every `SegmentFault` carries its plane
//!    and index — *"clinical segment 7 breaks the chain"* sends an operator somewhere,
//!    *"chain invalid"* does not.

mod attest;
mod chain;
mod chunk;
mod container;
mod error;
mod marker;
mod record;
mod segment;
mod verify;

#[cfg(test)]
mod testkit;

pub use attest::{
    build_segment_attestation, segment_commitment, verify_segment_attestation, SEGMENT_ATTEST_TYPE,
};
pub use chain::{
    chain_report, self_id_from_chain, verify_records, watermark, ChainReport, SegmentFault,
};
pub use container::{
    append_segment, parse_any, parse_container, parse_medium, serialize_container, serialize_v3,
    Container, MediumImage, MediumV3, MEDIUM_MAGIC_V1, MEDIUM_MAGIC_V2, MEDIUM_MAGIC_V3,
};
pub use error::BackupError;
pub use marker::{
    build_self_attestation, enrolls, event_set_commitment, scan_enrolls, verify_self_attestation,
    EnrollScan, SelfMarker, SELF_ATTEST_TYPE,
};
pub use record::MediumRecord;
pub use segment::{Plane, Segment, UnknownSegment};
pub use verify::{
    serialize_and_verify_container, verify_event, verify_events, verify_medium_bytes, VerifyReport,
};
