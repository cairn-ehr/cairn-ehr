//! Golden byte pins for every on-disk constant this crate writes.
//!
//! WHY THIS FILE EXISTS (#500 slice 2a review). Every other test in this crate round-trips
//! through the SAME encoder/decoder pair, so a *mirrored* edit — one that changes the writer
//! and the reader together — is invisible to all of them. A mutation audit confirmed it: the
//! plane tags could be swapped (`Node => 2, Clinical => 1`), the chunk length prefix flipped
//! from big- to little-endian, `MEDIUM_MAGIC_V2` renamed, the `KIND_*` marker discriminants
//! swapped, the section's `prev_commitment`/`self_node_id_hex` field order reversed, and the
//! record flag bits reassigned — and all 57 tests stayed green. Each of those changes makes
//! every medium already written unreadable, or (worse) silently misread: swapped plane tags
//! route clinical events to the `node_event` door and node events to `event_log`.
//!
//! These are not round-trips. Each test asserts the EXACT bytes, hand-derived from the format
//! definition, so it fails if the writer changes even when the reader changes with it. This is
//! the only kind of test that can hold invariant 1 ("CAIRNB1 and CAIRNB2 are frozen") honest,
//! and the only kind that pins CAIRNB3's layout before any medium exists in the field.
//!
//! ON HOUSE RULE 6 (no hard-coded cryptographic material): the fixtures below deliberately
//! carry NO signatures and NO keys. `parse_container`/`take_section` do not verify signatures
//! — that is `verify_records`' job — so the framing can be pinned with plain placeholder
//! payloads. A golden medium containing a real signature would be exactly the byte-array
//! literal in a crypto context that rule 6 (issue #146) forbids.

use crate::container::{
    parse_any, parse_container, serialize_container, serialize_v3, Container, MediumImage,
    MEDIUM_MAGIC_V1, MEDIUM_MAGIC_V2, MEDIUM_MAGIC_V3,
};
use crate::marker::{SelfMarker, SELF_ATTEST_TYPE};
use crate::record::MediumRecord;
use crate::segment::{Plane, Segment};

/// Render bytes as lowercase hex so a failure diff is readable rather than a wall of decimal.
fn hex_of(b: &[u8]) -> String {
    hex::encode(b)
}

/// A record with no optional fields: `signed_bytes` only, flags 0, and an explicit seq.
/// Plain placeholder payload — see the rule-6 note in this module's header.
fn plain_record(payload: &[u8], source_seq: i64) -> MediumRecord {
    MediumRecord {
        signed_bytes: payload.to_vec(),
        attestation: None,
        attester_key: None,
        dek_wrapped: None,
        source_seq,
    }
}

// ---------------------------------------------------------------------------
// The magic headers.
// ---------------------------------------------------------------------------

/// The three magic headers are wire constants. Renaming one orphans every medium written
/// under it; the mutation audit showed `MEDIUM_MAGIC_V2 -> b"CAIRNX2\n"` passing the whole
/// suite, because every test that reads a V2 medium also wrote it.
#[test]
fn the_magic_headers_are_exactly_these_bytes() {
    assert_eq!(MEDIUM_MAGIC_V1, b"CAIRNB1\n", "CAIRNB1 magic changed");
    assert_eq!(MEDIUM_MAGIC_V2, b"CAIRNB2\n", "CAIRNB2 magic changed");
    assert_eq!(MEDIUM_MAGIC_V3, b"CAIRNB3\n", "CAIRNB3 magic changed");
    // All three are 8 bytes: every parser strips a fixed-width prefix before dispatching.
    for m in [MEDIUM_MAGIC_V1, MEDIUM_MAGIC_V2, MEDIUM_MAGIC_V3] {
        assert_eq!(m.len(), 8, "magic {m:?} is not 8 bytes");
    }
}

// ---------------------------------------------------------------------------
// CAIRNB1 / CAIRNB2 — frozen. These bytes may never change.
// ---------------------------------------------------------------------------

/// A CAIRNB1 medium is magic ++ `[u32 BE len][bytes]` frames, nothing else.
///
/// This pins the chunk length prefix as BIG-endian. A mirrored BE->LE flip in
/// `put_chunk`/`take_chunk` passes every round-trip test in the crate while making every
/// medium in the field unparseable.
#[test]
fn cairnb1_medium_is_exactly_these_bytes() {
    let events = vec![b"AB".to_vec(), b"CDE".to_vec()];
    let mut image = Vec::from(MEDIUM_MAGIC_V1);
    for e in &events {
        image.extend_from_slice(&(e.len() as u32).to_be_bytes());
        image.extend_from_slice(e);
    }
    assert_eq!(
        hex_of(&image),
        concat!(
            "434149524e42310a", // "CAIRNB1\n"
            "00000002",
            "4142", // frame len 2 (BE), "AB"
            "00000003",
            "434445", // frame len 3 (BE), "CDE"
        ),
        "the hand-derived CAIRNB1 layout drifted"
    );
    // And this build still reads it as the two events it holds.
    let got = parse_container(&image).expect("a golden CAIRNB1 medium must parse");
    assert_eq!(
        got,
        Container {
            self_marker: None,
            events
        }
    );
}

/// The CAIRNB2 marker-kind discriminants: 0 = none, 1 = unsigned, 2 = signed.
///
/// The mutation audit swapped `KIND_UNSIGNED`/`KIND_SIGNED` and the suite stayed green — yet
/// that swap makes a field medium's UNSIGNED marker decode as `Signed(garbage)`, so
/// `verify_self_attestation` returns `None` and restore silently degrades to "no marker"
/// with no error anywhere. These three fixtures pin each discriminant by its exact byte.
#[test]
fn cairnb2_marker_kind_discriminants_are_pinned() {
    let event = b"AB".to_vec();
    let frames = "00000002 4142".replace(' ', "");

    // KIND_NONE = 0
    let none = serialize_container(None, std::slice::from_ref(&event)).expect("fits the cap");
    assert_eq!(
        hex_of(&none),
        format!("434149524e42320a{}{}", "00", frames),
        "KIND_NONE is not 0x00"
    );

    // KIND_UNSIGNED = 1, followed by the id as a chunk.
    let unsigned = serialize_container(
        Some(&SelfMarker::Unsigned("n1".into())),
        std::slice::from_ref(&event),
    )
    .expect("fits the cap");
    assert_eq!(
        hex_of(&unsigned),
        format!("434149524e42320a{}{}{}", "01", "000000026e31", frames),
        "KIND_UNSIGNED is not 0x01, or the id is not a length-prefixed chunk"
    );

    // KIND_SIGNED = 2, followed by the attestation bytes as a chunk. Placeholder payload:
    // parse_container never verifies it (rule-6 note in the module header).
    let signed = serialize_container(
        Some(&SelfMarker::Signed(b"XY".to_vec())),
        std::slice::from_ref(&event),
    )
    .expect("fits the cap");
    assert_eq!(
        hex_of(&signed),
        format!("434149524e42320a{}{}{}", "02", "000000025859", frames),
        "KIND_SIGNED is not 0x02"
    );

    // Each still parses back to the marker it names — the discriminant and the reader agree.
    for (image, want) in [
        (&none, None),
        (&unsigned, Some(SelfMarker::Unsigned("n1".into()))),
        (&signed, Some(SelfMarker::Signed(b"XY".to_vec()))),
    ] {
        assert_eq!(
            parse_container(image)
                .expect("golden CAIRNB2 must parse")
                .self_marker,
            want
        );
    }
}

// ---------------------------------------------------------------------------
// CAIRNB3 — pinned now, before any medium exists in the field.
// ---------------------------------------------------------------------------

/// The on-disk plane tags. **The single most consequential constant in this crate.**
///
/// Swapping them (mirrored across `tag()` and `from_tag()`) passed all 57 tests in the
/// mutation audit, because every plane test round-trips. The consequence is total and
/// silent: every CAIRNB3 medium already written reads its node segments as clinical and its
/// clinical segments as node, so clinical events are routed to the `node_event` door and node
/// events to `event_log`.
#[test]
fn the_plane_tags_and_labels_are_wire_constants() {
    assert_eq!(Plane::Node.tag(), 1, "the node plane tag is not 1");
    assert_eq!(Plane::Clinical.tag(), 2, "the clinical plane tag is not 2");
    assert_eq!(Plane::from_tag(1), Plane::Node);
    assert_eq!(Plane::from_tag(2), Plane::Clinical);
    // A tag this build does not know is carried VERBATIM, never dropped and never guessed.
    assert_eq!(Plane::from_tag(0), Plane::Unknown(0));
    assert_eq!(Plane::from_tag(3), Plane::Unknown(3));
    assert_eq!(
        Plane::Unknown(3).tag(),
        3,
        "and re-serialises to the same tag"
    );
    assert!(!Plane::Unknown(3).is_known());

    // The label is signed INTO the attestation payload, so it is as much a wire constant as
    // the tag: changing it invalidates every segment attestation ever written. It is `None`
    // for an unknown plane — this build cannot know what a newer Cairn calls its own plane,
    // which is exactly why the numeric tag is the conjunct verification actually binds.
    assert_eq!(Plane::Node.label(), Some("node"));
    assert_eq!(Plane::Clinical.label(), Some("clinical"));
    assert_eq!(Plane::Unknown(3).label(), None);
}

/// The two in-container event types are wire constants: they are written into signed bodies
/// and matched on verify. Changing either makes every existing medium report
/// `AttestationInvalid` — fail-closed, but still a silent field break no test named.
#[test]
fn the_in_container_event_types_are_wire_constants() {
    assert_eq!(SELF_ATTEST_TYPE, "node.self_attested");
    assert_eq!(crate::attest::SEGMENT_ATTEST_TYPE, "node.segment_attested");
}

/// The record flag bits, and the order the optional fields follow the flags byte in.
///
/// The audit showed `FLAG_ATTESTATION`/`FLAG_DEK` swapping undetected. That reassignment
/// makes a field medium's human attestation token decode as a wrapped DEK and vice versa —
/// custody and authorship silently exchanged.
#[test]
fn record_flag_bits_and_field_order_are_pinned() {
    // FIRST, each bit ALONE. With all three fields present the flags byte is 0b111 whichever
    // bit means what, and `put_record` writes the optional fields in a fixed order that the
    // constants do not drive — so an all-three fixture cannot tell the bits apart, and a
    // swap of FLAG_ATTESTATION/FLAG_DEK survives it. One field at a time is what pins them.
    for (which, expect_flag, payload) in [
        ("attestation", "01", b"A"),
        ("attester_key", "02", b"K"),
        ("dek_wrapped", "04", b"D"),
    ] {
        let mut r = plain_record(b"E", 1);
        match which {
            "attestation" => r.attestation = Some(payload.to_vec()),
            "attester_key" => r.attester_key = Some(payload.to_vec()),
            _ => r.dek_wrapped = Some(payload.to_vec()),
        }
        let seg = Segment {
            plane: Plane::Node,
            index: 0,
            prev_commitment: String::new(),
            self_node_id_hex: String::new(),
            attestation: None,
            records: vec![r],
        };
        let image = serialize_v3(&[seg]).expect("fits the cap");
        // Built from labelled parts rather than one long literal, so a failure says which
        // field moved.
        let body = [
            "01",               // plane tag: node
            "00000000",         // segment index 0
            "00000000",         // prev_commitment: empty chunk
            "00000000",         // self_node_id_hex: empty chunk
            "00000000",         // segment attestation: empty chunk
            "00000001",         // record count 1
            "00000001",         // signed_bytes chunk length 1
            "45",               // "E"
            expect_flag,        // <-- the bit under test, alone
            "00000001",         // the optional field's chunk length 1
            &hex_of(payload),   // its payload
            "0000000000000001", // source_seq 1
        ]
        .concat();
        assert_eq!(
            hex_of(&image),
            format!("434149524e42330a{:08x}{body}", body.len() / 2),
            "the flags byte for a lone `{which}` must be 0x{expect_flag}: a swapped bit \
             assignment makes a field medium's authorship token decode as a wrapped DEK"
        );
    }

    // All three optional fields present, with distinguishable placeholder payloads.
    let r = MediumRecord {
        signed_bytes: b"E".to_vec(),
        attestation: Some(b"A".to_vec()),
        attester_key: Some(b"K".to_vec()),
        dek_wrapped: Some(b"D".to_vec()),
        source_seq: 1,
    };
    let seg = Segment {
        plane: Plane::Node,
        index: 0,
        prev_commitment: String::new(),
        self_node_id_hex: String::new(),
        attestation: None,
        records: vec![r],
    };
    let image = serialize_v3(&[seg]).expect("fits the cap");
    let body = concat!(
        "01",       // plane tag: node
        "00000000", // segment index 0 (BE)
        "00000000", // prev_commitment: empty chunk
        "00000000", // self_node_id_hex: empty chunk
        "00000000", // segment attestation: empty chunk
        "00000001", // record count 1 (BE)
        // -- the record --
        "00000001",
        "45", // signed_bytes chunk: "E"
        "07", // flags: ATTESTATION|ATTESTER_KEY|DEK = 0b111
        "00000001",
        "41", // attestation chunk: "A"
        "00000001",
        "4b", // attester_key chunk: "K"
        "00000001",
        "44",               // dek_wrapped chunk: "D"
        "0000000000000001", // source_seq 1 (BE, i64)
    );
    assert_eq!(
        hex_of(&image),
        format!("434149524e42330a{:08x}{body}", body.len() / 2),
        "the hand-derived CAIRNB3 section layout drifted"
    );
}

/// A minimal CAIRNB3 section, hand-derived field by field. Pins the plane tag's position,
/// the segment index as big-endian u32, the three chunks' ORDER
/// (`prev_commitment`, then `self_node_id_hex`, then the attestation — a mirrored swap of the
/// first two passed the whole suite), the record count as big-endian u32, and `source_seq` as
/// a big-endian i64.
#[test]
fn cairnb3_section_layout_is_exactly_these_bytes() {
    let seg = Segment {
        plane: Plane::Clinical,
        index: 1,
        prev_commitment: "p".into(),
        self_node_id_hex: "s".into(),
        attestation: None,
        records: vec![plain_record(b"AB", 258)],
    };
    let image = serialize_v3(std::slice::from_ref(&seg)).expect("fits the cap");
    let body = concat!(
        "02",       // plane tag: CLINICAL (not 1)
        "00000001", // segment index 1, big-endian
        "00000001",
        "70", // prev_commitment chunk: "p"  <-- FIRST
        "00000001",
        "73",       // self_node_id_hex chunk: "s" <-- SECOND
        "00000000", // attestation chunk: empty => None
        "00000001", // record count 1
        "00000002",
        "4142",             // signed_bytes chunk: "AB"
        "00",               // flags: no optional fields
        "0000000000000102", // source_seq 258 = 0x0102, big-endian i64
    );
    assert_eq!(
        hex_of(&image),
        format!("434149524e42330a{:08x}{body}", body.len() / 2),
        "the hand-derived CAIRNB3 section layout drifted"
    );

    // And it reads back as the segment it was written from.
    let MediumImage::V3(m) = parse_any(&image).expect("golden CAIRNB3 must parse") else {
        panic!("a CAIRNB3 image must not parse as legacy");
    };
    assert_eq!(m.segments, vec![seg]);
}

/// A negative `source_seq` round-trips as two's-complement big-endian. `source_seq` is an
/// `i64` because it mirrors Postgres `bigint`; nothing forbids a negative one reaching here,
/// and a codec that treated it as unsigned would corrupt the medium's cursor.
#[test]
fn a_negative_source_seq_survives_the_wire() {
    let seg = Segment {
        plane: Plane::Node,
        index: 0,
        prev_commitment: String::new(),
        self_node_id_hex: String::new(),
        attestation: None,
        records: vec![plain_record(b"", -1)],
    };
    let image = serialize_v3(std::slice::from_ref(&seg)).expect("fits the cap");
    assert!(
        hex_of(&image).ends_with("ffffffffffffffff"),
        "-1 must encode as two's-complement big-endian, got {}",
        hex_of(&image)
    );
    let MediumImage::V3(m) = parse_any(&image).expect("parses") else {
        panic!("not legacy")
    };
    assert_eq!(m.segments, vec![seg]);
}
