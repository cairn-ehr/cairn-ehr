//! The steward's pure builders, checked by the verifier a terminal will actually run.

use crate::verify_tests::*;
use crate::*;

fn artifact_shell() -> Artifact {
    // digest/byte_len are placeholders: `assemble` must overwrite them from the bytes.
    Artifact {
        digest: [0; 32],
        byte_len: 0,
        ..window(b"")
    }
}

#[test]
fn an_assembled_bundle_verifies() {
    let (a, r1) = (key(1), key(11));
    let g = make_genesis("official", vec![signer(&a)], 1, vec![signer(&r1)], 1, &[&a]);
    let h = vec![genesis(&g).unwrap()];
    let bytes = b"the window program".to_vec();
    let bundle = assemble(&h, 3, vec![(artifact_shell(), bytes.clone())], &[&r1]);
    let (_, m) = verify_bundle(&h, &bundle.meta, 0).unwrap();
    assert_eq!(m.release_version, 3);
    assert_eq!(bundle.artifacts, vec![bytes.clone()]);
    check_artifact(&m, 0, &bytes).unwrap();
    // Tampering is caught: the digest was filled from the real bytes.
    assert!(check_artifact(&m, 0, b"other").is_err());
}

#[test]
fn a_rotation_built_here_advances_a_pin() {
    let (a, b, r1, r2) = (key(1), key(2), key(11), key(12));
    let g = make_genesis("official", vec![signer(&a)], 1, vec![signer(&r1)], 1, &[&a]);
    let h1 = vec![genesis(&g).unwrap()];
    // The caller's draft carries junk version/parent; `rotate` must fix both.
    let draft = doc(99, None, &[&b], 1, &[&r2], 1);
    let v2 = rotate(&h1[0], draft, &[&a]);
    let h2 = advance(&h1, std::slice::from_ref(&v2)).unwrap();
    assert_eq!(versions(&h2), [1, 2]);
    // A release signed by the NEW release set verifies against the pin's new history.
    let bundle = assemble(&h2, 1, vec![(artifact_shell(), b"x".to_vec())], &[&r2]);
    assert_eq!(bundle.meta.root_chain.len(), 2);
    // A terminal still pinned at v1 climbs via the bundle's own chain.
    verify_bundle(&h1, &bundle.meta, 0).unwrap();
}
