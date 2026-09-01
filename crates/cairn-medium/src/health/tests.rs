//! The four inputs that made this crate's LOOSE verdicts report a healthy medium that was
//! not one. Each is a regression pin on a confirmed defect, not a hypothetical.
//!
//! In every case the individual functions were honest and the COMPOSITE was a precise
//! untruth — issue #500's own signature. `assess` is what closes that, so these tests assert
//! on `sound()` and on the specific field that carries the bad news.

use super::*;
use crate::chain::SegmentFault;
use crate::container::{parse_any, serialize_v3, MediumImage};
use crate::segment::Segment;
use crate::testkit;

/// Parse a freshly-serialised segment list the way a real reader would, so these fixtures
/// exercise the same path a medium off disk takes.
fn parsed(segments: &[Segment]) -> crate::container::MediumV3 {
    let bytes = serialize_v3(segments).expect("fixture fits the cap");
    match parse_any(&bytes).expect("fixture parses") {
        MediumImage::V3(m) => m,
        MediumImage::Legacy(_) => panic!("a CAIRNB3 image must not parse as legacy"),
    }
}

/// **An 8-byte file `CAIRNB3\n` reported healthy.**
///
/// `chain_intact()` was `true` (no segments, so no faults) and `all_intact()` was `true`
/// (vacuously, at 0 of 0). A green cron check over an artifact that restores nothing —
/// exactly #502 item 2, one revision later.
///
/// Note what is deliberately NOT asserted: that `sound()` is false. An empty medium IS
/// internally consistent, and a node that genuinely holds no events yet must still be able to
/// write its first one. The fix is that `carries_nothing()` exists and is answered separately,
/// so a caller is handed both facts and cannot mistake one for the other.
#[test]
fn an_empty_medium_is_consistent_but_says_it_carries_nothing() {
    let bytes = serialize_v3(&[]).expect("an empty medium is writable");
    assert_eq!(bytes, b"CAIRNB3\n", "an empty medium is just its magic");
    let MediumImage::V3(m) = parse_any(&bytes).unwrap() else {
        panic!("not legacy")
    };
    let h = assess(&m);
    assert!(
        h.sound(),
        "an empty medium is internally consistent — that much was always true"
    );
    assert!(
        h.carries_nothing(),
        "and it must SAY it would restore nothing; reporting only `sound` over this is a \
         green light on an empty file, whose only other refusal comes at the disaster"
    );
    assert_eq!(h.records.total, 0);
}

/// **A medium missing an entire plane reported healthy.**
///
/// With the unknown-plane segment LAST, nothing chained off it, so there was no
/// `ChainBroken`; it lived in a separate `unknown` list that no verdict function consulted;
/// and `chain_intact()`, `all_intact()` and `self_id_from_chain` all reported success while a
/// whole plane was absent from every one of them. Invariant 6's own stated failure shape.
#[test]
fn a_medium_missing_a_whole_plane_is_not_sound() {
    let (m, _) = testkit::chain_of(3, 1);
    let mut bytes = serialize_v3(&m.segments).expect("fits the cap");
    // Relabel the LAST segment's plane tag to one this build does not know, the way a newer
    // Cairn writing a third plane would look from here.
    let mut offset = crate::container::MEDIUM_MAGIC_V3.len();
    for _ in 0..2 {
        let len = u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
        offset += 4 + len;
    }
    bytes[offset + 4] = 3;

    let MediumImage::V3(m2) = parse_any(&bytes).unwrap() else {
        panic!("not legacy")
    };
    let h = assess(&m2);
    assert!(
        !h.sound(),
        "a medium this build cannot fully read must never report sound"
    );
    assert!(
        h.needs_a_newer_build(),
        "and the remedy must be 'upgrade this node', not 'fetch another copy'"
    );
    assert!(
        h.records_in_unknown_planes > 0,
        "the records we could not route must be COUNTED, not silently omitted from totals"
    );
    let located = h.chain.faults.iter().any(|f| {
        matches!(
            f,
            SegmentFault::UnknownPlane {
                plane_tag: 3,
                position: 2,
                ..
            }
        )
    });
    assert!(
        located,
        "and the fault must name the tag AND where it sits: {:?}",
        h.chain.faults
    );
}

/// **A newer Cairn's plane in the MIDDLE used to read as a damaged medium.**
///
/// The unknown segment was dropped from the segment list, so the next known segment's
/// `prev_commitment` no longer matched and reported `ChainBroken` — telling an operator "this
/// medium is damaged" about a perfectly healthy one, and collapsing `verified_through` (and
/// so the watermark) back to before it. The chain must traverse it instead.
#[test]
fn an_unknown_plane_mid_chain_does_not_break_the_chain_for_what_follows() {
    // A newer Cairn writing a plane we do not know, captured with no signing key available
    // (principle 7: an unavailable key never blocks a backup). Built directly rather than by
    // byte-patching a signed segment, because patching a SIGNED segment's plane tag breaks
    // its attestation — correctly, and that case has its own test below.
    let (base, seqs) = testkit::verifiable_chain_of(3);
    let mut segments = base.segments.clone();
    segments[1] = Segment {
        plane: crate::segment::Plane::Unknown(3),
        attestation: None,
        ..segments[1].clone()
    };
    let m2 = parsed(&segments);
    let h = assess(&m2);
    assert!(
        !h.chain
            .faults
            .iter()
            .any(|f| matches!(f, SegmentFault::ChainBroken { .. })),
        "a newer Cairn's plane is NOT a broken chain — its records are readable as bytes, so \
         the commitment the next segment hangs from is still computable: {:?}",
        h.chain.faults
    );
    assert_eq!(
        h.chain.verified_through,
        Some(2),
        "and the segments after it must still verify, not become collateral damage"
    );
    assert_eq!(
        crate::chain::watermark(&m2, &h.chain, crate::segment::Plane::Clinical),
        Some(*seqs.last().unwrap()),
        "so the watermark does not regress and the next capture does not re-write everything"
    );
    assert!(h.needs_a_newer_build(), "but the gap is still reported");
    assert!(!h.sound());
}

/// Relabelling a SIGNED segment's plane tag is CAUGHT — it is not a free way to hide a
/// segment behind "this build cannot read that plane".
///
/// The attestation binds the numeric `plane_tag`, not only the human-legible label, precisely
/// so that an unknown plane loses no bind: for a plane this build cannot name, the label
/// conjunct is unknowable but the tag conjunct still holds. Without it, flipping one byte
/// would turn any segment into an unreadable-but-structurally-fine one.
#[test]
fn relabelling_a_signed_segments_plane_breaks_its_attestation() {
    let (m, _) = testkit::verifiable_chain_of(2);
    let mut bytes = serialize_v3(&m.segments).expect("fits the cap");
    bytes[crate::container::MEDIUM_MAGIC_V3.len() + 4] = 3; // first section's plane tag
    let MediumImage::V3(m2) = parse_any(&bytes).unwrap() else {
        panic!("not legacy")
    };
    let h = assess(&m2);
    assert!(
        h.chain
            .faults
            .iter()
            .any(|f| matches!(f, SegmentFault::AttestationInvalid { position: 0, .. })),
        "a signed segment whose plane tag was altered must fail its attestation: {:?}",
        h.chain.faults
    );
    assert!(!h.sound());
}

/// **A torn tail reported healthy.** `truncated_tail` lived on `MediumV3` and no verdict
/// function read it, so a medium that had lost its last append reported no fault at all.
#[test]
fn a_torn_tail_is_not_sound() {
    let (m, _) = testkit::chain_of(2, 1);
    let torn = testkit::medium_v3_torn(m.segments);
    let h = assess(&torn);
    assert!(
        h.chain.chain_intact(),
        "the chain over what survived is genuinely fine — that is why this was missed"
    );
    assert!(
        !h.sound(),
        "but a medium that lost its last append must never report sound"
    );
    assert!(h.truncated_tail, "and the caller is handed the reason");
}

/// **A tampered record in the LAST UNSIGNED segment reported healthy.**
///
/// An unsigned segment has no attestation to fail, and nothing chains off the last segment,
/// so `chain_report` sees nothing wrong. Only `verify_records` catches it — and nothing
/// forced a caller to run both. This is the sharpest of the four, because the crate's own
/// test suite tampered exactly this way and asserted only on `verify_records`.
#[test]
fn a_tampered_record_in_the_last_unsigned_segment_is_not_sound() {
    let mut m = testkit::unsigned_chain_of(2);
    m.segments[1].records[0].signed_bytes[0] ^= 0xff;

    let chain = crate::chain::chain_report(&m);
    assert!(
        chain.chain_intact(),
        "the CHAIN pass alone still reports intact — this is the trap, pinned so it cannot \
         be mistaken for a bug in the fix"
    );

    let h = assess(&m);
    assert!(
        !h.sound(),
        "the composed verdict must catch what the chain pass structurally cannot"
    );
    let loc = h
        .first_bad_record
        .expect("and the bad record must be LOCATED, not merely counted");
    assert_eq!(
        (loc.position, loc.ordinal_in_segment),
        (1, 0),
        "'record 1 of 2' is a count; 'segment at position 1, its first record' is a location"
    );
}

/// A clean, fully-signed medium is sound, carries something, and needs no newer build.
/// Without this, every assertion above could pass with `sound()` hard-coded to `false`.
#[test]
fn a_clean_medium_is_sound() {
    let (m, _) = testkit::verifiable_chain_of(3);
    let h = assess(&parsed(&m.segments));
    assert!(h.sound(), "faults: {:?}", h.chain.faults);
    assert!(!h.carries_nothing());
    assert!(!h.needs_a_newer_build());
    assert_eq!(h.first_bad_record, None);
    assert_eq!(h.records_in_unknown_planes, 0);
    assert_eq!((h.chain.signed_valid, h.chain.unsigned), (3, 0));
}
