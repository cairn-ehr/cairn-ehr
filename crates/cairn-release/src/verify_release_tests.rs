//! The ADR-0055 verifier, gates 2 and 3: release signatures and artifact digests. Split
//! from `verify_tests.rs` (the root chain) for the 500-line cap; the fixtures live there.

use crate::verify_tests::*;
use crate::*;
use cairn_event::sign_trust_root;

// ---- verify_release / verify_bundle -------------------------------------------------------

#[test]
fn a_rotated_out_release_key_is_refused_once_seen() {
    let c = chain();
    let (r1, r2) = (key(11), key(12));
    // This terminal already holds v2 (release set r2); a bundle that withholds v2 and is
    // signed by the rotated-out r1 must not pass.
    let h = history(&[&c.v1, &c.v2]);
    let m = manifest(1, 1);
    assert_eq!(
        verify_bundle(&h, &meta(vec![c.v1.clone()], &m, &[&r1]), 0).unwrap_err(),
        Refusal::ReleaseSignaturesShort { have: 0, need: 1 }
    );
    let (h2, got) = verify_bundle(&h, &meta(vec![c.v1.clone()], &m, &[&r2]), 0).unwrap();
    assert_eq!((versions(&h2), got), (vec![1, 2], m));
}

#[test]
fn an_older_root_cannot_be_elected() {
    let c = chain();
    let (r1, r2) = (key(11), key(12));
    let h = history(&[&c.v1]);
    let chain = vec![c.v1.clone(), c.v2.clone()];
    // root_version_ref names v1, whose release set r1 signed — but v2 is the newest seen.
    let m = manifest(1, 1);
    assert_eq!(
        verify_bundle(&h, &meta(chain.clone(), &m, &[&r1]), 0).unwrap_err(),
        Refusal::ReleaseSignaturesShort { have: 0, need: 1 }
    );
    let (h2, _) = verify_bundle(&h, &meta(chain, &m, &[&r2]), 0).unwrap();
    assert_eq!(versions(&h2), [1, 2]);
}

#[test]
fn only_release_context_signatures_from_release_signers_count() {
    let c = chain();
    let (a, r1) = (key(1), key(11));
    let g = history(&[&c.v1]);
    let m = manifest(1, 1);
    // A root signer (a) is not a release signer: its manifest signature counts 0.
    assert_eq!(
        verify_release(&g[0], &meta(vec![], &m, &[&a]), 0).unwrap_err(),
        Refusal::ReleaseSignaturesShort { have: 0, need: 1 }
    );
    // The release signer's key, but in the trust-root context: counts 0.
    let mut wrong_ctx = meta(vec![], &m, &[]);
    wrong_ctx.release_signatures = vec![sign_trust_root(&wrong_ctx.manifest, &r1).unwrap()];
    assert_eq!(
        verify_release(&g[0], &wrong_ctx, 0).unwrap_err(),
        Refusal::ReleaseSignaturesShort { have: 0, need: 1 }
    );
}

#[test]
fn a_release_not_newer_is_refused() {
    let c = chain();
    let r1 = key(11);
    let g = history(&[&c.v1]);
    for (offered, installed) in [(5, 5), (4, 5)] {
        assert_eq!(
            verify_release(
                &g[0],
                &meta(vec![], &manifest(offered, 1), &[&r1]),
                installed
            )
            .unwrap_err(),
            Refusal::NotNewer { installed, offered }
        );
    }
    let m = manifest(6, 1);
    assert_eq!(
        verify_release(&g[0], &meta(vec![], &m, &[&r1]), 5).unwrap(),
        m
    );
}

#[test]
fn a_release_from_another_channel_or_a_damaged_manifest_is_refused() {
    let c = chain();
    let r1 = key(11);
    let g = history(&[&c.v1]);
    let mut m = manifest(1, 1);
    m.channel_id = "community".into();
    assert_eq!(
        verify_release(&g[0], &meta(vec![], &m, &[&r1]), 0).unwrap_err(),
        Refusal::WrongChannel {
            pinned: "official".into(),
            offered: "community".into()
        }
    );
    let mut damaged = meta(vec![], &manifest(1, 1), &[&r1]);
    damaged.manifest.push(0);
    assert!(matches!(
        verify_release(&g[0], &damaged, 0),
        Err(Refusal::Malformed(_))
    ));
}

// ---- check_artifact ----------------------------------------------------------------------

#[test]
fn a_wrong_digest_or_length_is_an_artifact_mismatch() {
    let m = manifest(1, 1);
    check_artifact(&m, 0, b"the window program").unwrap();
    let mismatch = Err(Refusal::ArtifactMismatch { index: 0 });
    assert_eq!(check_artifact(&m, 0, b"the window programme"), mismatch);
    assert_eq!(check_artifact(&m, 0, b"the window prograM"), mismatch);
    // Right digest, wrong declared length: still not the file the list names.
    let mut lying = m.clone();
    lying.artifacts[0].byte_len += 1;
    assert_eq!(check_artifact(&lying, 0, b"the window program"), mismatch);
    assert_eq!(
        check_artifact(&m, 1, b"the window program"),
        Err(Refusal::ArtifactMismatch { index: 1 })
    );
}
