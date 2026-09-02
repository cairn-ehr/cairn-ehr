//! Magic dispatch and the on-disk framing of every medium revision: the CAIRNB1/CAIRNB2 magic
//! headers, the marker-kind discriminant, the [`Container`] parse result, and the
//! serialize/parse pair that round-trips a medium image. This is the "outermost" format layer —
//! it decides which revision a byte slice claims to be, writes and reads the marker block
//! itself (see `put_marker`, below, for why that writer lives here rather than in `marker`),
//! and hands the event frames to `chunk`'s primitive.

use crate::chunk::{put_chunk, take_chunk};
use crate::error::BackupError;
use crate::marker::SelfMarker;
use crate::segment::{put_segment, take_section, Segment};

/// Magic header for the original marker-less medium (ADR-0026 slice B). Kept for backward
/// compatibility: such a medium parses to events with `self_marker == None`.
pub const MEDIUM_MAGIC_V1: &[u8] = b"CAIRNB1\n";
/// Magic header for the self-marked medium (issue #53). Carries a marker block before the
/// event frames. Distinct from the keystore's `CAIRNK1` so the artifacts can never be confused.
pub const MEDIUM_MAGIC_V2: &[u8] = b"CAIRNB2\n";

/// Marker kind discriminant bytes (first byte of the CAIRNB2 marker block). File-private: the
/// only writer (`put_marker`) and the only reader (`parse_container`, below) both live in
/// this file, so nothing outside it needs to see these bytes.
const KIND_NONE: u8 = 0;
const KIND_UNSIGNED: u8 = 1;
const KIND_SIGNED: u8 = 2;

/// A parsed backup medium: the (optional) self-marker plus the signed event set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Container {
    /// `None` for a legacy CAIRNB1 medium (no marker) or a backup taken before enrollment.
    pub self_marker: Option<SelfMarker>,
    pub events: Vec<Vec<u8>>,
}

// ---------------------------------------------------------------------------
// Container serialize / parse (pure).
// ---------------------------------------------------------------------------

/// Serialize a self-marker into its kind-tagged block. Pure. Lives here, not in `marker.rs`,
/// because it is the WRITER of the discriminant `parse_container` (below) READS — a writer and
/// reader of one wire tag scheme belong in the same file, so the two can never drift apart.
/// `marker.rs` also carries a FROZEN header ("CAIRNB2 only ... Do not extend this module"):
/// container-wire-framing glue has no business living in a module future work is told to leave
/// alone. `serialize_container` is this function's only caller and lives right below it.
fn put_marker(out: &mut Vec<u8>, marker: Option<&SelfMarker>) -> Result<(), BackupError> {
    match marker {
        None => out.push(KIND_NONE),
        Some(SelfMarker::Unsigned(id)) => {
            out.push(KIND_UNSIGNED);
            put_chunk(out, id.as_bytes())?;
        }
        Some(SelfMarker::Signed(att)) => {
            out.push(KIND_SIGNED);
            put_chunk(out, att)?;
        }
    }
    Ok(())
}

/// Serialize a full CAIRNB2 container: magic ++ marker block ++ event frames. Pure. The event
/// order is preserved for legibility but is set-union-independent on restore (convergence is
/// by content-address).
pub fn serialize_container(
    marker: Option<&SelfMarker>,
    events: &[Vec<u8>],
) -> Result<Vec<u8>, BackupError> {
    let mut out = Vec::with_capacity(MEDIUM_MAGIC_V2.len() + 1 + 32 * events.len());
    out.extend_from_slice(MEDIUM_MAGIC_V2);
    put_marker(&mut out, marker)?;
    for e in events {
        put_chunk(&mut out, e)?;
    }
    Ok(out)
}

/// Parse a medium image into its marker + event set. Handles BOTH formats: a CAIRNB2 medium
/// yields its marker; a legacy CAIRNB1 medium yields `self_marker: None`. Errors (never
/// panics) on bad magic, an unknown marker kind, or a truncated frame.
pub fn parse_container(bytes: &[u8]) -> Result<Container, BackupError> {
    if let Some(rest) = bytes.strip_prefix(MEDIUM_MAGIC_V2) {
        let (&kind, mut rest) = rest
            .split_first()
            .ok_or_else(|| BackupError::Damaged("CAIRNB2 medium missing marker kind".into()))?;
        let self_marker = match kind {
            KIND_NONE => None,
            KIND_UNSIGNED => {
                let (id, r) = take_chunk(rest)?;
                rest = r;
                let id = std::str::from_utf8(id)
                    .map_err(|_| BackupError::Damaged("unsigned marker is not UTF-8".into()))?;
                Some(SelfMarker::Unsigned(id.to_string()))
            }
            KIND_SIGNED => {
                let (att, r) = take_chunk(rest)?;
                rest = r;
                Some(SelfMarker::Signed(att.to_vec()))
            }
            other => {
                return Err(BackupError::Damaged(format!("unknown marker kind {other}")));
            }
        };
        let events = take_frames(rest)?;
        Ok(Container {
            self_marker,
            events,
        })
    } else if let Some(rest) = bytes.strip_prefix(MEDIUM_MAGIC_V1) {
        Ok(Container {
            self_marker: None,
            events: take_frames(rest)?,
        })
    } else if bytes.starts_with(MEDIUM_MAGIC_V3) {
        // Say what it actually IS. This function reads the legacy revisions only, and a
        // CAIRNB3 medium reaching it is a caller on the wrong code path, not a bad file —
        // telling an operator "this is not a backup medium" about a perfectly good CAIRNB3
        // medium is the wrong-remedy failure the error taxonomy exists to prevent.
        Err(BackupError::UnsupportedByThisBuild(
            "this is a CAIRNB3 medium; parse_container reads CAIRNB1/CAIRNB2 only — use \
             parse_any"
                .into(),
        ))
    } else {
        Err(BackupError::NotAMedium(
            "missing CAIRNB1/CAIRNB2 magic header".into(),
        ))
    }
}

/// Read the trailing repeated event frames until a clean end-of-buffer (peer-stream EOF style).
fn take_frames(mut rest: &[u8]) -> Result<Vec<Vec<u8>>, BackupError> {
    let mut events = Vec::new();
    while !rest.is_empty() {
        let (frame, r) = take_chunk(rest)?;
        events.push(frame.to_vec());
        rest = r;
    }
    Ok(events)
}

/// Parse just the event set from a medium image (either format). For callers that only verify
/// signatures and do not care about the marker (e.g. `verify-backup`).
pub fn parse_medium(bytes: &[u8]) -> Result<Vec<Vec<u8>>, BackupError> {
    Ok(parse_container(bytes)?.events)
}

// ---------------------------------------------------------------------------
// CAIRNB3 — the append-only, two-plane medium (issue #500 slice 2a).
// ---------------------------------------------------------------------------

/// Magic for the append-only, two-plane medium (issue #500 slice 2a). Distinct from
/// CAIRNB1/CAIRNB2 so a reader never has to guess, and from the keystore's CAIRNK1 and the
/// local-state export's CAIRNL1 so the four artifacts can never be confused.
pub const MEDIUM_MAGIC_V3: &[u8] = b"CAIRNB3\n";

/// A parsed CAIRNB3 image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediumV3 {
    /// Every complete section, in file order — INCLUDING those whose plane tag this build
    /// does not recognise, which carry [`crate::segment::Plane::Unknown`].
    ///
    /// Unknown-plane segments used to live in a separate `unknown` list, which stranded them:
    /// the chain walk iterated only this field, so an unknown segment broke the chain for
    /// everything after it, and `chain_report` never saw it at all. Keeping one list is what
    /// lets the single global chain traverse a newer Cairn's plane, and what lets
    /// `chain::chain_report` raise a located `UnknownPlane` fault instead of silently
    /// reporting a medium sound while a whole plane is missing (#500 slice 2a review).
    pub segments: Vec<Segment>,
    /// The final section was cut short: an interrupted append. Everything before it is
    /// intact. Remedy: run the backup again; the watermark did not advance past the last
    /// verified segment, so the lost increment is re-captured.
    pub truncated_tail: bool,
    /// The offset, in the ORIGINAL bytes handed to [`parse_any`], one past the last
    /// COMPLETE section: `bytes[..complete_bytes]` is the intact prefix (magic included);
    /// `bytes[complete_bytes..]` is empty (clean parse) or the torn remnant.
    ///
    /// I4 (#500 final review): a WRITER recovering from a torn medium MUST
    /// `medium.truncate(complete_bytes)` before appending — otherwise the torn remnant
    /// becomes the next section's `[u32 length]` prefix and parsing stops there FOREVER,
    /// silently orphaning every later backup.
    pub complete_bytes: usize,
}

/// Either revision of the format, as parsed. Legacy media keep their exact prior code path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediumImage {
    /// CAIRNB1 / CAIRNB2 — a head marker plus bare event frames.
    Legacy(Container),
    /// CAIRNB3 — chained, plane-tagged segments.
    V3(MediumV3),
}

/// Serialize a fresh CAIRNB3 image. Pure. Returns `Result` (I6, #500 final review): each
/// segment is refused at the source, per `put_segment`'s doc, rather than silently writing
/// one `take_section` could never read back.
pub fn serialize_v3(segments: &[Segment]) -> Result<Vec<u8>, BackupError> {
    let mut out = Vec::from(MEDIUM_MAGIC_V3);
    for seg in segments {
        put_segment(&mut out, seg)?;
    }
    Ok(out)
}

/// Append one segment to an existing CAIRNB3 image, in place.
///
/// Byte-wise append: nothing already in `medium` is read or rewritten BY THIS CALL, which is
/// what makes the APPEND ITSELF cost O(new records) — narrower than "a capture costs
/// O(new records)" (an earlier version of this doc overclaimed exactly that — minor, #500
/// final review): the caller must first PARSE THE WHOLE IMAGE, an O(medium size) operation
/// with no streaming parse yet, to learn the watermark and predecessor before it can even
/// build the segment to append. So the append is O(new records); the capture around it is
/// not, until a streaming parse exists (deferred).
///
/// The caller writing this to disk owes the durability half — `write` then `sync_all()`
/// BEFORE advancing any health record (slice 2c; this crate does no I/O). Returns `Result`
/// (I6): see `put_segment`'s doc for why an over-cap segment is refused, not silently
/// written unreadable.
pub fn append_segment(medium: &mut Vec<u8>, seg: &Segment) -> Result<(), BackupError> {
    put_segment(medium, seg)
}

/// Parse a medium of any revision, dispatching on its magic.
///
/// `parse_container`/`serialize_container` (above) are untouched: a CAIRNB1/CAIRNB2 image
/// takes its exact prior code path, so existing media in the field are unaffected by this
/// function's existence. Only the CAIRNB3 magic is new behaviour.
pub fn parse_any(bytes: &[u8]) -> Result<MediumImage, BackupError> {
    let Some(mut rest) = bytes.strip_prefix(MEDIUM_MAGIC_V3) else {
        // Not CAIRNB3 — hand it to the untouched legacy parser, which refuses anything
        // that is not CAIRNB1/CAIRNB2.
        return Ok(MediumImage::Legacy(parse_container(bytes)?));
    };
    let mut segments = Vec::new();
    let mut truncated_tail = false;
    while !rest.is_empty() {
        match take_section(rest)? {
            None => {
                // A torn tail: fewer bytes remain than the next section claims. Everything
                // read so far is complete and kept; the loss is flagged, never silent.
                truncated_tail = true;
                break;
            }
            Some((seg, next)) => {
                segments.push(seg);
                rest = next;
            }
        }
    }
    // Bytes consumed == the offset one past the last COMPLETE section (I4): `rest` here is
    // either empty or the torn remnant either way. See `MediumV3::complete_bytes`'s doc.
    let complete_bytes = bytes.len() - rest.len();
    Ok(MediumImage::V3(MediumV3 {
        segments,
        truncated_tail,
        complete_bytes,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attest::{segment_commitment, tests_support};
    use crate::marker::build_self_attestation;
    use crate::segment::Plane;
    use crate::testkit::{enroll, kid, node_id, sk};

    /// This module's shorthand for building a signed segment fixture: `n` distinct records
    /// under one signed segment, mirroring `attest.rs`'s own `signed_segment` test helper
    /// so the two files don't each grow a slightly different way of doing the same thing.
    /// The lineage is fixed at 1 — none of this module's tests splice segments between two
    /// DIFFERENT media, which is the only case where distinct lineages are load-bearing (see
    /// `tests_support::distinct_record`'s doc comment).
    fn signed_segment(
        sk: &cairn_event::SigningKey,
        self_id: &str,
        plane: Plane,
        index: u32,
        prev: &str,
        n: usize,
    ) -> Segment {
        let records = (0..n)
            .map(|i| tests_support::distinct_record(1, i as u8))
            .collect();
        tests_support::signed(sk, self_id, plane, index, prev, records)
    }

    #[test]
    fn container_roundtrips_unsigned_marker_and_events() {
        let k = sk();
        let g = enroll(&k, "Self");
        let events = vec![g.clone()];
        let marker = SelfMarker::Unsigned(node_id(&g));
        let image = serialize_container(Some(&marker), &events).expect("fixture fits the cap");
        assert!(
            image.starts_with(MEDIUM_MAGIC_V2),
            "self-marked medium carries CAIRNB2 magic"
        );
        let got = parse_container(&image).unwrap();
        assert_eq!(got.self_marker, Some(marker));
        assert_eq!(got.events, events, "parse recovers the exact event set");
    }

    #[test]
    fn container_roundtrips_a_signed_marker() {
        let k = sk();
        let g = enroll(&k, "Self");
        let att = build_self_attestation(&k, &kid(&k), &node_id(&g), std::slice::from_ref(&g));
        let image = serialize_container(Some(&SelfMarker::Signed(att.clone())), &[g])
            .expect("fixture fits the cap");
        let got = parse_container(&image).unwrap();
        assert_eq!(got.self_marker, Some(SelfMarker::Signed(att)));
    }

    #[test]
    fn container_roundtrips_no_marker() {
        let k = sk();
        let events = vec![enroll(&k, "Self")];
        let image = serialize_container(None, &events).expect("fixture fits the cap");
        let got = parse_container(&image).unwrap();
        assert_eq!(got.self_marker, None);
        assert_eq!(got.events, events);
    }

    #[test]
    fn legacy_cairnb1_medium_parses_with_no_marker() {
        // A CAIRNB1 image (magic ++ frames) must still parse, yielding self_marker == None.
        let k = sk();
        let g = enroll(&k, "Self");
        let mut image = MEDIUM_MAGIC_V1.to_vec();
        put_chunk(&mut image, &g).expect("fixture fits the cap");
        let got = parse_container(&image).unwrap();
        assert_eq!(got.self_marker, None, "legacy medium has no marker");
        assert_eq!(got.events, vec![g]);
    }

    #[test]
    fn parse_rejects_missing_magic_and_unknown_kind() {
        // Not a medium at all: the operator picked the wrong file. Nothing is damaged, and
        // saying "damaged" here would send them looking for a second copy that does not exist.
        assert!(
            matches!(
                parse_container(b"not a medium"),
                Err(BackupError::NotAMedium(_))
            ),
            "bytes with no recognised magic are NOT a medium, not a damaged one"
        );
        // CAIRNB2 with an out-of-range marker kind: real damage.
        let mut bad = MEDIUM_MAGIC_V2.to_vec();
        bad.push(99);
        assert!(matches!(
            parse_container(&bad),
            Err(BackupError::Damaged(_))
        ));
    }

    /// A CAIRNB3 medium handed to the LEGACY parser must say what it actually is.
    ///
    /// `parse_container` reads CAIRNB1/CAIRNB2 only, and `cairn-node`'s `verify-backup` still
    /// routes through it — so without this arm, pointing that command at a perfectly good
    /// CAIRNB3 medium reports "this is not a backup medium". That is the wrong-remedy failure
    /// the error taxonomy exists to prevent: an operator told their good medium is not a
    /// medium may well go looking for another copy, mid-disaster.
    #[test]
    fn a_cairnb3_medium_is_named_not_dismissed_by_the_legacy_parser() {
        let sk = sk();
        let seg = signed_segment(&sk, "abcd", Plane::Node, 0, "", 1);
        let bytes = serialize_v3(&[seg]).unwrap();
        let err = parse_container(&bytes).expect_err("the legacy parser cannot read CAIRNB3");
        assert!(
            matches!(err, BackupError::UnsupportedByThisBuild(_)),
            "a CAIRNB3 medium is not damaged and is not a non-medium: {err:?}"
        );
        assert!(
            err.to_string().contains("CAIRNB3"),
            "and the error must name what it actually is: {err}"
        );
    }

    #[test]
    fn a_v3_medium_roundtrips_both_planes() {
        let sk = sk();
        let node = signed_segment(&sk, "abcd", Plane::Node, 0, "", 2);
        let clin = signed_segment(
            &sk,
            "abcd",
            Plane::Clinical,
            1,
            &segment_commitment(&node.records),
            3,
        );
        let bytes = serialize_v3(&[node.clone(), clin.clone()]).unwrap();
        match parse_any(&bytes).unwrap() {
            MediumImage::V3(m) => {
                assert_eq!(m.segments, vec![node, clin]);
                assert!(m.segments.iter().all(|s| s.plane.is_known()));
                assert!(!m.truncated_tail);
            }
            MediumImage::Legacy(_) => panic!("CAIRNB3 magic must not parse as legacy"),
        }
    }

    /// Appending is byte-wise: the existing image is untouched and the new section lands
    /// at the end. This is the property that makes capture O(new records).
    #[test]
    fn appending_leaves_the_existing_bytes_untouched() {
        let sk = sk();
        let first = signed_segment(&sk, "abcd", Plane::Node, 0, "", 1);
        let mut image = serialize_v3(std::slice::from_ref(&first)).unwrap();
        let before = image.clone();
        let second = signed_segment(
            &sk,
            "abcd",
            Plane::Clinical,
            1,
            &segment_commitment(&first.records),
            1,
        );
        append_segment(&mut image, &second).unwrap();
        assert_eq!(
            &image[..before.len()],
            &before[..],
            "an append must not rewrite a byte"
        );
        match parse_any(&image).unwrap() {
            MediumImage::V3(m) => assert_eq!(m.segments.len(), 2),
            other => panic!("{other:?}"),
        }
    }

    /// Shared fixture for the two torn-medium tests below: a two-segment CAIRNB3 image
    /// where `b` is cut short 12 bytes in, plus the offset where the intact prefix ends.
    /// One definition, so "what counts as torn here" cannot drift between the two tests.
    fn torn_two_segment_image(sk: &cairn_event::SigningKey) -> (Vec<u8>, Segment, usize) {
        let a = signed_segment(sk, "abcd", Plane::Node, 0, "", 1);
        let b = signed_segment(
            sk,
            "abcd",
            Plane::Clinical,
            1,
            &segment_commitment(&a.records),
            4,
        );
        let mut image = serialize_v3(std::slice::from_ref(&a)).unwrap();
        let intact = image.len();
        append_segment(&mut image, &b).unwrap();
        image.truncate(intact + 12); // a crash partway through the second section — torn
        (image, a, intact)
    }

    /// A torn append yields every complete segment before it, plus the flag. Nothing
    /// earlier is lost, and the loss that did occur is visible.
    #[test]
    fn a_torn_append_yields_the_complete_prefix_and_says_so() {
        let (image, a, _intact) = torn_two_segment_image(&sk());
        match parse_any(&image).unwrap() {
            MediumImage::V3(m) => {
                assert_eq!(m.segments, vec![a], "the complete prefix survives whole");
                assert!(
                    m.truncated_tail,
                    "and the torn tail is REPORTED, never silent"
                );
            }
            other => panic!("{other:?}"),
        }
    }

    /// I4 (#500 final review): `complete_bytes` is what makes a torn medium safely
    /// resumable — without it, appending after a torn tail's partial bytes makes them the
    /// next section's length prefix, and parsing stops there FOREVER. Performs the recovery
    /// recipe the field's doc prescribes: truncate, THEN append, then confirm a clean parse.
    #[test]
    fn a_writer_can_resume_after_a_torn_tail_by_truncating_to_complete_bytes() {
        let sk = sk();
        let (mut image, a, intact) = torn_two_segment_image(&sk);
        let torn = match parse_any(&image).unwrap() {
            MediumImage::V3(m) => m,
            other => panic!("{other:?}"),
        };
        assert!(torn.truncated_tail, "must report the tear before recovery");
        assert_eq!(torn.complete_bytes, intact);
        image.truncate(torn.complete_bytes); // the recipe: truncate BEFORE appending
        let fresh = signed_segment(
            &sk,
            "abcd",
            Plane::Clinical,
            1,
            &segment_commitment(&a.records),
            2,
        );
        append_segment(&mut image, &fresh).unwrap();
        match parse_any(&image).unwrap() {
            MediumImage::V3(m) => {
                assert_eq!(m.segments, vec![a, fresh], "every segment is present");
                assert!(!m.truncated_tail, "a properly recovered medium is not torn");
            }
            other => panic!("{other:?}"),
        }
    }

    /// An unknown plane is collected and named while parsing continues past it.
    #[test]
    fn an_unknown_plane_is_collected_and_parsing_continues() {
        let sk = sk();
        let a = signed_segment(&sk, "abcd", Plane::Node, 0, "", 1);
        let b = signed_segment(
            &sk,
            "abcd",
            Plane::Clinical,
            1,
            &segment_commitment(&a.records),
            2,
        );
        let mut image = serialize_v3(&[a.clone(), b.clone()]).unwrap();
        // Corrupt the FIRST section's plane tag: 8 magic bytes + 4 length bytes.
        image[MEDIUM_MAGIC_V3.len() + 4] = 77;
        match parse_any(&image).unwrap() {
            MediumImage::V3(m) => {
                assert_eq!(
                    m.segments.len(),
                    2,
                    "an unknown plane stays IN the segment list — dropping it is what broke \
                     the chain for every segment after it"
                );
                assert_eq!(m.segments[0].plane, Plane::Unknown(77), "carried verbatim");
                assert_eq!(
                    m.segments[0].records, a.records,
                    "and with every record intact, so the chain can still traverse it"
                );
                assert_eq!(m.segments[1], b, "the known segment is untouched");
            }
            other => panic!("{other:?}"),
        }
    }

    /// CAIRNB1 and CAIRNB2 still dispatch to the legacy path, byte for byte.
    #[test]
    fn legacy_media_still_parse_as_legacy() {
        let sk = sk();
        let events = vec![enroll(&sk, "a")];
        let v2 = serialize_container(Some(&SelfMarker::Unsigned("abcd".into())), &events)
            .expect("fixture fits the cap");
        match parse_any(&v2).unwrap() {
            MediumImage::Legacy(c) => {
                assert_eq!(c.events, events);
                assert_eq!(c.self_marker, Some(SelfMarker::Unsigned("abcd".into())));
            }
            other => panic!("a CAIRNB2 medium must not become a V3 image: {other:?}"),
        }
    }

    #[test]
    fn a_medium_with_no_recognised_magic_is_refused() {
        assert!(parse_any(b"NOTACAIRN\n").is_err());
    }
}
