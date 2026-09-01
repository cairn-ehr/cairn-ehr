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
//! KNOWN LIMITATION (CAIRNB2 ONLY) — the converged-peer splice (issue #53 follow-up). Every
//! sentence in this paragraph is about `event_set_commitment` and `verify_self_attestation`,
//! i.e. the CAIRNB2 head marker. CAIRNB3's counterpart residual is stated at the end of it.
//! The "never misdirect"
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
//! CAIRNB3's counterpart, stated rather than left to inference: `chain::self_id_from_chain`
//! binds a segment attestation to a genesis ON THIS MEDIUM signed by the same key, so a
//! converged peer's medium — which carries that peer's genesis — can still yield that peer's
//! id. The exposure is NARROWER than CAIRNB2's, and for a structural reason worth naming: a
//! CAIRNB2 marker commits to the whole event SET, which two converged peers hold identically,
//! whereas a CAIRNB3 segment commits to per-record `source_seq` — each node's own LOCAL
//! insertion order, which converged peers do NOT share. Two mutual peers therefore hold
//! byte-identical event sets but different segment commitments, so the CAIRNB2 splice does not
//! carry over unchanged. It is not proven impossible, and the same restore-time provenance
//! check plus physical custody remain the defences.
//!
//! This CRATE does no DB and no I/O (serialization, parsing, and signature checks only), so it
//! is trivially unit-testable and reusable by both the backup and restore paths. It is *mostly*
//! pure — [`build_self_attestation`] and [`build_segment_attestation`] are the two exceptions,
//! each minting a fresh `event_id` UUID (see their docs; the id is neither committed to nor
//! checked on verify, so two calls differing is harmless).
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
//! - `error` — the fault taxonomy. Read it before matching on a failure: "damaged",
//!   "written by a newer build" and "not a medium at all" have OPPOSITE remedies, and one
//!   opaque variant for all three is how an operator ends up discarding a good medium.
//! - `verify` — flat, whole-set signature verification: "do these bytes verify".
//! - `chain` — "does this medium's CHAIN hold": the chain pass over CAIRNB3 segments, the
//!   per-plane watermark and its gaps, and self-identification. Split out of `verify` — same
//!   seam as `attest`/`segment`: a responsibility boundary, not a line-count cut.
//! - `health` — **"is this medium sound", the composed verdict, and the one to reach for.**
//!   Every other verdict in this crate is partial and each returns `true` for some medium
//!   that is not sound; `health::assess` is the only one that cannot be read that way.
//! - `wire_pins` (test-only) — golden byte fixtures for every on-disk constant. The only
//!   tests here that can catch a MIRRORED change to the format, where writer and reader move
//!   together and every round-trip stays green.
//!
//! # The invariants, in one place
//!
//! A reader arriving at this crate needs the rules, not a tour of the modules above.
//!
//! 1. **CAIRNB1 and CAIRNB2 are frozen.** Media in the field are unaffected by this crate
//!    existing, forever. `parse_container`, `serialize_container`, `take_frames`, `put_marker`
//!    and `take_chunk` are byte-for-byte the code they were in `cairn-node`. Two functions in
//!    `marker` were touched, both semantics-preserving and both pinned by test:
//!    `event_set_commitment` now delegates to `commitment_over` (the hash PRE-IMAGE is
//!    identical — `attest`'s `event_set_commitment_is_unchanged_by_the_shared_helper` derives
//!    it independently at N=1 and N=2), and `build_self_attestation` imports the same
//!    `NIL_PATIENT` literal from `cairn-event`. Saying "byte-unchanged" of the whole module
//!    would be false, and a reader who believed it would never re-derive the equality that is
//!    the actual guarantee. The wire constants themselves are pinned in `wire_pins`.
//! 2. **CAIRNB3 is append-only.** `append_segment` writes bytes and reads none. Nothing
//!    already on a medium is ever rewritten.
//! 3. **Every SIGNED segment is chained and bound.** Its attestation binds its contents, its
//!    plane (both the numeric tag and, when known, the label), its position and its
//!    predecessor's commitment; a genuine segment replayed elsewhere in a chain, or spliced
//!    from another medium, fails. An UNSIGNED segment carries no attestation, so it is bound
//!    by its `prev_commitment` alone — a value anyone holding the preceding records can
//!    derive. Invariant 5 is where that weakness is stated in full; it is named here because
//!    an invariant list is read one line at a time, and the unqualified claim overstated it.
//!    Empty segments are refused at write, because an empty segment's commitment is the same
//!    constant on every medium and would let anything chaining off it be spliced in freely.
//! 4. **A torn tail never reads as corruption, but the reverse is not proven.** Fewer bytes
//!    than a section's length prefix claims always reads as an interrupted append: keep the
//!    complete prefix, flag the tail, re-capture — that direction is airtight, and a
//!    MALFORMED BODY under an honest length is damage, not a tear. A length prefix BEYOND the
//!    section cap is always corruption. But a corrupt length prefix UNDER the cap is
//!    INDISTINGUISHABLE from a genuine torn tail: a mid-file bit flip landing inside the
//!    length field reads as "your last backup was interrupted, run it again," and a naive
//!    re-run then appends after the damage, permanently orphaning everything between. A
//!    sentinel-based fix is deliberately NOT attempted here — filed as #523.
//! 5. **`verified_through` bounds STRUCTURAL loss, not tamper-evidence.** It marks the last
//!    segment whose chain link (and, when signed, attestation and self-id bind) held — the
//!    watermark is derived from it, never from the file's tail, which bounds the loss from
//!    any tail damage or break to one increment. It is NOT a tamper-evidence boundary: an
//!    UNSIGNED segment advances it on a matching `prev_commitment` alone (a value derivable
//!    from public bytes, since principle 7 forbids ever blocking a backup on a missing key),
//!    so anyone able to append a well-formed unsigned segment can advance the watermark with
//!    arbitrary `source_seq` and no tamper-evidence — deferred, not fixed, to the slice that
//!    adds the operator surface. `SelfIdUnbound` and `UnknownPlane` deliberately do NOT
//!    retract it; see `chain::ChainReport::verified_through`.
//! 6. **Nothing unrecognised is skipped in silence.** An unknown record flag bit is REFUSED
//!    (as `UnsupportedByThisBuild`, never as damage). An unknown PLANE tag is carried as a
//!    first-class `Plane::Unknown`, keeps all its records, chains normally, and is reported
//!    as a located `UnknownPlane` fault — so a newer Cairn's medium neither reads as damaged
//!    nor passes as fully readable. Dropping such a segment (as an earlier build did) broke
//!    the chain for every segment after it AND let a medium missing an entire plane report
//!    itself sound, which is this invariant's own stated failure shape.
//! 7. **Unsigned is a declared limitation, not a fault.** An unavailable signing key never
//!    blocks a backup. It travels flagged, and no caller may treat it as tamper-evident.
//! 8. **`None` is not zero.** A plane with no verified segment has no watermark. Zero is a
//!    claim; absence is the honest answer. And a watermark is a high-water MARK, not a
//!    completeness claim — `max` proves nothing about the gaps below it, so `chain::seq_gaps`
//!    reports those separately rather than letting `Some(N)` imply a contiguous run.
//! 9. **A fault is located, never merely counted** — and located by a coordinate the medium
//!    cannot lie about. Every `SegmentFault` carries the segment's `position` (where the
//!    reader found it) as well as its self-declared `index`, because on an unsigned segment
//!    that declared index is attacker-controlled: locating by it alone can send an operator
//!    to a segment that does not exist. A failing RECORD is likewise resolved from its flat
//!    ordinal back to `(position, plane, index, ordinal)` by `chain::locate_record`.
//! 10. **No partial verdict may be read as a whole one.** `chain_intact()`, `all_intact()`
//!     and `truncated_tail` each answer a fragment, and each returns a clean result for some
//!     medium that is not sound — an empty file, a medium missing a plane, a torn tail, a
//!     tampered record in the last unsigned segment. `health::assess` composes all of them
//!     and is the only verdict a caller should decide on. A medium that reports healthy while
//!     carrying nothing is issue #500 exactly, and this crate must not be able to say it.

mod attest;
mod chain;
mod chunk;
mod container;
mod error;
mod health;
mod marker;
mod record;
mod segment;
mod verify;

#[cfg(test)]
mod testkit;
#[cfg(test)]
mod wire_pins;

pub use attest::{
    build_segment_attestation, segment_commitment, verify_segment_attestation, SEGMENT_ATTEST_TYPE,
};
pub use chain::{
    chain_report, locate_record, self_id_from_chain, seq_gaps, verify_records, watermark,
    ChainReport, SegmentFault,
};
pub use container::{
    append_segment, parse_any, parse_container, parse_medium, serialize_container, serialize_v3,
    Container, MediumImage, MediumV3, MEDIUM_MAGIC_V1, MEDIUM_MAGIC_V2, MEDIUM_MAGIC_V3,
};
pub use error::BackupError;
pub use health::{assess, MediumHealth, RecordLocation};
pub use marker::{
    build_self_attestation, enrolls, event_set_commitment, scan_enrolls, verify_self_attestation,
    EnrollScan, SelfMarker, SELF_ATTEST_TYPE,
};
pub use record::MediumRecord;
pub use segment::{Plane, Segment};
pub use verify::{
    serialize_and_verify_container, serialize_and_verify_v3, verify_and_append_segment,
    verify_event, verify_events, verify_medium_bytes, VerifyReport,
};
