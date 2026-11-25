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
