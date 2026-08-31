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
//!   CAIRNB3's equivalent is `segment`, because a whole-set commitment cannot
//!   survive an append (see the crate docs). Do not extend this module.
//! - `container` — magic dispatch and the on-disk framing of every revision.
//! - `verify` — signature verification, and (from slice 2a) the chain pass.

mod chunk;
mod container;
mod error;
mod marker;
mod verify;

#[cfg(test)]
mod testkit;

pub use container::{
    parse_container, parse_medium, serialize_container, Container, MEDIUM_MAGIC_V1, MEDIUM_MAGIC_V2,
};
pub use error::BackupError;
pub use marker::{
    build_self_attestation, enrolls, event_set_commitment, scan_enrolls, verify_self_attestation,
    EnrollScan, SelfMarker, SELF_ATTEST_TYPE,
};
pub use verify::{
    serialize_and_verify_container, verify_event, verify_events, verify_medium_bytes, VerifyReport,
};
