use crate::*;

/// Runtime-derived filler (house rule 6: no literal crypto material).
fn arr(seed: u8) -> [u8; 32] {
    std::array::from_fn(|i| seed.wrapping_add(i as u8))
}

fn signer(seed: u8) -> Signer {
    Signer {
        kid: vec![seed; 4],
        pubkey: arr(seed),
        custody_note: "hw token".into(),
    }
}

fn root() -> TrustRoot {
    TrustRoot {
        channel_id: "official".into(),
        version: 2,
        root_signers: vec![signer(1), signer(2)],
        root_threshold: 2,
        release_signers: vec![signer(3)],
        release_threshold: 1,
        prev_root_hash: Some(arr(9)),
    }
}

fn meta() -> ReleaseMeta {
    let sr = SignedRoot {
        payload: encode(&root()),
        signatures: vec![vec![1, 2], vec![3]],
    };
    let m = Manifest {
        channel_id: "official".into(),
        release_version: 7,
        root_version_ref: 2,
        artifacts: vec![Artifact {
            kind: "window".into(),
            digest: arr(5),
            byte_len: 3,
            os: Some("linux".into()),
            arch: Some("aarch64".into()),
            platform: Some("debian-12".into()),
            generation_line: None,
        }],
    };
    ReleaseMeta {
        root_chain: vec![sr],
        manifest: encode(&m),
        release_signatures: vec![vec![7; 64]],
        inclusion_proof: None,
    }
}

#[test]
fn every_document_round_trips() {
    let r = root();
    assert_eq!(decode::<TrustRoot>(&encode(&r)).unwrap(), r);
    let m = meta();
    assert_eq!(decode::<ReleaseMeta>(&encode(&m)).unwrap(), m);
    let sr = m.root_chain[0].clone();
    assert_eq!(sr.root().unwrap(), r);
    let man: Manifest = decode(&m.manifest).unwrap();
    assert_eq!(decode::<Manifest>(&encode(&man)).unwrap(), man);
    let b = BundleFile {
        meta: m,
        artifacts: vec![vec![1, 2, 3]],
    };
    assert_eq!(decode::<BundleFile>(&encode(&b)).unwrap(), b);
}

#[test]
fn the_root_hash_covers_the_payload_only() {
    let a = SignedRoot {
        payload: encode(&root()),
        signatures: vec![vec![1]],
    };
    let b = SignedRoot {
        payload: a.payload.clone(),
        signatures: vec![vec![2], vec![3]],
    };
    assert_eq!(a.hash(), b.hash());
    let c = SignedRoot {
        payload: vec![0],
        signatures: vec![],
    };
    assert_ne!(a.hash(), c.hash());
}

#[test]
fn garbage_decodes_to_malformed() {
    assert!(matches!(
        decode::<TrustRoot>(&[0xff, 0x00, 0x13]),
        Err(Refusal::Malformed(_))
    ));
    assert!(matches!(
        decode::<BundleFile>(&[]),
        Err(Refusal::Malformed(_))
    ));
}

#[test]
fn there_is_no_official_root_before_the_ceremony() {
    assert!(official_root().is_none());
}

#[test]
fn no_channel_says_who() {
    assert_eq!(
        Refusal::NoChannel.sentence(Who::Node),
        "No release channel is set up on this node yet, so it accepts no update. Whoever set it up must install the channel's first trust file."
    );
    assert_eq!(
        Refusal::NoChannel.sentence(Who::Terminal),
        "No release channel is set up on this terminal yet, so it accepts no update. Whoever set it up must install the channel's first trust file."
    );
    assert_eq!(
        Refusal::Malformed("x".into()).sentence(Who::Node),
        "This update file is damaged or is not a Cairn update; fetch it again."
    );
}

/// True if `hay` contains `needle` as a contiguous run.
fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

fn artifact(digest: [u8; 32]) -> Artifact {
    Artifact {
        kind: "window".into(),
        digest,
        byte_len: 3,
        os: None,
        arch: None,
        platform: None,
        generation_line: None,
    }
}

#[test]
fn every_byte_field_is_a_cbor_byte_string() {
    // [u8; 32]: major type 2, length 32 => header 0x58 0x20, then the bytes.
    let d = arr(5);
    let mut want = vec![0x58, 0x20];
    want.extend_from_slice(&d);
    assert!(contains(&encode(&artifact(d)), &want));
    // Vec<u8> (Signer.kid, 4 bytes): header 0x44.
    let s = signer(7);
    let mut want = vec![0x44];
    want.extend_from_slice(&s.kid);
    assert!(contains(&encode(&s), &want));
    // Option<[u8; 32]> (prev_root_hash) is a byte string too.
    let mut want = vec![0x58, 0x20];
    want.extend_from_slice(&arr(9));
    assert!(contains(&encode(&root()), &want));
    // Some(Vec<u8>) inclusion proof: 3 bytes => 0x43.
    let mut m = meta();
    m.inclusion_proof = Some(vec![1, 2, 3]);
    assert!(contains(&encode(&m), &[0x43, 1, 2, 3]));
}

#[test]
fn a_short_digest_is_malformed() {
    let mut bytes = encode(&artifact(arr(5)));
    // Rewrite the 32-byte string header to 31 bytes and drop one byte.
    let at = bytes.windows(2).position(|w| w == [0x58, 0x20]).unwrap();
    bytes[at + 1] = 0x1f;
    bytes.remove(at + 2);
    assert!(matches!(
        decode::<Artifact>(&bytes),
        Err(Refusal::Malformed(_))
    ));
}

#[test]
fn trailing_bytes_are_malformed() {
    let mut bytes = encode(&root());
    bytes.push(0);
    assert!(matches!(
        decode::<TrustRoot>(&bytes),
        Err(Refusal::Malformed(_))
    ));
}

#[test]
fn encoding_is_deterministic_and_proof_round_trips() {
    let mut m = meta();
    m.inclusion_proof = Some(vec![9; 40]);
    assert_eq!(encode(&m), encode(&m.clone()));
    assert_eq!(decode::<ReleaseMeta>(&encode(&m)).unwrap(), m);
}
