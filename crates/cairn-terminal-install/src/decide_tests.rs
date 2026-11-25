//! `decide`: the installer's verdict on a staged release, before a byte is read into memory.
//! The cases mirror the verifier's own (cairn-release `verify_tests.rs`), driven through the
//! installer's gate, plus the three refusals only a terminal makes.
use super::*;
use crate::fixtures::*;
use cairn_release::staging::MAX_ARTIFACT;
use cairn_release::*;

fn versions(h: &[VerifiedRoot]) -> Vec<u64> {
    h.iter().map(|v| v.root().version).collect()
}

/// A good release 7 for this terminal, signed by v1's release key.
fn good() -> (Vec<VerifiedRoot>, ReleaseMeta) {
    let c = chain();
    let h = history(&[&c.v1]);
    let b = release(&h, 7, vec![(window_for(&this()), script(GEN, 7))], &r1());
    (h, b.meta)
}

#[test]
fn a_verified_release_names_its_program_for_this_terminal() {
    let c = chain();
    let h = history(&[&c.v1]);
    let elsewhere = Target {
        platform: "raspbian-11".into(),
        ..this()
    };
    let mut firmware = window_for(&this());
    firmware.kind = "firmware".into();
    let b = release(
        &h,
        7,
        vec![
            (window_for(&elsewhere), b"other".to_vec()),
            (firmware, b"firmware".to_vec()),
            (window_for(&this()), script(GEN, 7)),
        ],
        &r1(),
    );
    let (h2, m, index) = decide(&h, &b.meta, 10, 0, &this(), None).unwrap();
    assert_eq!((versions(&h2), m.release_version, index), (vec![1], 7, 2));
}

#[test]
fn an_incident_refuses_everything() {
    let (h, meta) = good();
    let fork = Refusal::Fork { version: 2 };
    assert_eq!(
        decide(&h, &meta, 10, 0, &this(), Some(2)),
        Err(fork.clone())
    );
    // Before any other check, a size one included.
    assert_eq!(
        decide(&h, &meta, MAX_ARTIFACT + 1, 0, &this(), Some(2)),
        Err(fork)
    );
}

#[test]
fn an_oversized_file_is_too_large() {
    let (h, meta) = good();
    assert_eq!(
        decide(&h, &meta, MAX_ARTIFACT + 1, 0, &this(), None),
        Err(Refusal::TooLarge)
    );
    // Size comes before the pin: even a terminal with no channel says "too large".
    assert_eq!(
        decide(&[], &meta, MAX_ARTIFACT + 1, 0, &this(), None),
        Err(Refusal::TooLarge)
    );
    assert_eq!(
        decide(&[], &meta, MAX_ARTIFACT, 0, &this(), None),
        Err(Refusal::NoChannel)
    );
}

#[test]
fn a_platform_mismatch_names_this_terminal() {
    let c = chain();
    let h = history(&[&c.v1]);
    let elsewhere = Target {
        platform: "raspbian-11".into(),
        ..this()
    };
    let mut other_kind = window_for(&this());
    other_kind.kind = "firmware".into();
    for artifacts in [
        vec![(window_for(&elsewhere), b"p".to_vec())],
        vec![(other_kind, b"p".to_vec())],
        vec![],
    ] {
        let b = release(&h, 7, artifacts, &r1());
        assert_eq!(
            decide(&h, &b.meta, 1, 0, &this(), None),
            Err(Refusal::NoProgramForThisTerminal {
                os: "linux".into(),
                arch: "aarch64".into(),
                platform: "debian-12".into(),
            })
        );
    }
}

#[test]
fn no_pin_is_no_channel() {
    let (_, meta) = good();
    assert_eq!(
        decide(&[], &meta, 10, 0, &this(), None),
        Err(Refusal::NoChannel)
    );
}

#[test]
fn a_two_rotation_gap_verifies_from_an_old_pin() {
    let c = chain();
    let full = history(&[&c.v1, &c.v2, &c.v3]);
    let mut meta = release(&full, 7, vec![(window_for(&this()), script(GEN, 7))], &r2()).meta;
    meta.root_chain = vec![c.v2.clone(), c.v3.clone()]; // the roots after the pin, as staged
    let (h2, _, _) = decide(&history(&[&c.v1]), &meta, 10, 0, &this(), None).unwrap();
    assert_eq!(versions(&h2), [1, 2, 3]);
}

#[test]
fn a_missing_version_is_a_chain_gap() {
    let c = chain();
    let full = history(&[&c.v1, &c.v2, &c.v3]);
    let mut meta = release(&full, 7, vec![(window_for(&this()), script(GEN, 7))], &r2()).meta;
    meta.root_chain = vec![c.v3.clone()];
    assert_eq!(
        decide(&history(&[&c.v1]), &meta, 10, 0, &this(), None),
        Err(Refusal::ChainGap {
            pinned: 1,
            needs: 3
        })
    );
}

#[test]
fn two_successors_are_a_fork() {
    let c = chain();
    let (h, mut meta) = good();
    meta.root_chain = vec![c.v2.clone(), c.rival_v2.clone()];
    assert_eq!(
        decide(&h, &meta, 10, 0, &this(), None),
        Err(Refusal::Fork { version: 2 })
    );
}

#[test]
fn a_rotated_out_release_key_is_refused() {
    let c = chain();
    let h = history(&[&c.v1, &c.v2]);
    let meta = release(&h, 7, vec![(window_for(&this()), script(GEN, 7))], &r1()).meta;
    assert_eq!(
        decide(&h, &meta, 10, 0, &this(), None),
        Err(Refusal::ReleaseSignaturesShort { have: 0, need: 1 })
    );
}

#[test]
fn a_release_not_newer_is_refused() {
    let (h, meta) = good();
    for installed in [7, 8] {
        assert_eq!(
            decide(&h, &meta, 10, installed, &this(), None),
            Err(Refusal::NotNewer {
                installed,
                offered: 7
            })
        );
    }
}

#[test]
fn a_root_from_another_channel_is_the_wrong_channel() {
    let (h, mut meta) = good();
    let a = a();
    let community = make_genesis("community", vec![signer(&a)], 1, vec![signer(&a)], 1, &[&a]);
    meta.root_chain = vec![community];
    assert_eq!(
        decide(&h, &meta, 10, 0, &this(), None),
        Err(Refusal::WrongChannel {
            pinned: "official".into(),
            offered: "community".into()
        })
    );
}

#[test]
fn the_new_program_must_answer_exactly_its_signed_lines() {
    let a = window_for(&this());
    let line = format!("{GEN}\n");
    assert!(answers_match(&a, 7, Some(&line), Some("7\n")));
    assert!(!answers_match(&a, 7, Some("85 00\n"), Some("7\n")));
    assert!(!answers_match(&a, 7, Some(&line), Some("6\n")));
    assert!(!answers_match(&a, 7, Some(&line), Some("07\n")));
    assert!(!answers_match(&a, 7, None, Some("7\n")));
    assert!(!answers_match(&a, 7, Some(&line), None));
    let unnamed = Artifact {
        generation_line: None,
        ..a
    };
    assert!(!answers_match(&unnamed, 7, Some(&line), Some("7\n")));
}

/// Ruling R18: only what a corrupt transfer can cause is fetched again.
#[test]
fn only_transit_damage_is_fetched_again() {
    assert!(!refused_for_good(&Refusal::Malformed("x".into())));
    assert!(!refused_for_good(&Refusal::ArtifactMismatch { index: 0 }));
    for r in [
        Refusal::NoChannel,
        Refusal::Fork { version: 2 },
        Refusal::NotNewer {
            installed: 7,
            offered: 7,
        },
        Refusal::ReleaseSignaturesShort { have: 0, need: 1 },
        Refusal::TooLarge,
        Refusal::WontStart,
    ] {
        assert!(refused_for_good(&r), "{r:?}");
    }
}

#[test]
fn complete_is_a_decimal_u64_only() {
    assert_eq!(parse_complete(b"7"), Some(7));
    assert_eq!(parse_complete(b"18446744073709551615"), Some(u64::MAX));
    for bad in [
        &b""[..],
        b"+7",
        b" 7",
        b"7\n",
        b"-1",
        b"0x7",
        b"seven",
        b"18446744073709551616",
    ] {
        assert_eq!(parse_complete(bad), None, "{bad:?}");
    }
}
