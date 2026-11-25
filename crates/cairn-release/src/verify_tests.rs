//! The ADR-0055 verifier, driven by roots and bundles signed with keys minted at runtime.
//! Each test names one rule of decisions 2, 5 and 6; read the test name as the rule.

use crate::*;
use cairn_event::{sign_release_manifest, sign_trust_root, SigningKey};
use sha2::{Digest, Sha256};

// ---- fixtures ---------------------------------------------------------------------------

/// A runtime-derived Ed25519 key (house rule 6: never a literal in a crypto context).
pub(super) fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&std::array::from_fn(|i| (i as u8).wrapping_mul(31) ^ seed))
}

pub(super) fn signer(sk: &SigningKey) -> Signer {
    let pubkey = sk.verifying_key().to_bytes();
    Signer {
        kid: pubkey[..4].to_vec(),
        pubkey,
        custody_note: "test token".into(),
    }
}

/// A trust root on channel "official" with the given signer sets and thresholds.
pub(super) fn doc(
    version: u64,
    prev: Option<[u8; 32]>,
    roots: &[&SigningKey],
    root_threshold: u32,
    releases: &[&SigningKey],
    release_threshold: u32,
) -> TrustRoot {
    TrustRoot {
        channel_id: "official".into(),
        version,
        root_signers: roots.iter().map(|k| signer(k)).collect(),
        root_threshold,
        release_signers: releases.iter().map(|k| signer(k)).collect(),
        release_threshold,
        prev_root_hash: prev,
    }
}

pub(super) fn signed(root: &TrustRoot, with: &[&SigningKey]) -> SignedRoot {
    let payload = encode(root);
    let signatures = with
        .iter()
        .map(|k| sign_trust_root(&payload, *k).unwrap())
        .collect();
    SignedRoot {
        payload,
        signatures,
    }
}

/// The standard three-root chain: v1 (root a,b ≥2; release r1), v2 (root c,d ≥2; release r2)
/// signed by a,b, and v3 (root c,d; release r2) signed by c,d.
pub(super) struct Chain {
    pub(super) v1: SignedRoot,
    pub(super) v2: SignedRoot,
    pub(super) v3: SignedRoot,
}

pub(super) fn chain() -> Chain {
    let (a, b, c, d, r1, r2) = (key(1), key(2), key(3), key(4), key(11), key(12));
    let v1 = signed(&doc(1, None, &[&a, &b], 2, &[&r1], 1), &[&a, &b]);
    let v2 = signed(&doc(2, Some(v1.hash()), &[&c, &d], 2, &[&r2], 1), &[&a, &b]);
    let v3 = signed(&doc(3, Some(v2.hash()), &[&c, &d], 2, &[&r2], 1), &[&c, &d]);
    Chain { v1, v2, v3 }
}

/// Verify `roots` from genesis, as a node rebuilding its stored history would.
pub(super) fn history(roots: &[&SignedRoot]) -> Vec<VerifiedRoot> {
    let g = genesis(roots[0]).unwrap();
    let rest: Vec<SignedRoot> = roots[1..].iter().map(|r| (*r).clone()).collect();
    advance(&[g], &rest).unwrap()
}

pub(super) fn versions(h: &[VerifiedRoot]) -> Vec<u64> {
    h.iter().map(|v| v.root().version).collect()
}

pub(super) fn window(bytes: &[u8]) -> Artifact {
    Artifact {
        kind: "window".into(),
        digest: Sha256::digest(bytes).into(),
        byte_len: bytes.len() as u64,
        os: Some("linux".into()),
        arch: Some("aarch64".into()),
        platform: Some("debian-12".into()),
        generation_line: None,
    }
}

pub(super) fn manifest(release_version: u64, root_version_ref: u64) -> Manifest {
    Manifest {
        channel_id: "official".into(),
        release_version,
        root_version_ref,
        artifacts: vec![window(b"the window program")],
    }
}

pub(super) fn meta(root_chain: Vec<SignedRoot>, m: &Manifest, with: &[&SigningKey]) -> ReleaseMeta {
    let manifest = encode(m);
    let release_signatures = with
        .iter()
        .map(|k| sign_release_manifest(&manifest, *k).unwrap())
        .collect();
    ReleaseMeta {
        root_chain,
        manifest,
        release_signatures,
        inclusion_proof: None,
    }
}

// ---- genesis ----------------------------------------------------------------------------

#[test]
fn genesis_needs_its_own_threshold() {
    let (a, b, stranger, r) = (key(1), key(2), key(9), key(11));
    let root = doc(1, None, &[&a, &b], 2, &[&r], 1);
    assert_eq!(
        genesis(&signed(&root, &[&a, &stranger])).unwrap_err(),
        Refusal::RootSignaturesShort {
            version: 1,
            have: 1,
            need: 2
        }
    );
    let ok = genesis(&signed(&root, &[&a, &b])).unwrap();
    assert_eq!(ok.root(), &root);
    assert_eq!(ok.hash(), signed(&root, &[]).hash());
}

#[test]
fn a_genesis_must_be_version_one_with_no_parent() {
    let (a, r) = (key(1), key(11));
    for bad in [
        doc(2, None, &[&a], 1, &[&r], 1),
        doc(1, Some([0; 32]), &[&a], 1, &[&r], 1),
    ] {
        assert!(matches!(
            genesis(&signed(&bad, &[&a])),
            Err(Refusal::Malformed(_))
        ));
    }
}

#[test]
fn a_root_with_a_zero_or_unmeetable_threshold_or_a_repeated_key_is_malformed() {
    let (a, r) = (key(1), key(11));
    let mut bads = vec![
        doc(1, None, &[&a], 0, &[&r], 1),
        doc(1, None, &[&a], 1, &[&r], 0),
        doc(1, None, &[&a], 2, &[&r], 1),
        doc(1, None, &[&a], 1, &[&r], 2),
        doc(1, None, &[&a, &a], 1, &[&r], 1),
        doc(1, None, &[&a], 1, &[&r, &r], 1),
    ];
    // The same key under two different labels is still one signer.
    let mut relabelled = doc(1, None, &[&a, &a], 1, &[&r], 1);
    relabelled.root_signers[1].kid = vec![0xEE];
    bads.push(relabelled);
    for bad in bads {
        assert!(
            matches!(genesis(&signed(&bad, &[&a])), Err(Refusal::Malformed(_))),
            "{bad:?}"
        );
    }
}

#[test]
fn a_signer_key_that_is_not_a_usable_public_key_is_malformed() {
    let (a, r) = (key(1), key(11));
    // The identity point decodes but is weak (small order): a forger's key, never a signer's.
    let weak: [u8; 32] = std::array::from_fn(|i| u8::from(i == 0));
    // A value that is not a curve point at all, found at runtime (house rule 6).
    let not_a_point = (0u8..=255)
        .map(|seed| std::array::from_fn(|i| (i as u8).wrapping_mul(13) ^ seed))
        .find(|b: &[u8; 32]| cairn_event::VerifyingKey::from_bytes(b).is_err())
        .expect("some 32-byte value is not a point");
    for bad in [weak, not_a_point] {
        for in_release in [false, true] {
            let mut root = doc(1, None, &[&a], 1, &[&r], 1);
            if in_release {
                root.release_signers[0].pubkey = bad;
            } else {
                root.root_signers.push(Signer {
                    kid: vec![9],
                    pubkey: bad,
                    custody_note: String::new(),
                });
            }
            assert!(
                matches!(genesis(&signed(&root, &[&a])), Err(Refusal::Malformed(_))),
                "{bad:?} in_release={in_release}"
            );
        }
    }
}

#[test]
fn junk_signatures_count_for_nothing_and_never_panic() {
    let (a, r) = (key(1), key(11));
    let root = doc(1, None, &[&a], 1, &[&r], 1);
    let mut s = signed(&root, &[]);
    // Garbage, a signature over other bytes, and a release-context signature by the right key.
    s.signatures = vec![
        vec![0xff; 70],
        vec![],
        sign_trust_root(b"other bytes", &a).unwrap(),
        sign_release_manifest(&s.payload, &a).unwrap(),
    ];
    assert_eq!(
        genesis(&s).unwrap_err(),
        Refusal::RootSignaturesShort {
            version: 1,
            have: 0,
            need: 1
        }
    );
}

#[test]
fn one_signer_counted_once() {
    let (a, b, r) = (key(1), key(2), key(11));
    let root = doc(1, None, &[&a, &b], 2, &[&r], 1);
    assert_eq!(
        genesis(&signed(&root, &[&a, &a])).unwrap_err(),
        Refusal::RootSignaturesShort {
            version: 1,
            have: 1,
            need: 2
        }
    );
    // The same for release signatures: one key signing twice meets a threshold of 1, not 2.
    let (r2, x) = (key(12), key(13));
    let g = genesis(&signed(&doc(1, None, &[&a], 1, &[&r2, &x], 2), &[&a])).unwrap();
    assert_eq!(
        verify_release(g.root(), &meta(vec![], &manifest(1, 1), &[&r2, &r2]), 0).unwrap_err(),
        Refusal::ReleaseSignaturesShort { have: 1, need: 2 }
    );
}

// ---- advance: the root chain -------------------------------------------------------------

#[test]
fn a_rotation_needs_the_previous_signers() {
    let (a, b, c, d, r) = (key(1), key(2), key(3), key(4), key(11));
    let h = history(&[&signed(&doc(1, None, &[&a, &b], 2, &[&r], 1), &[&a, &b])]);
    let next = doc(2, Some(h[0].hash()), &[&c, &d], 2, &[&r], 1);
    assert_eq!(
        advance(&h, &[signed(&next, &[&c, &d])]).unwrap_err(),
        Refusal::RootSignaturesShort {
            version: 2,
            have: 0,
            need: 2
        }
    );
    // Even fully signed by the old set, a successor that does not name its parent fails.
    let orphan = doc(2, Some([7; 32]), &[&c, &d], 2, &[&r], 1);
    assert!(matches!(
        advance(&h, &[signed(&orphan, &[&a, &b])]),
        Err(Refusal::RootSignaturesShort { version: 2, .. })
    ));
    let h2 = advance(&h, &[signed(&next, &[&a, &b])]).unwrap();
    assert_eq!(versions(&h2), [1, 2]);
}

#[test]
fn a_two_rotation_gap_verifies_from_an_old_pin() {
    let c = chain();
    let h = history(&[&c.v1]);
    // Given out of order and with the already-held v1 repeated: still a clean 1 -> 2 -> 3.
    let h3 = advance(&h, &[c.v3.clone(), c.v1.clone(), c.v2.clone()]).unwrap();
    assert_eq!(versions(&h3), [1, 2, 3]);
    assert_eq!(h3[2].hash(), c.v3.hash());
}

#[test]
fn a_missing_version_is_a_chain_gap() {
    let c = chain();
    let h = history(&[&c.v1, &c.v2]);
    let (cc, dd, r) = (key(3), key(4), key(12));
    let v4 = signed(
        &doc(4, Some([1; 32]), &[&cc, &dd], 2, &[&r], 1),
        &[&cc, &dd],
    );
    let err = advance(&h, &[v4]).unwrap_err();
    assert_eq!(
        err,
        Refusal::ChainGap {
            pinned: 2,
            needs: 4
        }
    );
    let s = err.sentence(Who::Node);
    assert!(s.contains("version 2") && s.contains("version 4"), "{s}");
}

#[test]
fn a_gap_after_a_verified_step_names_the_held_version() {
    // advance is all or nothing, so the machine still holds v1, not the v2 verified on the way.
    let c = chain();
    let h = history(&[&c.v1]);
    let (cc, dd, r) = (key(3), key(4), key(12));
    let v4 = signed(
        &doc(4, Some([1; 32]), &[&cc, &dd], 2, &[&r], 1),
        &[&cc, &dd],
    );
    assert_eq!(
        advance(&h, &[c.v2.clone(), v4]).unwrap_err(),
        Refusal::ChainGap {
            pinned: 1,
            needs: 4
        }
    );
}

#[test]
fn the_same_new_payload_twice_is_accepted_once() {
    let c = chain();
    let h = history(&[&c.v1]);
    let (a, b) = (key(1), key(2));
    // Same payload, a different signature set: one version, not a fork.
    let again = SignedRoot {
        payload: c.v2.payload.clone(),
        signatures: vec![
            sign_trust_root(&c.v2.payload, &b).unwrap(),
            sign_trust_root(&c.v2.payload, &a).unwrap(),
        ],
    };
    let h2 = advance(&h, &[c.v2.clone(), again, c.v2.clone()]).unwrap();
    assert_eq!(versions(&h2), [1, 2]);
}

#[test]
fn two_successors_are_a_fork() {
    let c = chain();
    let h = history(&[&c.v1]);
    let (a, b, e, r) = (key(1), key(2), key(5), key(11));
    let rival = signed(&doc(2, Some(c.v1.hash()), &[&e], 1, &[&r], 1), &[&a, &b]);
    assert_eq!(
        advance(&h, &[c.v2.clone(), rival]).unwrap_err(),
        Refusal::Fork { version: 2 }
    );
    // A rival that does NOT verify is noise, not a fork: it cannot block a genuine rotation.
    let junk = signed(&doc(2, Some(c.v1.hash()), &[&e], 1, &[&r], 1), &[&e]);
    assert_eq!(
        versions(&advance(&h, &[junk, c.v2.clone()]).unwrap()),
        [1, 2]
    );
}

#[test]
fn a_differing_known_version_is_a_fork_only_if_it_verifies() {
    let c = chain();
    let h = history(&[&c.v1, &c.v2]);
    let (a, b, e, r) = (key(1), key(2), key(5), key(11));
    let rival = doc(2, Some(c.v1.hash()), &[&e], 1, &[&r], 1);
    assert_eq!(
        advance(&h, &[signed(&rival, &[&a, &b])]).unwrap_err(),
        Refusal::Fork { version: 2 }
    );
    let unchanged = advance(&h, &[signed(&rival, &[&e])]).unwrap();
    assert_eq!(unchanged, h);
    // A second genesis has nothing earlier to verify against: it is ignored, never a fork.
    let other_genesis = signed(&doc(1, None, &[&e], 1, &[&r], 1), &[&e]);
    assert_eq!(advance(&h, &[other_genesis]).unwrap(), h);
}

#[test]
fn a_regressed_chain_changes_nothing() {
    let c = chain();
    let h = history(&[&c.v1, &c.v2, &c.v3]);
    assert_eq!(advance(&h, &[c.v1.clone(), c.v2.clone()]).unwrap(), h);
    assert_eq!(advance(&h, &[]).unwrap(), h);
}

#[test]
fn a_root_from_another_channel_is_the_wrong_channel() {
    let c = chain();
    let h = history(&[&c.v1]);
    let (a, b, r) = (key(1), key(2), key(11));
    let mut other = doc(2, Some(c.v1.hash()), &[&a, &b], 2, &[&r], 1);
    other.channel_id = "community".into();
    assert_eq!(
        advance(&h, &[signed(&other, &[&a, &b])]).unwrap_err(),
        Refusal::WrongChannel {
            pinned: "official".into(),
            offered: "community".into()
        }
    );
}

#[test]
fn a_damaged_chain_entry_or_an_empty_pin_is_refused() {
    let c = chain();
    let h = history(&[&c.v1]);
    let damaged = SignedRoot {
        payload: vec![0xff, 0x00],
        signatures: vec![],
    };
    assert!(matches!(
        advance(&h, &[damaged]),
        Err(Refusal::Malformed(_))
    ));
    assert_eq!(advance(&[], &[c.v1]).unwrap_err(), Refusal::NoChannel);
}
