//! The ADR-0055 documents. Every type is a plain struct of structs, `Vec`s, `Option`s
//! and fixed arrays, so `ciborium`'s output is deterministic (field order = declaration
//! order). Never add a `HashMap` here: it would make the signed bytes unstable.

use crate::refusal::Refusal;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// One trusted signing key and a human note about where its private half lives.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Signer {
    #[serde(with = "serde_bytes")]
    pub kid: Vec<u8>,
    pub pubkey: [u8; 32],
    pub custody_note: String,
}

/// Who may sign roots and releases for a channel, at one monotonic `version`.
/// `prev_root_hash` chains it to the root before it (None only for genesis).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TrustRoot {
    pub channel_id: String,
    pub version: u64,
    pub root_signers: Vec<Signer>,
    pub root_threshold: u32,
    pub release_signers: Vec<Signer>,
    pub release_threshold: u32,
    pub prev_root_hash: Option<[u8; 32]>,
}

/// A trust root as signed: the exact `payload` bytes plus detached signatures over them.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SignedRoot {
    #[serde(with = "serde_bytes")]
    pub payload: Vec<u8>,
    #[serde(with = "bytes_vec")]
    pub signatures: Vec<Vec<u8>>,
}

impl SignedRoot {
    /// Decode the payload. Verification of the signatures is the verifier's job (Task 3).
    pub fn root(&self) -> Result<TrustRoot, Refusal> {
        decode(&self.payload)
    }

    /// Content address: SHA-256 of the payload ALONE, so adding/removing signatures
    /// never changes which root this is.
    pub fn hash(&self) -> [u8; 32] {
        Sha256::digest(&self.payload).into()
    }
}

/// One file in a release. `os`/`arch`/`platform`/`generation_line` apply to programs
/// (`kind = "window"`) and are `None` for kinds that have no such notion.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Artifact {
    pub kind: String,
    pub digest: [u8; 32],
    pub byte_len: u64,
    pub os: Option<String>,
    pub arch: Option<String>,
    pub platform: Option<String>,
    pub generation_line: Option<String>,
}

/// The signed list of what a release contains.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Manifest {
    pub channel_id: String,
    pub release_version: u64,
    pub root_version_ref: u64,
    pub artifacts: Vec<Artifact>,
}

/// The signed part of a release: the root chain, the manifest bytes and its signatures.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ReleaseMeta {
    pub root_chain: Vec<SignedRoot>,
    #[serde(with = "serde_bytes")]
    pub manifest: Vec<u8>,
    #[serde(with = "bytes_vec")]
    pub release_signatures: Vec<Vec<u8>>,
    pub inclusion_proof: Option<serde_bytes::ByteBuf>,
}

/// The whole bundle file: meta plus the artifact bytes, in manifest order.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct BundleFile {
    pub meta: ReleaseMeta,
    #[serde(with = "bytes_vec")]
    pub artifacts: Vec<Vec<u8>>,
}

/// The pinned root compiled into the program. `None` until the genesis ceremony, so
/// every intake/install refuses with `NoChannel` before then.
pub fn official_root() -> Option<SignedRoot> {
    None
}

/// Canonical CBOR (the same `ciborium::into_writer` `cairn_event::canonical_cbor` uses).
pub fn encode<T: Serialize>(value: &T) -> Vec<u8> {
    let mut buf = Vec::new();
    // Writing to a Vec cannot fail and these types have no unserialisable members.
    ciborium::into_writer(value, &mut buf).expect("CBOR encode to Vec");
    buf
}

pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, Refusal> {
    ciborium::from_reader(bytes).map_err(|e| Refusal::Malformed(e.to_string()))
}

/// Serde adapter: a list of byte strings as CBOR byte strings (not arrays of ints,
/// which would bloat a multi-megabyte program ~2x).
mod bytes_vec {
    use serde::{Deserializer, Serializer};
    use serde_bytes::ByteBuf;

    pub fn serialize<S: Serializer>(v: &[Vec<u8>], s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        let mut seq = s.serialize_seq(Some(v.len()))?;
        for b in v {
            seq.serialize_element(serde_bytes::Bytes::new(b))?;
        }
        seq.end()
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Vec<u8>>, D::Error> {
        let v: Vec<ByteBuf> = serde::Deserialize::deserialize(d)?;
        Ok(v.into_iter().map(ByteBuf::into_vec).collect())
    }
}
