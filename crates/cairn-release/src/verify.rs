//! The ADR-0055 verifier: the one safety-critical seam between "bytes someone handed us"
//! and "a program that runs on the screen clinicians type their passphrase into".
//!
//! It is PURE: no I/O, no clock, no database. The node's intake and the terminal's
//! root-owned installer both call it with what they hold (their verified root history,
//! their last installed release) and what they were offered (a `ReleaseMeta`), and act only
//! on `Ok`. Gate order (ADR-0055 decision 6):
//!
//! 1. `advance` — walk the offered root chain up from the pinned history, one version at a
//!    time, each version proven by the root signers of the version before it (decision 2),
//!    stopping on a gap or a fork (decision 5);
//! 2. `verify_release` — the manifest must be signed by the release signers of the NEWEST
//!    verified root, never an older one the manifest names (decision 6);
//! 3. `check_artifact` — every file's bytes must match the digest and length the signed
//!    manifest lists.
//!
//! How a signature "counts" (controller ruling R6): a detached signature names its key by
//! the 32-byte public key (`detached_key_id`). It counts toward a threshold only if that key
//! is in the trusted signer list AND the signature verifies with it; each distinct key
//! counts once however many times it signs. Anything unparseable simply does not count.

use crate::docs::{decode, Manifest, ReleaseMeta, SignedRoot, Signer, TrustRoot};
use crate::refusal::Refusal;
use cairn_event::{
    detached_key_id, verify_release_manifest_sig, verify_trust_root_sig, EventError, VerifyingKey,
};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

/// A trust root this program has proven, by `genesis` or `advance`. The fields are private
/// so the only way to hold one is to have verified it: callers that persist roots store the
/// `SignedRoot`s and re-verify them on load.
#[derive(Clone, Debug, PartialEq)]
pub struct VerifiedRoot {
    root: TrustRoot,
    signed: SignedRoot,
    hash: [u8; 32],
}

impl VerifiedRoot {
    /// The decoded document.
    pub fn root(&self) -> &TrustRoot {
        &self.root
    }
    /// The document as signed (payload + the signatures that came with it).
    pub fn signed(&self) -> &SignedRoot {
        &self.signed
    }
    /// The content address: SHA-256 of the payload alone.
    pub fn hash(&self) -> [u8; 32] {
        self.hash
    }
}

/// The signature check for one document kind (trust root or manifest), so the counting
/// below is written once and the signing context can never be mixed up by a caller.
type SigCheck = fn(&[u8], &[u8], &VerifyingKey) -> Result<(), EventError>;

/// How many DISTINCT keys from `trusted` validly signed `payload`. A signature whose key is
/// not in `trusted`, that does not parse, or that does not verify adds nothing.
fn count_valid(sigs: &[Vec<u8>], payload: &[u8], trusted: &[Signer], check: SigCheck) -> u32 {
    let mut counted: Vec<[u8; 32]> = Vec::new();
    for sig in sigs {
        let Ok(kid) = detached_key_id(sig) else {
            continue;
        };
        let Some(signer) = trusted.iter().find(|s| s.pubkey[..] == kid[..]) else {
            continue;
        };
        if counted.contains(&signer.pubkey) {
            continue; // the same key signing twice is still one signer
        }
        let Ok(vk) = VerifyingKey::from_bytes(&signer.pubkey) else {
            continue;
        };
        if check(sig, payload, &vk).is_ok() {
            counted.push(signer.pubkey);
        }
    }
    counted.len() as u32
}

/// The shape rules every trust root must meet before any signature is counted against it:
/// thresholds are at least 1 (0 would mean "no signatures needed") and meetable (an
/// unmeetable one would brick the channel forever), no signer list repeats a key, and every
/// key is a usable, non-weak Ed25519 public key. Linear in the list length: it runs on
/// unverified input.
fn check_shape(root: &TrustRoot) -> Result<(), Refusal> {
    for (what, signers, threshold) in [
        ("root", &root.root_signers, root.root_threshold),
        ("release", &root.release_signers, root.release_threshold),
    ] {
        if threshold == 0 || threshold as usize > signers.len() {
            return Err(Refusal::Malformed(format!(
                "{what} threshold {threshold} with {} signers",
                signers.len()
            )));
        }
        let mut seen = HashSet::with_capacity(signers.len());
        for s in signers {
            if !seen.insert(s.pubkey) {
                return Err(Refusal::Malformed(format!("{what} signer key repeated")));
            }
            // A pubkey that is not a curve point, or is a small-order (weak) point that
            // "verifies" forged signatures, can never be a real signer (ruling R11).
            match VerifyingKey::from_bytes(&s.pubkey) {
                Ok(vk) if !vk.is_weak() => {}
                _ => return Err(Refusal::Malformed(format!("{what} signer key unusable"))),
            }
        }
    }
    Ok(())
}

/// Decode a signed root and check its shape.
fn decode_root(signed: &SignedRoot) -> Result<TrustRoot, Refusal> {
    let root = signed.root()?;
    check_shape(&root)?;
    Ok(root)
}

/// Is `candidate` a valid successor of `prev`? It must name `prev` as its parent and carry
/// at least `prev`'s root threshold of signatures from `prev`'s root signers. Returns the
/// count either way so a refusal can say "have of need"; a wrong parent counts as 0.
fn successor_signatures(prev: &VerifiedRoot, candidate: &TrustRoot, signed: &SignedRoot) -> u32 {
    if candidate.prev_root_hash != Some(prev.hash) {
        return 0;
    }
    count_valid(
        &signed.signatures,
        &signed.payload,
        &prev.root.root_signers,
        verify_trust_root_sig,
    )
}

/// Accept a channel's first trust root (version 1, no parent), self-signed by at least its
/// own root threshold of its own root signers. This is the provisioning step: whoever hands
/// a machine its genesis is trusted to hand it the right one.
pub fn genesis(signed: &SignedRoot) -> Result<VerifiedRoot, Refusal> {
    let root = decode_root(signed)?;
    if root.version != 1 || root.prev_root_hash.is_some() {
        return Err(Refusal::Malformed(
            "a genesis root is version 1 with no parent".into(),
        ));
    }
    let have = count_valid(
        &signed.signatures,
        &signed.payload,
        &root.root_signers,
        verify_trust_root_sig,
    );
    if have < root.root_threshold {
        return Err(Refusal::RootSignaturesShort {
            version: 1,
            have,
            need: root.root_threshold,
        });
    }
    Ok(VerifiedRoot {
        root,
        signed: signed.clone(),
        hash: signed.hash(),
    })
}

/// Extend a verified, ascending, non-empty `history` with whatever of `chain` follows it.
/// Returns the new history (equal to `history` when the chain adds nothing). All or
/// nothing: on any refusal the caller keeps its old history.
///
/// Flow: decode every entry and refuse a damaged one or one from another channel; check
/// entries at versions we already hold for a verifiable rival (a fork); then climb one
/// version at a time, accepting a version only when exactly one distinct payload for it
/// verifies against the version below.
pub fn advance(
    history: &[VerifiedRoot],
    chain: &[SignedRoot],
) -> Result<Vec<VerifiedRoot>, Refusal> {
    let Some(pinned) = history.last() else {
        return Err(Refusal::NoChannel);
    };
    let mut offered: Vec<(TrustRoot, &SignedRoot)> = Vec::new();
    for signed in chain {
        let root = decode_root(signed)?;
        if root.channel_id != pinned.root.channel_id {
            return Err(Refusal::WrongChannel {
                pinned: pinned.root.channel_id.clone(),
                offered: root.channel_id,
            });
        }
        offered.push((root, signed));
    }

    // Versions we already hold: an identical payload is a harmless repeat; a different one
    // that verifies against the version below it is a fork. One that does not verify (or
    // has nothing below it in our history, e.g. a rival genesis) is noise and is ignored.
    for (root, signed) in &offered {
        let Some(held) = history.iter().find(|h| h.root.version == root.version) else {
            continue;
        };
        if held.hash == signed.hash() {
            continue;
        }
        let below = history
            .iter()
            .find(|h| Some(h.root.version) == root.version.checked_sub(1));
        if let Some(below) = below {
            if successor_signatures(below, root, signed) >= below.root.root_threshold {
                return Err(Refusal::Fork {
                    version: root.version,
                });
            }
        }
    }

    // Climb from the pin. At each step gather every offered entry for the next version.
    let mut out = history.to_vec();
    loop {
        let top = out.last().expect("history is non-empty");
        let Some(next) = top.root.version.checked_add(1) else {
            return Ok(out); // nothing can follow the last representable version
        };
        let candidates: Vec<&(TrustRoot, &SignedRoot)> =
            offered.iter().filter(|(r, _)| r.version == next).collect();
        if candidates.is_empty() {
            // Nothing for `next`: done, unless something higher was offered (a gap). This
            // fails CLOSED: even unverifiable junk at a high version refuses the whole chain
            // as a gap rather than being silently skipped, so a human looks at it.
            return match offered
                .iter()
                .map(|(r, _)| r.version)
                .filter(|v| *v > next)
                .min()
            {
                // `pinned` is what the caller still holds: advance is all or nothing (R10).
                Some(needs) => Err(Refusal::ChainGap {
                    pinned: pinned.root.version,
                    needs,
                }),
                None => Ok(out),
            };
        }
        let need = top.root.root_threshold;
        let mut best = 0;
        let mut accepted: Option<VerifiedRoot> = None;
        for (root, signed) in candidates {
            let have = successor_signatures(top, root, signed);
            best = best.max(have);
            if have < need {
                continue; // an unverifiable candidate is noise; it cannot block a real one
            }
            match &accepted {
                Some(a) if a.hash != signed.hash() => {
                    return Err(Refusal::Fork { version: next });
                }
                Some(_) => {} // the same payload again, perhaps with other signatures
                None => {
                    accepted = Some(VerifiedRoot {
                        root: root.clone(),
                        signed: (*signed).clone(),
                        hash: signed.hash(),
                    })
                }
            }
        }
        match accepted {
            Some(v) => out.push(v),
            None => {
                return Err(Refusal::RootSignaturesShort {
                    version: next,
                    have: best,
                    need,
                })
            }
        }
    }
}

/// Check a release manifest against the NEWEST verified root: it decodes, is on this
/// channel, carries at least `release_threshold` valid signatures from `newest`'s release
/// signers (the manifest's `root_version_ref` is diagnostic only and never chooses the
/// set), and is newer than `last_installed`. Returns the decoded manifest.
pub fn verify_release(
    newest: &TrustRoot,
    meta: &ReleaseMeta,
    last_installed: u64,
) -> Result<Manifest, Refusal> {
    let manifest: Manifest = decode(&meta.manifest)?;
    if manifest.channel_id != newest.channel_id {
        return Err(Refusal::WrongChannel {
            pinned: newest.channel_id.clone(),
            offered: manifest.channel_id,
        });
    }
    let have = count_valid(
        &meta.release_signatures,
        &meta.manifest,
        &newest.release_signers,
        verify_release_manifest_sig,
    );
    if have < newest.release_threshold {
        return Err(Refusal::ReleaseSignaturesShort {
            have,
            need: newest.release_threshold,
        });
    }
    if manifest.release_version <= last_installed {
        return Err(Refusal::NotNewer {
            installed: last_installed,
            offered: manifest.release_version,
        });
    }
    Ok(manifest)
}

/// Check that `bytes` are exactly artifact `index` of a verified manifest: same length and
/// same SHA-256. A missing index is a mismatch too.
pub fn check_artifact(manifest: &Manifest, index: usize, bytes: &[u8]) -> Result<(), Refusal> {
    let mismatch = Err(Refusal::ArtifactMismatch { index });
    let Some(a) = manifest.artifacts.get(index) else {
        return mismatch;
    };
    if a.byte_len != bytes.len() as u64 || a.digest != <[u8; 32]>::from(Sha256::digest(bytes)) {
        return mismatch;
    }
    Ok(())
}

/// Gates 1 and 2 together: advance the pin over the bundle's root chain, then verify the
/// manifest against the newest root. Returns the new history (for the caller to persist)
/// and the manifest (for `check_artifact` on each file).
///
/// All or nothing (ruling R12): if the manifest fails, a rotation the bundle's chain proved
/// is NOT kept either; the caller keeps its old history and the next good bundle carries
/// the rotation again.
pub fn verify_bundle(
    history: &[VerifiedRoot],
    meta: &ReleaseMeta,
    last_installed: u64,
) -> Result<(Vec<VerifiedRoot>, Manifest), Refusal> {
    let history = advance(history, &meta.root_chain)?;
    let newest = &history
        .last()
        .expect("advance returns a non-empty history")
        .root;
    let manifest = verify_release(newest, meta, last_installed)?;
    Ok((history, manifest))
}
