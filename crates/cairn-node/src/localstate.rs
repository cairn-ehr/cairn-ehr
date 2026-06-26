//! ADR-0026 slice D — the sealed local-state export (container shape).
//!
//! WHY THIS EXISTS: ADR-0026 point 3 requires a node's NON-EVENT, non-signing-key
//! material — the data-at-rest keystore (node-default DEKs + sealed-episode DEKs),
//! node config, and the draft/scratchpad store — to be exportable as an encrypted
//! bundle co-located with the cold-peer backup medium, so a dead disk does not lose
//! it. The signing key is DELIBERATELY EXCLUDED (point 4): a stolen, unsealed artifact
//! must yield read access, never a signing identity.
//!
//! SCOPE (slice D): the federation-node tier has no clinical surface yet, so the bundle
//! is EMPTY today. This module builds the can't-retrofit SHAPE — the format, the
//! dual-recipient secret lifecycle (a long-lived local-state DEK dual-wrapped once at
//! provisioning), the container, and the restore path — with typed empty slots the
//! clinical tier fills later via additive evolution (principle 11). The genuine
//! day-one piece is `establish_lsk`: state accrued before the channel exists has no
//! durability path, so the channel must exist from `init`.

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

/// The node-local material ADR-0026 point 3 exports. Every slot is EMPTY at the
/// federation-node tier (no clinical surface yet); the clinical tier fills them via
/// additive evolution. The leaf type is opaque `Vec<u8>` so we reserve the SLOT SHAPE
/// without committing to the clinical tier's internal schema (no speculative generality).
///
/// The signing key is DELIBERATELY ABSENT (ADR-0026 point 4): a stolen, unsealed export
/// must grant read access, never a signing identity. Do not add it here.
///
/// `serde(default)` on every content field makes this ADDITIVELY evolvable (principle 11):
/// a bundle written before a field existed still deserializes, with that field defaulted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalState {
    /// Bundle format version (bump only on a NON-additive change, which we avoid).
    /// NOT `#[serde(default)]`: absence of a version is always a malformed bundle —
    /// we must refuse it rather than silently assume v1.
    pub version: u8,
    /// Node-default data-at-rest keys. Empty today.
    #[serde(default)]
    pub node_default_deks: Vec<Vec<u8>>,
    /// Sealed-episode DEKs (minus any erased — ADR-0026 point 6). Empty today.
    #[serde(default)]
    pub episode_deks: Vec<Vec<u8>>,
    /// Node config blob. None today.
    #[serde(default)]
    pub config: Option<Vec<u8>>,
    /// Draft / scratchpad store. Empty today.
    #[serde(default)]
    pub drafts: Vec<Vec<u8>>,
}

impl LocalState {
    /// The empty bundle a federation-tier node exports today.
    pub fn empty() -> Self {
        LocalState {
            version: 1,
            node_default_deks: Vec::new(),
            episode_deks: Vec::new(),
            config: None,
            drafts: Vec::new(),
        }
    }

    /// True iff the bundle carries no content (the only valid state at this tier).
    pub fn is_empty(&self) -> bool {
        self.node_default_deks.is_empty()
            && self.episode_deks.is_empty()
            && self.config.is_none()
            && self.drafts.is_empty()
    }
}

/// Serialize a bundle to CBOR. Pure. (No magic header — the bundle is always carried
/// INSIDE a sealed container, which has its own magic; this is the plaintext that gets
/// encrypted.)
pub fn to_cbor(ls: &LocalState) -> Vec<u8> {
    let mut out = Vec::new();
    ciborium::into_writer(ls, &mut out).expect("CBOR serialization of LocalState cannot fail");
    out
}

/// Parse a bundle from CBOR. Errors (never panics) on a malformed body.
pub fn from_cbor(bytes: &[u8]) -> Result<LocalState, LocalStateError> {
    ciborium::from_reader(bytes).map_err(|e| LocalStateError::Decode(e.to_string()))
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
    let sealed = seal_local_state(wraps, op_pass, &to_cbor(bundle))?;
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

/// Read the node's exportable local state from the DB. At the federation-node tier there
/// is no clinical surface — no DEK store, no draft store, no config table — so this returns
/// the EMPTY bundle. THIS IS THE SEAM the clinical tier extends: it will read the keystore's
/// node-default + sealed-episode DEKs (minus erased — ADR-0026 point 6), node config, and
/// the draft/scratchpad store into the typed slots. The signing key is never read here
/// (point 4). Async + DB-handle so the future shape needs no signature change.
pub async fn read_local_state(_db: &tokio_postgres::Client) -> anyhow::Result<LocalState> {
    Ok(LocalState::empty())
}

/// Apply a restored local-state bundle into a fresh node. At this tier the bundle is empty,
/// so this is a validated noop — it asserts the bundle carries no content it cannot yet
/// honour, rather than silently dropping it. THIS IS THE SEAM the clinical tier extends: it
/// will load DEKs into the keystore, restore config, and rehydrate drafts. A non-empty
/// bundle here means a newer node wrote content an older restorer cannot apply — fail loud.
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
