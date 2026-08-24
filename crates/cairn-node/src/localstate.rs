//! ADR-0026 slice D — the sealed local-state export (container shape).
//!
//! WHY THIS EXISTS: ADR-0026 point 3 requires a node's NON-EVENT, non-signing-key
//! material — the data-at-rest keystore (node-default DEKs + sealed-episode DEKs),
//! node config, and the draft/scratchpad store — to be exportable as an encrypted
//! bundle co-located with the cold-peer backup medium, so a dead disk does not lose
//! it. The signing key is DELIBERATELY EXCLUDED (point 4): a stolen, unsealed artifact
//! must yield read access, never a signing identity.
//!
//! SCOPE (slice D): this module builds the can't-retrofit SHAPE — the format, the
//! dual-recipient secret lifecycle (a long-lived local-state DEK dual-wrapped once at
//! provisioning), the container, and the restore path — with typed slots the clinical tier
//! fills later via additive evolution (principle 11). **Two of those slots are no longer
//! empty** — see the state of play below; this paragraph is left otherwise as written
//! because its own expiry is the lesson recorded under it. The genuine
//! day-one piece is `establish_lsk`: state accrued before the channel exists has no
//! durability path, so the channel must exist from `init`.
//!
//! # History of this header, because it is the lesson (#495)
//!
//! The scope paragraph above once ended *"the federation-node tier has no clinical surface
//! yet, so the bundle is EMPTY today"*. That was true when slice D was written and
//! **expired silently** when ADR-0052 made every clinical body born-sealed: the node began
//! holding real `event_dek` custody while these slots stayed empty, and because restore
//! mints a fresh signing seed (ADR-0026 decision 4) from which the X25519 unwrap secret was
//! then HKDF-derived (ADR-0052 decision 4), **every born-sealed body on a restored SOLO
//! node was unopenable**. A document whose stated precondition had expired, believed for
//! months. Read the paragraph above with that in mind before adding to it.
//!
//! # What is closed, and what is still open — state of play
//!
//! **CLOSED (#495 / ADR-0066 — see `docs/spec/decisions/`, ADR number 0066, "Identity dies
//! with the disk; custody must not").** The node's unwrap key is now an INDEPENDENT X25519
//! keypair (decision 1) living in its own `<key>.unwrap` keystore file, and it rides this
//! export beside the custody rows (decision 3): [`LocalState::unwrap_secret`] carries the
//! secret, [`LocalState::episode_deks`] carries the wrapped `event_dek` rows minus every
//! target in `erasure_shred_log` (decision 7). The producer is
//! [`crate::localstate_read::read_local_state`].
//!
//! Read that heading narrowly: what is closed is the **EXPORT half**. "Identity dies with
//! the disk; custody must not" is the ADR's title, not yet this system's behaviour — the
//! restore half below is still owed, so custody currently leaves the dying node and is then
//! refused on arrival.
//!
//! **STILL OPEN, do not read this module as closing them:**
//!
//! - **The backup medium carries no clinical event (#500).** It exports the federation
//!   plane only. Until that lands, a restored node gets a working key and nothing to open
//!   with it — neither half is useful alone. This is the next slice.
//! - **The restore side does not land custody yet.** [`apply_local_state`] still refuses a
//!   non-empty bundle rather than installing it — loudly, which is correct, but it means the
//!   export currently writes what the restorer cannot yet apply. #495's restore half.
//! - **Promise 2 has no subject.** [`LocalState::node_default_deks`] stays empty because no
//!   node-default data-at-rest keystore exists anywhere in the built system. That slot's
//!   emptiness is neither honoured nor violated; it names a tier that must exist first.
//!
//! `crates/cairn-node/tests/dr_clinical_guarantee_gap.rs` holds the guards for all of the
//! above, and says of each whether it asserts a guarantee or pins a surviving defect.

use crate::seal::{
    self, aead_decrypt, aead_encrypt, normalize_recovery_code, rand_bytes, ArgonParams, Wrap,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
// `Zeroizing` wipes the freshly-minted LSK on drop (issue #54), matching the convention
// in `seal.rs`. The LSK recovered inside `seal_local_state`/`unseal_local_state_*` is
// already `Zeroizing` because `seal::try_unwrap` now returns it wrapped.
use zeroize::Zeroizing;

/// Magic for the `.lsk` sidecar (the dual-wrapped LSK). 8 bytes, like CAIRNK1/CAIRNB1.
const SIDECAR_MAGIC: &[u8] = b"CAIRNX1\n";
/// Magic for the export container (the sealed local-state bundle).
const CONTAINER_MAGIC: &[u8] = b"CAIRNL1\n";

#[derive(thiserror::Error, Debug)]
pub enum LocalStateError {
    /// The bytes are not a valid bundle / container / sidecar (bad magic or malformed body).
    #[error("decode: {0}")]
    Decode(String),
    /// A sealing/unsealing step failed (wrong secret, tamper, or entropy failure).
    /// Reachable from `establish_lsk`, `seal_local_state`, and their callers.
    #[error("seal: {0}")]
    Seal(String),
    // NOTE: no `Io` variant — no `localstate` function does file I/O (reads happen in
    // `main.rs` via `anyhow`). Adding it here would be YAGNI; add it when a function
    // here actually touches the filesystem.
}

/// The highest bundle `version` this build understands. A bundle declaring a higher
/// version must be REFUSED, not partially applied — see [`from_cbor`].
pub const SUPPORTED_LOCAL_STATE_VERSION: u8 = 1;

/// The node-local material ADR-0026 point 3 exports. The leaf type is opaque `Vec<u8>` so
/// we reserve the SLOT SHAPE without committing to the clinical tier's internal schema (no
/// speculative generality).
///
/// **Which slots carry something, as of #495 / ADR-0066** — read this before writing a
/// comment that calls the bundle empty, because two earlier comments here said exactly that
/// and both went stale (the module header has the history):
///
/// - [`Self::episode_deks`] and [`Self::unwrap_secret`] are **FILLED** on a provisioned node
///   by [`crate::localstate_read::read_local_state`]. Together they are the custody that
///   survives a dead disk.
/// - [`Self::node_default_deks`], [`Self::config`] and [`Self::drafts`] are still empty,
///   legitimately: none of the three has a store anywhere in the built system to be filled
///   from. Their emptiness is "nothing exists yet", not "we forgot".
///
/// The signing key is DELIBERATELY ABSENT (ADR-0026 point 4): a stolen, unsealed export
/// must grant read access, never a signing identity. Do not add it here. An unwrap secret is
/// read access and so belongs; a signing seed is an identity and does not (ADR-0066
/// decision 2 restates this and keeps the boundary exactly where ADR-0026 drew it).
///
/// `serde(default)` on every content field makes this ADDITIVELY evolvable (principle 11):
/// a bundle written before a field existed still deserializes, with that field defaulted.
///
/// `deny_unknown_fields` is the OTHER half of that contract, and the fix for review finding
/// A7c: without it, a bundle written by a NEWER cairn-node carrying a content-bearing field
/// this build doesn't know (e.g. a future clinical-tier `episode_deks` variant) would have
/// that field SILENTLY DROPPED on read, and `is_empty()` would call the restore a success
/// while quietly discarding key material — the exact failure the format is "can't-retrofit"
/// to guard against. With this, an unknown field is a LOUD refusal instead. `default` (for
/// missing fields) and `deny_unknown_fields` (for extra fields) are orthogonal and compose:
/// an OLDER bundle still deserializes, a NEWER one is refused rather than silently lossy.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalState {
    /// Bundle format version (bump only on a NON-additive change, which we avoid).
    /// NOT `#[serde(default)]`: absence of a version is always a malformed bundle —
    /// we must refuse it rather than silently assume v1.
    pub version: u8,
    /// Node-default data-at-rest keys. Empty — and no store exists to fill them from
    /// (#495: promise 2 has no subject; see `dr_clinical_guarantee_gap.rs`).
    #[serde(default)]
    pub node_default_deks: Vec<Vec<u8>>,
    /// Sealed-episode DEKs — one [`EpisodeDek`] per surviving `event_dek` row, CBOR-encoded
    /// into this slot's opaque leaf type, minus every event named in `erasure_shred_log`
    /// (ADR-0026 point 6 / ADR-0066 decision 7). Each DEK travels **wrapped**, exactly as it
    /// sits in the database; [`Self::unwrap_secret`] is what opens them.
    #[serde(default)]
    pub episode_deks: Vec<Vec<u8>>,
    /// Node config blob. None today (no node config table exists yet).
    #[serde(default)]
    pub config: Option<Vec<u8>>,
    /// Draft / scratchpad store. Empty today (no draft store exists yet).
    #[serde(default)]
    pub drafts: Vec<Vec<u8>>,
    /// ADR-0066: this node's INDEPENDENT X25519 unwrap secret, so a restored node inherits
    /// custody of every body it also inherits. The signing key is still deliberately absent
    /// (ADR-0026 point 4): a stolen, unsealed export must yield READ access, never a
    /// signing identity — and an unwrap secret is exactly read access.
    #[serde(default)]
    pub unwrap_secret: Option<Vec<u8>>,
}

impl LocalState {
    /// The bundle's ZERO VALUE — the honest answer for a node that holds nothing to carry
    /// (a freshly-`init`ed node, or one whose custody tables are empty).
    ///
    /// It is one of exactly TWO producers of a `LocalState`; the other is
    /// [`crate::localstate_read::read_local_state`], which reads real custody out of the
    /// database and is the one that must filter erased events. That pairing is pinned by
    /// `dr_clinical_guarantee_gap.rs`, because a THIRD producer skipping the filter is how
    /// an erased body's key would travel.
    pub fn empty() -> Self {
        LocalState {
            version: 1,
            node_default_deks: Vec::new(),
            episode_deks: Vec::new(),
            config: None,
            drafts: Vec::new(),
            unwrap_secret: None,
        }
    }

    /// True iff the bundle carries no content at all.
    ///
    /// ⚠️ **Not a validity check, and never a success condition.** Two earlier comments here
    /// framed emptiness first as "the only valid state at this tier" and then as "the state
    /// a node in #495 is stuck in"; both aged badly. What it means now is narrow and stable:
    /// this node had nothing to export. On a provisioned node holding born-sealed bodies
    /// that answer is FALSE, and must be — see [`Self::episode_deks`].
    ///
    /// Every content slot participates, [`Self::unwrap_secret`] included: a bundle carrying
    /// only the secret still carries key material, and treating it as empty would let
    /// [`apply_local_state`] wave it through as a no-op.
    pub fn is_empty(&self) -> bool {
        self.node_default_deks.is_empty()
            && self.episode_deks.is_empty()
            && self.config.is_none()
            && self.drafts.is_empty()
            && self.unwrap_secret.is_none()
    }
}

/// Hand-written so the raw unwrap secret can never reach a log line, a panic message, or an
/// `assert_eq!` failure. `Debug` was DERIVED until #495, which was harmless while every slot
/// was empty and is not now: `{:?}` on a populated bundle would print this node's custody key
/// in full. Everything else is shown as usual — only the secret is redacted, and its presence
/// is still visible so a reader can tell "absent" from "hidden".
impl std::fmt::Debug for LocalState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalState")
            .field("version", &self.version)
            .field("node_default_deks", &self.node_default_deks)
            .field("episode_deks", &self.episode_deks)
            .field("config", &self.config)
            .field("drafts", &self.drafts)
            .field(
                "unwrap_secret",
                &self.unwrap_secret.as_ref().map(|_| "<redacted 32 bytes>"),
            )
            .finish()
    }
}

/// Wipe the raw unwrap secret when a bundle is dropped (issues #46/#54 — the convention this
/// module already follows for the LSK).
///
/// A `Vec<u8>` field rather than `Zeroizing<Vec<u8>>` because the field must round-trip
/// through `serde`/`ciborium` unchanged; wiping on drop gets the same protection without
/// asking the serializer to understand a wrapper type. Every bundle that ever held the secret
/// — the one the producer built, the one the restore path decoded — passes through here.
///
/// LIMITS, so nobody reads this as more than it is: a `Vec` that was reallocated while being
/// built leaves its earlier buffer behind, and `serde` makes its own copies during encode and
/// decode. Wiping is a real reduction in exposure, not an erasure guarantee (#508).
impl Drop for LocalState {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        if let Some(secret) = self.unwrap_secret.as_mut() {
            secret.zeroize();
        }
    }
}

/// One event's wrapped custody row, as it travels in [`LocalState::episode_deks`].
///
/// The slot's leaf type is opaque `Vec<u8>` by design (the container reserved the SLOT SHAPE
/// without committing to the clinical tier's schema), so each element is a small CBOR struct
/// rather than a format change: the container format is untouched by this type existing.
///
/// The DEK travels **wrapped**, exactly as it sits in `event_dek` — the export never holds
/// raw key material, and the separately-carried [`LocalState::unwrap_secret`] is what opens
/// it. `event_id` is the hyphenated UUID TEXT, matching how this crate carries every event
/// id (tokio-postgres's `uuid` feature is not enabled here).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpisodeDek {
    pub event_id: String,
    pub dek_wrapped: Vec<u8>,
}

/// Serialize one custody row for the export slot. Pure.
pub fn episode_dek_to_cbor(d: &EpisodeDek) -> Vec<u8> {
    let mut out = Vec::new();
    ciborium::into_writer(d, &mut out).expect("CBOR serialization of EpisodeDek cannot fail");
    out
}

/// Parse one custody row from the export slot. Errors, never panics — a restore reading a
/// bit-rotted or foreign element must degrade honestly rather than abort a node mid-recovery.
pub fn episode_dek_from_cbor(bytes: &[u8]) -> Result<EpisodeDek, LocalStateError> {
    ciborium::from_reader(bytes).map_err(|e| LocalStateError::Decode(e.to_string()))
}

/// Serialize a bundle to CBOR. Pure. (No magic header — the bundle is always carried
/// INSIDE a sealed container, which has its own magic; this is the plaintext that gets
/// encrypted.)
pub fn to_cbor(ls: &LocalState) -> Vec<u8> {
    let mut out = Vec::new();
    ciborium::into_writer(ls, &mut out).expect("CBOR serialization of LocalState cannot fail");
    out
}

/// Parse a bundle from CBOR. Errors (never panics) on a malformed body, an UNKNOWN field
/// (a newer-format bundle — `deny_unknown_fields`), or a `version` this build cannot fully
/// honour. All three refuse rather than silently drop content (review finding A7c).
pub fn from_cbor(bytes: &[u8]) -> Result<LocalState, LocalStateError> {
    let ls: LocalState =
        ciborium::from_reader(bytes).map_err(|e| LocalStateError::Decode(e.to_string()))?;
    if ls.version > SUPPORTED_LOCAL_STATE_VERSION {
        return Err(LocalStateError::Decode(format!(
            "local-state bundle version {} exceeds supported {} — this build may not understand \
             all of its content; upgrade cairn-node to restore it (refusing rather than dropping)",
            ls.version, SUPPORTED_LOCAL_STATE_VERSION
        )));
    }
    Ok(ls)
}

/// The dual-wraps of a long-lived local-state DEK (LSK), established ONCE at provisioning
/// (the can't-retrofit day-one piece). A random 32-byte LSK is wrapped under a KEK from the
/// operational passphrase AND a KEK from the recovery code; either secret recovers it.
/// This is the `.lsk` sidecar's payload. `Debug` is intentionally NOT derived (mirrors
/// `SealedKey`) so a stray `{:?}` cannot dump wrapped key material.
#[derive(Clone, Serialize, Deserialize)]
pub struct LskWraps {
    pub argon: ArgonParams,
    pub salt_op: [u8; 16],
    pub salt_rec: [u8; 16],
    pub wrap_op: Wrap,
    pub wrap_rec: Wrap,
}

/// A sealed local-state export: the stable LSK wraps PLUS this export's freshly-encrypted
/// bundle. Self-contained — an off-site restore needs only this (the recovery code unwraps
/// the LSK, which decrypts the payload). `Debug` deliberately not derived.
#[derive(Clone, Serialize, Deserialize)]
pub struct SealedLocalState {
    pub wraps: LskWraps,
    pub payload_nonce: [u8; 24],
    pub payload_ct: Vec<u8>,
}

/// Establish the long-lived local-state DEK and dual-wrap it. Called ONCE at provisioning
/// (`init`/`seal-key`/`establish-local-state-key`) when BOTH secrets are in hand. The LSK
/// itself is discarded after wrapping — every later export re-derives it from the op-pass.
/// Reuses `seal::wrap_dek` (the same audited Argon2id+AEAD wrap the signing key uses).
pub fn establish_lsk(op_pass: &str, recovery_code: &str) -> Result<LskWraps, LocalStateError> {
    let argon = ArgonParams::default();
    // The LSK is discarded after wrapping (every later export re-derives it from the
    // op-pass), so hold it in `Zeroizing` — it must not linger on the stack afterwards.
    let lsk = Zeroizing::new(rand_bytes::<32>().map_err(|e| LocalStateError::Seal(e.to_string()))?);
    let salt_op = rand_bytes::<16>().map_err(|e| LocalStateError::Seal(e.to_string()))?;
    let salt_rec = rand_bytes::<16>().map_err(|e| LocalStateError::Seal(e.to_string()))?;
    let wrap_op = seal::wrap_dek(&lsk, op_pass, &salt_op, &argon)
        .map_err(|e| LocalStateError::Seal(e.to_string()))?;
    // Normalize the recovery code so any spacing/case the human re-types still unseals.
    let wrap_rec = seal::wrap_dek(
        &lsk,
        &normalize_recovery_code(recovery_code),
        &salt_rec,
        &argon,
    )
    .map_err(|e| LocalStateError::Seal(e.to_string()))?;
    Ok(LskWraps {
        argon,
        salt_op,
        salt_rec,
        wrap_op,
        wrap_rec,
    })
}

/// Seal the current bundle for export: unwrap the LSK with the op-pass (the unattended,
/// runtime-available secret), then AEAD-encrypt the bundle under the LSK with a fresh nonce.
/// The wraps are carried through unchanged (stable across exports — ADR-0026 point 5).
/// Errors if the op-pass cannot unwrap the LSK (never seals under a wrong/garbage key).
pub fn seal_local_state(
    wraps: &LskWraps,
    op_pass: &str,
    bundle: &[u8],
) -> Result<SealedLocalState, LocalStateError> {
    let lsk = seal::try_unwrap(&wraps.wrap_op, op_pass, &wraps.salt_op, &wraps.argon).ok_or_else(
        || {
            LocalStateError::Seal(
                "operational passphrase did not unwrap the local-state key".into(),
            )
        },
    )?;
    let payload_nonce = rand_bytes::<24>().map_err(|e| LocalStateError::Seal(e.to_string()))?;
    let payload_ct = aead_encrypt(&lsk, &payload_nonce, bundle)
        .map_err(|_| LocalStateError::Seal("aead".into()))?;
    Ok(SealedLocalState {
        wraps: wraps.clone(),
        payload_nonce,
        payload_ct,
    })
}

/// Recover the bundle via the operational passphrase (re-export / self-verify path).
pub fn unseal_local_state_op(s: &SealedLocalState, op_pass: &str) -> Option<Vec<u8>> {
    let lsk = seal::try_unwrap(&s.wraps.wrap_op, op_pass, &s.wraps.salt_op, &s.wraps.argon)?;
    aead_decrypt(&lsk, &s.payload_nonce, &s.payload_ct)
}

/// Recover the bundle via the recovery code (the disaster-recovery path — the only
/// guaranteed-available secret after total disk loss). The code is normalized first.
pub fn unseal_local_state_rec(s: &SealedLocalState, recovery_code: &str) -> Option<Vec<u8>> {
    let lsk = seal::try_unwrap(
        &s.wraps.wrap_rec,
        &normalize_recovery_code(recovery_code),
        &s.wraps.salt_rec,
        &s.wraps.argon,
    )?;
    aead_decrypt(&lsk, &s.payload_nonce, &s.payload_ct)
}

/// Serialize a sealed export to magic-prefixed CBOR for the `CAIRNL1` sibling file. Pure.
pub fn serialize_container(s: &SealedLocalState) -> Vec<u8> {
    let mut out = CONTAINER_MAGIC.to_vec();
    ciborium::into_writer(s, &mut out).expect("CBOR serialization of SealedLocalState cannot fail");
    out
}

/// Parse a `CAIRNL1` container. Errors (never panics) on bad magic / malformed body.
pub fn parse_container(bytes: &[u8]) -> Result<SealedLocalState, LocalStateError> {
    let body = bytes
        .strip_prefix(CONTAINER_MAGIC)
        .ok_or_else(|| LocalStateError::Decode("missing CAIRNL1 magic".into()))?;
    ciborium::from_reader(body).map_err(|e| LocalStateError::Decode(e.to_string()))
}

/// Seal a bundle for export AND frame it as the on-disk `CAIRNL1` container, in one fallible
/// step. Combining the seal and the framing lets the `backup` caller treat the whole optional
/// export as a SINGLE degrade-on-error operation (warn + skip on failure, never abort backup).
/// Errors only if the op-pass cannot unwrap the LSK or AEAD fails — never frames a container
/// under a wrong/garbage key.
pub fn build_export_container(
    wraps: &LskWraps,
    op_pass: &str,
    bundle: &LocalState,
) -> Result<Vec<u8>, LocalStateError> {
    // `Zeroizing` is load-bearing here since #495, and it was not before. The bundle now
    // carries a RAW X25519 unwrap secret (`LocalState::unwrap_secret`), so this CBOR
    // plaintext is a full copy of it in the clear; without the wrapper it would be dropped
    // unwiped the moment `seal_local_state` returned, leaving the node's custody key
    // readable in freed heap for anything that later reads that memory (a core dump, a swap
    // file, a heap-spray). Same reasoning, and the same issues (#46/#54), that made
    // `seal::try_unwrap` return `Zeroizing` for the LSK.
    //
    // RESIDUAL, stated rather than implied: `to_cbor` builds its `Vec` by GROWING it, so any
    // reallocation during serialization frees an intermediate buffer that still holds part of
    // the secret and that nothing can reach to wipe. Wiping the final buffer is a real
    // reduction, not a guarantee — the guarantee needs a serializer writing into a
    // pre-sized zeroizing buffer. Tracked in #508.
    let plaintext = Zeroizing::new(to_cbor(bundle));
    let sealed = seal_local_state(wraps, op_pass, &plaintext)?;
    Ok(serialize_container(&sealed))
}

/// Serialize the LSK wraps to magic-prefixed CBOR for the `.lsk` sidecar. Pure.
pub fn serialize_sidecar(w: &LskWraps) -> Vec<u8> {
    let mut out = SIDECAR_MAGIC.to_vec();
    ciborium::into_writer(w, &mut out).expect("CBOR serialization of LskWraps cannot fail");
    out
}

/// Parse a `.lsk` sidecar. Errors on bad magic / malformed body.
pub fn parse_sidecar(bytes: &[u8]) -> Result<LskWraps, LocalStateError> {
    let body = bytes
        .strip_prefix(SIDECAR_MAGIC)
        .ok_or_else(|| LocalStateError::Decode("missing CAIRNX1 magic".into()))?;
    ciborium::from_reader(body).map_err(|e| LocalStateError::Decode(e.to_string()))
}

/// The export sibling for a backup medium: `<medium>.localstate` in the same directory,
/// so the operator carries ONE artifact off-site (ADR-0026 point 3 — "same artifact"). Pure.
pub fn localstate_path_for(medium: &Path) -> PathBuf {
    let mut name = medium
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(".localstate");
    medium.with_file_name(name)
}

/// The `.lsk` sidecar for a key file: `<key>.lsk`, sibling of the signing key. Pure.
pub fn lsk_sidecar_path_for(key: &Path) -> PathBuf {
    let mut name = key
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(".lsk");
    key.with_file_name(name)
}

/// The DB-reading producer, re-exported so every existing `localstate::read_local_state`
/// call site keeps resolving after the move.
///
/// It lives in [`crate::localstate_read`] rather than here because this module owns the
/// FORMAT (container, seal, slots) and was already past the project's 500-line file-size
/// GUIDELINE (a guideline, not a cap — `tests/patient_register_demographics.rs` records the
/// correction to the "house limit" phrasing); reading custody out of a database is a
/// different job with a different dependency.
pub use crate::localstate_read::read_local_state;

/// Apply a restored local-state bundle into a fresh node. Today this is a validated noop: it
/// asserts the bundle carries no content it cannot yet honour, rather than silently dropping
/// it. THIS IS THE SEAM the clinical tier extends: it must install the unwrap secret, load
/// DEKs into the keystore, restore config, and rehydrate drafts.
///
/// ⚠️ **#495, RESTORE HALF — and the export half has LANDED, so this refusal now fires in
/// practice.** [`read_local_state`] fills `episode_deks` and `unwrap_secret` on any
/// provisioned node, so the bundle reaching a restore is no longer empty and the `bail!`
/// below rejects it. That is loud rather than lossy, which is the correct failure direction
/// (silently discarding recovered key material is the outcome this whole area exists to
/// prevent) — but it means ADR-0026 decision 1 is NOT yet true end-to-end. What is still
/// owed here, per ADR-0066 decision 4: install the recovered secret and register its public
/// half FIRST (`node_unwrap_key` is a singleton whose registrar refuses a differing key, and
/// wrapping needs the public half present), then land the custody rows. The `bail!` must
/// stay until that exists.
pub async fn apply_local_state(
    _db: &tokio_postgres::Client,
    ls: &LocalState,
) -> anyhow::Result<()> {
    if !ls.is_empty() {
        anyhow::bail!(
            "restored local-state bundle carries content this node version cannot apply \
             (the clinical-tier apply seam is not built yet); refusing to silently drop it"
        );
    }
    Ok(())
}

/// The `status` local-state line. Pure (presence flags injected). Honest about BOTH the
/// day-one escrow (`.lsk` present) and whether an export has been written. Absent escrow is
/// the loud case — a node accruing real content without the channel would lose it on a dead disk.
pub fn describe_local_state(lsk_present: bool, export_present: bool) -> String {
    match (lsk_present, export_present) {
        (false, _) => {
            "no local-state escrow — run `cairn-node establish-local-state-key`".to_string()
        }
        (true, false) => {
            "escrow set (dual-recipient); no export yet — run `cairn-node backup`".to_string()
        }
        (true, true) => {
            "escrow set (dual-recipient); exported alongside the last backup".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_bundle_cbor_roundtrips() {
        let ls = LocalState::empty();
        let bytes = to_cbor(&ls);
        let back = from_cbor(&bytes).expect("an empty bundle must roundtrip");
        assert_eq!(back, ls, "roundtrip must recover the exact bundle");
        assert!(
            back.is_empty(),
            "a fresh node's bundle has no content today"
        );
    }

    #[test]
    fn from_cbor_rejects_garbage() {
        assert!(from_cbor(b"not a bundle").is_err());
    }

    #[test]
    fn older_bundle_without_a_later_field_defaults_it() {
        // Additive evolution (principle 11): a bundle serialized by an OLDER node that
        // lacks a field this node knows about must still deserialize, with the missing
        // field defaulted. We simulate "older" by constructing a ciborium Value::Map
        // omitting later fields, then serializing it to CBOR — encode a map missing `drafts`.
        let mut partial = std::collections::BTreeMap::new();
        partial.insert(
            "version".to_string(),
            ciborium::value::Value::Integer(1.into()),
        );
        // Intentionally omit node_default_deks/episode_deks/config/drafts.
        let val = ciborium::value::Value::Map(
            partial
                .into_iter()
                .map(|(k, v)| (ciborium::value::Value::Text(k), v))
                .collect(),
        );
        let mut bytes = Vec::new();
        ciborium::into_writer(&val, &mut bytes).unwrap();
        let back = from_cbor(&bytes).expect("a bundle missing later fields must still parse");
        assert!(back.is_empty(), "omitted collections default to empty");
    }

    /// Helper: encode a top-level CBOR map from (key, Value) pairs, as a newer/older writer would.
    fn encode_map(entries: Vec<(&str, ciborium::value::Value)>) -> Vec<u8> {
        let val = ciborium::value::Value::Map(
            entries
                .into_iter()
                .map(|(k, v)| (ciborium::value::Value::Text(k.to_string()), v))
                .collect(),
        );
        let mut bytes = Vec::new();
        ciborium::into_writer(&val, &mut bytes).unwrap();
        bytes
    }

    #[test]
    fn newer_bundle_with_unknown_field_is_refused_not_silently_dropped() {
        // Review fix A7c: a NEWER cairn-node adds a content-bearing field this build does
        // not know. `deny_unknown_fields` must make that a LOUD refusal — never a silent
        // drop that reports the restore a success while discarding (e.g.) episode DEKs.
        let bytes = encode_map(vec![
            ("version", ciborium::value::Value::Integer(1.into())),
            // A field from the future carrying real content this build cannot represent.
            (
                "episode_wrapped_deks_v2",
                ciborium::value::Value::Bytes(vec![1, 2, 3]),
            ),
        ]);
        let err = from_cbor(&bytes).expect_err("an unknown field must be refused, not dropped");
        assert!(
            matches!(err, LocalStateError::Decode(_)),
            "unknown field -> Decode error"
        );
    }

    #[test]
    fn bundle_version_beyond_supported_is_refused() {
        // A bundle declaring a version this build cannot fully honour must be refused rather
        // than partially applied (the version-gate half of the A7c contract).
        let bytes = encode_map(vec![(
            "version",
            ciborium::value::Value::Integer(((SUPPORTED_LOCAL_STATE_VERSION + 1) as i32).into()),
        )]);
        assert!(
            from_cbor(&bytes).is_err(),
            "a too-new version must be refused"
        );
    }

    const OP: &str = "op-pass";
    const REC: &str = "AB12C-D34EF";

    #[test]
    fn lsk_seal_then_unseal_via_both_recipients() {
        let wraps = establish_lsk(OP, REC).unwrap();
        let bundle = to_cbor(&LocalState::empty());
        let sealed = seal_local_state(&wraps, OP, &bundle).unwrap();
        // Either secret recovers the same plaintext bundle.
        assert_eq!(
            unseal_local_state_op(&sealed, OP).as_deref(),
            Some(bundle.as_slice())
        );
        assert_eq!(
            unseal_local_state_rec(&sealed, REC).as_deref(),
            Some(bundle.as_slice()),
            "the recovery code (off-node escrow) must unseal — the disaster-recovery path"
        );
    }

    #[test]
    fn lsk_unseal_rejects_wrong_secret_and_tamper() {
        let wraps = establish_lsk(OP, REC).unwrap();
        let sealed = seal_local_state(&wraps, OP, &to_cbor(&LocalState::empty())).unwrap();
        assert_eq!(
            unseal_local_state_op(&sealed, "nope"),
            None,
            "wrong op-pass => None"
        );
        assert_eq!(
            unseal_local_state_rec(&sealed, "ZZZZZ"),
            None,
            "wrong recovery code => None"
        );
        // Flip a byte of the payload ciphertext: AEAD tag must fail.
        let mut t = sealed.clone();
        t.payload_ct[0] ^= 1;
        assert_eq!(
            unseal_local_state_op(&t, OP),
            None,
            "tampered payload must fail unseal"
        );
        // The LSK wrap is where the key actually lives on disk (the real storage-attacker
        // target): a flipped wrap ciphertext must fail the unwrap's AEAD tag, not silently
        // recover a corrupted key.
        let mut t2 = sealed.clone();
        t2.wraps.wrap_op.ct[0] ^= 1;
        assert_eq!(
            unseal_local_state_op(&t2, OP),
            None,
            "tampered op-wrap must fail unseal"
        );

        let mut t3 = sealed.clone();
        t3.wraps.wrap_rec.ct[0] ^= 1;
        assert_eq!(
            unseal_local_state_rec(&t3, REC),
            None,
            "tampered rec-wrap must fail unseal"
        );
    }

    #[test]
    fn seal_local_state_needs_the_op_pass_to_unwrap_the_lsk() {
        // seal_local_state unwraps the LSK with the op-pass; a wrong op-pass cannot
        // unwrap it, so sealing must fail rather than silently produce a bundle under a
        // wrong/garbage key.
        let wraps = establish_lsk(OP, REC).unwrap();
        assert!(seal_local_state(&wraps, "wrong-op", &to_cbor(&LocalState::empty())).is_err());
    }

    use std::path::Path;

    #[test]
    fn container_roundtrips_and_has_magic() {
        let wraps = establish_lsk(OP, REC).unwrap();
        let sealed = seal_local_state(&wraps, OP, b"x").unwrap();
        let bytes = serialize_container(&sealed);
        assert!(
            bytes.starts_with(b"CAIRNL1\n"),
            "export container must carry CAIRNL1 magic"
        );
        let back = parse_container(&bytes).unwrap();
        assert_eq!(
            unseal_local_state_rec(&back, REC).as_deref(),
            Some(b"x".as_slice())
        );
    }

    #[test]
    fn build_export_container_frames_a_sealed_bundle_and_rejects_a_wrong_op_pass() {
        // The `backup` arm calls this as ONE fallible step it degrades on (warn + skip) so a
        // missing/wrong passphrase never aborts an already-complete event backup.
        let wraps = establish_lsk(OP, REC).unwrap();
        let bytes = build_export_container(&wraps, OP, &LocalState::empty())
            .expect("the right op-pass must seal + frame the export");
        assert!(
            bytes.starts_with(b"CAIRNL1\n"),
            "the built export must carry the container magic"
        );
        // The off-node recovery code still unseals the framed container to the empty bundle.
        let parsed = parse_container(&bytes).unwrap();
        let plaintext = unseal_local_state_rec(&parsed, REC).expect("recovery code must unseal");
        assert!(from_cbor(&plaintext).unwrap().is_empty());
        // A wrong op-pass cannot unwrap the LSK, so building fails rather than emitting a
        // container under a wrong/garbage key — this Err is exactly what drives the warn+skip.
        assert!(
            build_export_container(&wraps, "wrong-op", &LocalState::empty()).is_err(),
            "a wrong op-pass must fail the build, not produce a bad container"
        );
    }

    #[test]
    fn sidecar_roundtrips_and_has_magic() {
        let wraps = establish_lsk(OP, REC).unwrap();
        let bytes = serialize_sidecar(&wraps);
        assert!(
            bytes.starts_with(b"CAIRNX1\n"),
            "lsk sidecar must carry CAIRNX1 magic"
        );
        let back = parse_sidecar(&bytes).unwrap();
        // The recovered wraps still unseal an export sealed under the originals.
        let sealed = seal_local_state(&back, OP, b"y").unwrap();
        assert_eq!(
            unseal_local_state_op(&sealed, OP).as_deref(),
            Some(b"y".as_slice())
        );
    }

    #[test]
    fn parse_rejects_wrong_or_missing_magic() {
        assert!(parse_container(b"nope").is_err());
        assert!(parse_sidecar(b"nope").is_err());
        // A container's bytes are not a valid sidecar and vice-versa (distinct magics).
        let wraps = establish_lsk(OP, REC).unwrap();
        let container = serialize_container(&seal_local_state(&wraps, OP, b"z").unwrap());
        assert!(
            parse_sidecar(&container).is_err(),
            "a container must not parse as a sidecar"
        );
        // ...and the reverse: the invariant is bidirectional (distinct 8-byte magics),
        // so a sidecar's bytes must not parse as a container either.
        let sidecar = serialize_sidecar(&wraps);
        assert!(
            parse_container(&sidecar).is_err(),
            "a sidecar must not parse as a container"
        );
    }

    #[test]
    fn paths_are_deterministic_siblings() {
        assert_eq!(
            localstate_path_for(Path::new("/mnt/backup/cairn.medium")),
            Path::new("/mnt/backup/cairn.medium.localstate")
        );
        assert_eq!(
            lsk_sidecar_path_for(Path::new("/var/lib/cairn/node.key")),
            Path::new("/var/lib/cairn/node.key.lsk")
        );
    }

    #[test]
    fn describe_local_state_is_honest_about_escrow_and_export() {
        assert!(describe_local_state(false, false).contains("no local-state escrow"));
        assert!(describe_local_state(true, false).contains("escrow set"));
        assert!(describe_local_state(true, false).contains("no export yet"));
        assert!(describe_local_state(true, true).contains("exported"));
    }

    #[test]
    fn re_export_keeps_wraps_stable_but_refreshes_the_payload() {
        // ADR-0026 point 5 / Approach 1: the LSK (and thus its dual-wraps) is long-lived
        // across exports — only the payload re-encrypts. So two seals over the SAME wraps
        // must carry byte-identical wrap_op/wrap_rec (the recovery code still unseals both)
        // but DIFFERENT payload ciphertext (fresh nonce), and each unseals to its own bundle.
        let wraps = establish_lsk(OP, REC).unwrap();
        let a = seal_local_state(&wraps, OP, b"bundle-A").unwrap();
        let b = seal_local_state(&wraps, OP, b"bundle-B").unwrap();
        assert_eq!(
            a.wraps.wrap_op.ct, b.wraps.wrap_op.ct,
            "LSK op-wrap is stable across exports"
        );
        assert_eq!(
            a.wraps.wrap_rec.ct, b.wraps.wrap_rec.ct,
            "LSK rec-wrap is stable across exports"
        );
        assert_ne!(
            a.payload_ct, b.payload_ct,
            "each export re-encrypts the payload (fresh nonce)"
        );
        assert_eq!(
            unseal_local_state_rec(&a, REC).as_deref(),
            Some(b"bundle-A".as_slice())
        );
        assert_eq!(
            unseal_local_state_rec(&b, REC).as_deref(),
            Some(b"bundle-B".as_slice())
        );
    }
}
