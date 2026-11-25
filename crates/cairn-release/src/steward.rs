//! The steward's pure builders (ADR-0055). These run ONLY on the steward's offline machine
//! (via the `cairn-release` binary) and in tests; never on a node, never in CI, because
//! they take signing keys. They hold no I/O: the binary does the file reading, so every
//! rule here is testable by round-tripping through the real verifier.

use crate::VerifiedRoot;
use crate::{encode, Artifact, BundleFile, Manifest, ReleaseMeta, SignedRoot, Signer, TrustRoot};
use cairn_event::{sign_release_manifest, sign_trust_root, SigningKey};
use sha2::{Digest, Sha256};

// Signing an in-memory payload with a valid key cannot fail in practice; a failure would be
// a bug in cairn-event, so it is loud rather than threaded through every caller.
const SIGN_OK: &str = "signing an in-memory payload";

/// Sign an encoded trust root with each key, in the trust-root context.
fn sign_root(root: &TrustRoot, sign_with: &[&SigningKey]) -> SignedRoot {
    let payload = encode(root);
    let signatures = sign_with
        .iter()
        .map(|k| sign_trust_root(&payload, *k).expect(SIGN_OK))
        .collect();
    SignedRoot {
        payload,
        signatures,
    }
}

/// A channel's first trust root: version 1, no parent, self-signed by (at least the
/// threshold of) its own root signers. `genesis()` accepts exactly this shape.
pub fn make_genesis(
    channel_id: &str,
    root_signers: Vec<Signer>,
    root_threshold: u32,
    release_signers: Vec<Signer>,
    release_threshold: u32,
    sign_with: &[&SigningKey],
) -> SignedRoot {
    let root = TrustRoot {
        channel_id: channel_id.into(),
        version: 1,
        root_signers,
        root_threshold,
        release_signers,
        release_threshold,
        prev_root_hash: None,
    };
    sign_root(&root, sign_with)
}

/// The next trust root after `prev`. Version and parent hash are set HERE (not trusted
/// from `next`) so a chain can't be mis-linked by hand. It must be signed by the PREVIOUS
/// root's signers: that is what lets a pinned terminal accept it.
pub fn rotate(prev: &VerifiedRoot, mut next: TrustRoot, sign_with: &[&SigningKey]) -> SignedRoot {
    next.version = prev.root().version + 1;
    next.prev_root_hash = Some(prev.hash());
    sign_root(&next, sign_with)
}

/// One release bundle. `history` is the verified chain from genesis (the whole chain, so
/// a terminal pinned at any version can climb); the manifest names the newest root and is
/// signed by that root's release signers. Digest and length are filled from the bytes, so
/// a caller cannot sign a list that disagrees with the files it ships.
pub fn assemble(
    history: &[VerifiedRoot],
    release_version: u64,
    artifacts: Vec<(Artifact, Vec<u8>)>,
    sign_with: &[&SigningKey],
) -> BundleFile {
    let newest = history.last().expect("a non-empty root history").root();
    let (listed, files): (Vec<Artifact>, Vec<Vec<u8>>) = artifacts
        .into_iter()
        .map(|(mut a, bytes)| {
            a.digest = Sha256::digest(&bytes).into();
            a.byte_len = bytes.len() as u64;
            (a, bytes)
        })
        .unzip();
    let manifest = encode(&Manifest {
        channel_id: newest.channel_id.clone(),
        release_version,
        root_version_ref: newest.version,
        artifacts: listed,
    });
    let release_signatures = sign_with
        .iter()
        .map(|k| sign_release_manifest(&manifest, *k).expect(SIGN_OK))
        .collect();
    BundleFile {
        meta: ReleaseMeta {
            root_chain: history.iter().map(|v| v.signed().clone()).collect(),
            manifest,
            release_signatures,
            inclusion_proof: None,
        },
        artifacts: files,
    }
}
