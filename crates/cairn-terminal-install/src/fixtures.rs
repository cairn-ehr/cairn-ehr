//! Test fixtures: a three-root channel and signed releases, all minted at runtime (house
//! rule 6: keys are derived with `std::array::from_fn`, never written as literals).
use cairn_event::SigningKey;
use cairn_release::staging::Target;
use cairn_release::*;

/// A runtime-derived Ed25519 key.
pub fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&std::array::from_fn(|i| (i as u8).wrapping_mul(29) ^ seed))
}

pub fn signer(sk: &SigningKey) -> Signer {
    let pubkey = sk.verifying_key().to_bytes();
    Signer {
        kid: pubkey[..4].to_vec(),
        pubkey,
        custody_note: "test token".into(),
    }
}

/// The terminal every test is: a Raspberry Pi on Debian 12.
pub fn this() -> Target {
    Target {
        os: "linux".into(),
        arch: "aarch64".into(),
        platform: "debian-12".into(),
    }
}

/// The generation line every test program prints.
pub const GEN: &str = "86 00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

/// A `window` artifact for `t` (digest and length are filled in by `assemble`).
pub fn window_for(t: &Target) -> Artifact {
    Artifact {
        kind: "window".into(),
        digest: [0; 32],
        byte_len: 0,
        os: Some(t.os.clone()),
        arch: Some(t.arch.clone()),
        platform: Some(t.platform.clone()),
        generation_line: Some(GEN.into()),
    }
}

/// Root keys a, b (v1) and c, d (v2, v3); release keys r1 (v1) and r2 (v2, v3).
pub fn a() -> SigningKey {
    key(1)
}
pub fn r1() -> SigningKey {
    key(11)
}
pub fn r2() -> SigningKey {
    key(12)
}

/// v1 (root a,b ≥2, release r1); v2 (root c,d ≥2, release r2) signed by a,b; v3 (the same
/// sets) signed by c,d. `rival_v2` is a second, differently-signed-for v2 that also verifies.
pub struct Chain {
    pub v1: SignedRoot,
    pub v2: SignedRoot,
    pub v3: SignedRoot,
    pub rival_v2: SignedRoot,
}

fn next(roots: &[&SigningKey], release: &SigningKey) -> TrustRoot {
    TrustRoot {
        channel_id: "official".into(),
        version: 0,
        root_signers: roots.iter().map(|k| signer(k)).collect(),
        root_threshold: 2,
        release_signers: vec![signer(release)],
        release_threshold: 1,
        prev_root_hash: None,
    }
}

pub fn chain() -> Chain {
    let (a, b, c, d) = (a(), key(2), key(3), key(4));
    let v1 = make_genesis(
        "official",
        vec![signer(&a), signer(&b)],
        2,
        vec![signer(&r1())],
        1,
        &[&a, &b],
    );
    let h1 = genesis(&v1).unwrap();
    let v2 = rotate(&h1, next(&[&c, &d], &r2()), &[&a, &b]);
    let rival_v2 = rotate(&h1, next(&[&c, &d], &r1()), &[&a, &b]);
    let h2 = advance(std::slice::from_ref(&h1), std::slice::from_ref(&v2)).unwrap();
    let v3 = rotate(h2.last().unwrap(), next(&[&c, &d], &r2()), &[&c, &d]);
    Chain {
        v1,
        v2,
        v3,
        rival_v2,
    }
}

/// Verify `roots` from genesis, as the installer does with its stored pin.
pub fn history(roots: &[&SignedRoot]) -> Vec<VerifiedRoot> {
    let g = genesis(roots[0]).unwrap();
    let rest: Vec<SignedRoot> = roots[1..].iter().map(|r| (*r).clone()).collect();
    advance(&[g], &rest).unwrap()
}

/// Release `version` carrying `artifacts`, signed by `with` under `h`'s newest root. Its
/// `root_chain` is the whole of `h`; a test replaces it to model what the window stages.
pub fn release(
    h: &[VerifiedRoot],
    version: u64,
    artifacts: Vec<(Artifact, Vec<u8>)>,
    with: &SigningKey,
) -> BundleFile {
    assemble(h, version, artifacts, &[with])
}

/// A `#!/bin/sh` "program" that answers the two print flags as a window would.
pub fn script(generation: &str, release: u64) -> Vec<u8> {
    format!(
        "#!/bin/sh\ncase \"$1\" in\n  --print-schema-generation) echo '{generation}' ;;\n  \
         --print-release-version) echo {release} ;;\n  *) exit 2 ;;\nesac\n"
    )
    .into_bytes()
}
