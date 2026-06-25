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

use serde::{Deserialize, Serialize};

#[derive(thiserror::Error, Debug)]
pub enum LocalStateError {
    /// The bytes are not a valid bundle / container / sidecar (bad magic or malformed body).
    #[error("decode: {0}")]
    Decode(String),
    /// A sealing/unsealing step failed (wrong secret, tamper, or entropy failure).
    #[error("seal: {0}")]
    Seal(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_bundle_cbor_roundtrips() {
        let ls = LocalState::empty();
        let bytes = to_cbor(&ls);
        let back = from_cbor(&bytes).expect("an empty bundle must roundtrip");
        assert_eq!(back, ls, "roundtrip must recover the exact bundle");
        assert!(back.is_empty(), "a fresh node's bundle has no content today");
    }

    #[test]
    fn from_cbor_rejects_garbage() {
        assert!(from_cbor(b"not a bundle").is_err());
    }

    #[test]
    fn older_bundle_without_a_later_field_defaults_it() {
        // Additive evolution (principle 11): a bundle serialized by an OLDER node that
        // lacks a field this node knows about must still deserialize, with the missing
        // field defaulted. We simulate "older" by serializing a struct with a subset of
        // fields via a serde_json->cbor shim: encode a map missing `drafts`.
        let mut partial = std::collections::BTreeMap::new();
        partial.insert("version".to_string(), ciborium::value::Value::Integer(1.into()));
        // Intentionally omit node_default_deks/episode_deks/config/drafts.
        let val = ciborium::value::Value::Map(
            partial.into_iter()
                .map(|(k, v)| (ciborium::value::Value::Text(k), v))
                .collect(),
        );
        let mut bytes = Vec::new();
        ciborium::into_writer(&val, &mut bytes).unwrap();
        let back = from_cbor(&bytes).expect("a bundle missing later fields must still parse");
        assert!(back.is_empty(), "omitted collections default to empty");
    }
}
