//! Task 8 of #500 slice 2c (Erratum E2) — `backup::node_plane_events`, the ONE place that
//! answers "which events does the restore path apply?" for either medium revision.
//!
//! WHY THIS FILE EXISTS AND LANDS BEFORE THE WRITER SWITCHES. `restore` and `verify-backup`
//! both still read a medium through the LEGACY parser, which refuses CAIRNB3 outright. The
//! very next task in this slice makes `backup_to` write CAIRNB3, so without this file's
//! subject landing FIRST, that commit would leave the tree writing a medium neither command
//! can read back — a nightly backup that verifies red, and a restore that refuses an
//! operator's only copy. These tests pin the fix at the pure-function level, with no
//! database and no signing key: `node_plane_events` never verifies anything (that stays
//! `verify_events`'s job, run separately by both callers), it only decides WHICH bytes come
//! back, so a placeholder byte string is exactly as good a fixture as a real signed event —
//! and using one keeps this suite fast and DB-free.

use cairn_medium::{
    chain_report, parse_any, segment_commitment, serialize_container, serialize_v3, MediumImage,
    MediumRecord, Plane, Segment, SelfMarker,
};
use cairn_node::backup;

/// One record carrying `marker` as its "signed bytes". Not a real signed event — see the
/// module doc for why that is fine here — just a byte string distinct enough to prove WHICH
/// record came back and in what order.
fn record(marker: Vec<u8>, seq: i64) -> MediumRecord {
    MediumRecord {
        signed_bytes: marker,
        attestation: None,
        attester_key: None,
        dek_wrapped: None,
        source_seq: seq,
    }
}

/// One segment. `attestation: None` throughout this file: `node_plane_events` never
/// consults a segment's attestation (see its doc for why), and `parse_any`/`take_section`
/// round-trip an unsigned segment exactly as they would a signed one, so a real signing key
/// would buy this suite nothing but slower tests.
fn segment(plane: Plane, index: u32, prev: &str, records: Vec<MediumRecord>) -> Segment {
    Segment {
        plane,
        index,
        prev_commitment: prev.to_string(),
        self_node_id_hex: String::new(),
        attestation: None,
        records,
    }
}

/// A CAIRNB1/CAIRNB2 medium predates the plane split entirely — every event on it IS the
/// federation plane, and `node_plane_events` must hand every one of them back unchanged,
/// in the same order, forever. Media already in the field are unaffected by this task.
#[test]
fn a_legacy_medium_still_reads_exactly_as_before() {
    let events = vec![vec![1_u8, 2, 3], vec![4_u8, 5, 6, 7]];
    let marker = SelfMarker::Unsigned("deadbeef".into());
    let bytes = serialize_container(Some(&marker), &events).expect("fixture fits the cap");
    let image = parse_any(&bytes).expect("a CAIRNB2 medium parses via parse_any");

    assert_eq!(
        backup::node_plane_events(&image).unwrap(),
        events,
        "a legacy medium's events come back byte-for-byte, in file order"
    );
}

/// The compatibility obligation this whole task exists for: restore must behave EXACTLY as
/// it does today on the node plane of a CAIRNB3 medium. Restoring the clinical plane is
/// 2d's job, and this test is what stops someone reading 2c as having done it.
#[test]
fn a_v3_medium_yields_its_node_plane_and_ignores_the_clinical_one() {
    let node_events = vec![vec![10_u8, 20], vec![30_u8, 40]];
    let clinical_event = vec![99_u8, 99, 99];

    let node_records: Vec<MediumRecord> = node_events
        .iter()
        .cloned()
        .enumerate()
        .map(|(i, bytes)| record(bytes, i as i64))
        .collect();
    let node_seg = segment(Plane::Node, 0, "", node_records);
    let prev = segment_commitment(&node_seg.records);
    let clinical_seg = segment(
        Plane::Clinical,
        1,
        &prev,
        vec![record(clinical_event.clone(), 0)],
    );

    let bytes = serialize_v3(&[node_seg, clinical_seg]).expect("fixture fits the cap");
    let image = parse_any(&bytes).expect("a CAIRNB3 medium parses via parse_any");

    let got = backup::node_plane_events(&image).unwrap();
    assert_eq!(got, node_events, "the node plane comes back in file order");
    assert!(
        !got.contains(&clinical_event),
        "2c does not restore clinical events — 2d does"
    );
}

/// Pins the design choice documented beside `node_plane_events`: a Node-plane segment is
/// returned regardless of whether `chain::chain_report` could verify it. Without this test
/// the choice is only a comment — a future "helpful" fix could filter by
/// `verified_through` and this suite would stay green while quietly starting to drop
/// federation events an operator can see plainly in the file.
#[test]
fn a_node_plane_segment_after_a_broken_chain_link_is_still_returned() {
    let first = segment(Plane::Node, 0, "", vec![record(vec![1], 0)]);
    // A structurally BROKEN link: its declared `prev_commitment` does not match `first`'s
    // real commitment, so `chain::chain_report` marks the chain broken here and never
    // advances `verified_through` past position 0. It sits on the CLINICAL plane — the one
    // this call site does not even restore — which is the point: an unrelated fault must
    // not cost the federation plane a record.
    let broken = segment(
        Plane::Clinical,
        1,
        "not-the-real-commitment",
        vec![record(vec![2], 0)],
    );
    let broken_commitment = segment_commitment(&broken.records);
    // Well-formed and correctly chained onto `broken` — but because the chain already broke
    // one segment earlier, it sits PAST `verified_through` too.
    let after_break = segment(Plane::Node, 2, &broken_commitment, vec![record(vec![3], 0)]);

    let bytes =
        serialize_v3(&[first, broken, after_break]).expect("fixture fits the cap");
    let image = parse_any(&bytes).expect("a CAIRNB3 medium parses even with a broken chain");

    // Anti-vacuity: confirm the fixture really does break the chain before position 2, or
    // this test would pass without exercising the design decision at all.
    match &image {
        MediumImage::V3(m) => {
            let report = chain_report(m);
            assert_eq!(
                report.verified_through,
                Some(0),
                "fixture must break the chain at position 1, leaving position 2 unverified"
            );
        }
        MediumImage::Legacy(_) => panic!("CAIRNB3 magic must not parse as legacy"),
    }

    let got = backup::node_plane_events(&image).unwrap();
    assert_eq!(
        got,
        vec![vec![1_u8], vec![3_u8]],
        "both node-plane segments come back, including the one past the break"
    );
}
