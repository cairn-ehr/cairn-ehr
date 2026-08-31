//! CAIRNB3 — the append-only, plane-tagged segment: the chained, signed GROUP of records
//! that makes an append-only medium safe to write in place.
//!
//! WHY THIS EXISTS, and why it is not [`crate::marker`]: a CAIRNB2 medium carries ONE
//! head marker whose signature commits to `event_set_commitment(events)` — the whole
//! sorted event set. That is unappendable by construction. Adding a single event changes
//! the commitment, so every append would need the head re-signed, and rewriting the head
//! shifts every byte after it: a whole-file rewrite on every backup, over a log that
//! grows for the life of a clinic.
//!
//! So CAIRNB3 has no head block. Each SEGMENT — one append increment — carries its own
//! signed attestation naming this node and committing to its own records plus its
//! predecessor's commitment. Appending costs O(new records), the chain is verifiable
//! end-to-end, and a segment lifted from another medium fails on its predecessor.
//!
//! A segment's shape tracks the APPEND/CHAIN design; it changes when durability design
//! changes. The records it carries are a separate concern with a separate reason to
//! change — see [`crate::record`], which tracks the sync WIRE shape instead.

use crate::chunk::{put_chunk, take_chunk};
use crate::error::BackupError;
use crate::record::{put_record, take_record, MediumRecord};

// NOTE ON THE `expect(dead_code)` ATTRIBUTES BELOW: this module lands one task ahead of
// its own caller. `put_segment`/`take_section` are the entry points a production caller
// will use, but nothing in this crate calls them yet — the CAIRNB3 container task wires
// them into `container.rs`. `expect` rather than `allow` is deliberate: it is
// self-cleaning — once that caller lands, the lint stops firing, the `expect` itself
// becomes an "unfulfilled expectation" error, and the build forces its removal instead of
// it going stale silently. The attribute goes ONLY on the two unreferenced entry points —
// dead-code liveness propagates from an allowed/expected root to whatever it calls, so
// `MAX_SECTION_BYTES`, `Plane`, `Segment`, `UnknownSegment` and `TakenSection` are already
// covered through them and must NOT carry their own `expect`, or that second attribute
// finds nothing left to suppress and itself becomes an "unfulfilled expectation" error.
// `put_record`/`take_record` (in `crate::record`) carry no such attribute of their own:
// `put_segment` calls `put_record` and `take_section` calls `take_record`, which makes
// them live from this file — their sole production caller today.

/// Upper bound on one section. A section holds ONE capture increment, which slice 2b
/// bounds by a batch limit; 256 MiB is generous for that and still caps a corrupt length
/// prefix. Note the real protection against a bogus prefix is that decoding works over an
/// in-memory slice and refuses a length exceeding what remains — this cap is what
/// separates "damaged medium" from "interrupted backup" (see `take_section`).
const MAX_SECTION_BYTES: usize = 256 * 1024 * 1024;

/// Which plane a segment's records belong to. The two planes share ONE record shape and
/// ONE codec; the tag is how a reader knows which door the records are destined for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plane {
    /// `node_event` — enrolments, pairings, supersedes. Records carry no custody.
    Node,
    /// `event_log` — the clinical, demographic, identity, registration and erasure streams.
    Clinical,
}

impl Plane {
    pub fn tag(self) -> u8 {
        match self {
            Plane::Node => 1,
            Plane::Clinical => 2,
        }
    }

    /// `None` for a tag this build does not know — the caller reports it as an
    /// [`UnknownSegment`] rather than skipping it.
    pub fn from_tag(t: u8) -> Option<Plane> {
        match t {
            1 => Some(Plane::Node),
            2 => Some(Plane::Clinical),
            _ => None,
        }
    }

    /// The stable string used inside a segment attestation's signed payload.
    pub fn label(self) -> &'static str {
        match self {
            Plane::Node => "node",
            Plane::Clinical => "clinical",
        }
    }
}

/// One append increment: a run of records, tagged with its plane, positioned in the
/// chain, and (normally) signed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub plane: Plane,
    /// Position in the medium's single chain, in file order, from 0. NOT per-plane: one
    /// chain over the whole medium detects a reordering or a splice ACROSS planes, which
    /// two independent chains could not.
    pub index: u32,
    /// The preceding segment's commitment; empty for index 0.
    pub prev_commitment: String,
    /// Which node wrote this segment. Empty before enrolment (a node with no identity to
    /// name). Present even when unsigned — that is what closes the operator-typo footgun.
    pub self_node_id_hex: String,
    /// The signed `node.segment_attested` bytes, or `None` when the signing key was not
    /// available at capture. An unavailable key never BLOCKS a backup; it travels flagged.
    ///
    /// NOTE the asymmetry with `crate::record::MediumRecord::attestation`: on decode, an
    /// EMPTY attestation chunk collapses into this same `None` (see `take_section` below),
    /// deliberately — an empty segment attestation is not a meaningful state to preserve
    /// on its own, `attest::verify_segment_attestation` treats it exactly like "absent",
    /// and no door distinguishes the two. `MediumRecord` cannot make the same
    /// simplification: there, `None` (no token travelled) vs. `Some(vec![])` (an empty
    /// token travelled) is load-bearing at the clinical apply door, which reacts to each
    /// differently. Two neighbouring layers, two different rules — deliberate, not drift.
    pub attestation: Option<Vec<u8>>,
    pub records: Vec<MediumRecord>,
}

/// A segment whose plane tag this build does not recognise. Reported, never skipped: its
/// header layout is fixed regardless of plane, so we can still say how much we could not
/// read — and NAMING what was not understood is the difference between honest degradation
/// and a medium that parses cleanly while missing a plane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownSegment {
    pub plane_tag: u8,
    pub index: u32,
    pub record_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TakenSection {
    Known(Segment),
    Unknown(UnknownSegment),
}

/// Encode one segment as a length-prefixed section. Pure.
///
/// The outer `[u32 len]` is what makes a torn append detectable without parsing the
/// segment: a reader that has fewer than `len` bytes left knows the append was cut short,
/// and stops cleanly at the last complete section.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "wired up by the CAIRNB3 container task")
)]
pub(crate) fn put_segment(out: &mut Vec<u8>, seg: &Segment) {
    let mut body = Vec::new();
    body.push(seg.plane.tag());
    body.extend_from_slice(&seg.index.to_be_bytes());
    put_chunk(&mut body, seg.prev_commitment.as_bytes());
    put_chunk(&mut body, seg.self_node_id_hex.as_bytes());
    put_chunk(&mut body, seg.attestation.as_deref().unwrap_or(&[]));
    body.extend_from_slice(&(seg.records.len() as u32).to_be_bytes());
    for r in &seg.records {
        put_record(&mut body, r);
    }
    debug_assert!(
        body.len() <= MAX_SECTION_BYTES,
        "section exceeds the medium cap"
    );
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(&body);
}

/// Read one section.
///
/// Three outcomes, deliberately distinct:
///   - `Ok(Some(..))` — a complete section, known or unknown plane;
///   - `Ok(None)` — a TORN TAIL: fewer bytes remain than the section claims, which is what
///     an interrupted append looks like. The caller keeps everything before it and flags
///     the tail. Remedy: run the backup again.
///   - `Err(..)` — CORRUPTION: a length prefix beyond the cap, or a malformed body. The
///     remedy is different ("this medium is damaged"), so the verdicts never collapse.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "wired up by the CAIRNB3 container task")
)]
pub(crate) fn take_section(rest: &[u8]) -> Result<Option<(TakenSection, &[u8])>, BackupError> {
    if rest.len() < 4 {
        return Ok(None); // not even a complete length prefix — torn
    }
    let len = u32::from_be_bytes([rest[0], rest[1], rest[2], rest[3]]) as usize;
    if len > MAX_SECTION_BYTES {
        return Err(BackupError::Decode(format!(
            "medium section length {len} exceeds the {MAX_SECTION_BYTES}-byte cap — the \
             medium is damaged (an INTERRUPTED backup reads as a short tail, not as this)"
        )));
    }
    if rest.len() < 4 + len {
        return Ok(None); // the section is complete on no copy of this file — torn
    }
    let (body, tail) = (&rest[4..4 + len], &rest[4 + len..]);
    let (&plane_tag, b) = body
        .split_first()
        .ok_or_else(|| BackupError::Decode("empty medium section: no plane tag".into()))?;
    if b.len() < 4 {
        return Err(BackupError::Decode(
            "medium section truncated: no segment index after the plane tag".into(),
        ));
    }
    let (idx, b) = b.split_at(4);
    let index = u32::from_be_bytes(idx.try_into().expect("4 bytes"));
    let (prev, b) = take_chunk(b)?;
    let (self_id, b) = take_chunk(b)?;
    let (att, b) = take_chunk(b)?;
    if b.len() < 4 {
        return Err(BackupError::Decode(
            "medium section truncated: no record count".into(),
        ));
    }
    let (count_bytes, mut b) = b.split_at(4);
    let record_count = u32::from_be_bytes(count_bytes.try_into().expect("4 bytes"));

    let Some(plane) = Plane::from_tag(plane_tag) else {
        // NAMED, never skipped. We consumed the section by its length, so parsing
        // continues past it — but the caller is told exactly what it did not understand.
        return Ok(Some((
            TakenSection::Unknown(UnknownSegment {
                plane_tag,
                index,
                record_count,
            }),
            tail,
        )));
    };

    let mut records = Vec::with_capacity(record_count as usize);
    for _ in 0..record_count {
        let (r, next) = take_record(b)?;
        records.push(r);
        b = next;
    }
    if !b.is_empty() {
        return Err(BackupError::Decode(format!(
            "medium section has {} trailing byte(s) after its {record_count} record(s)",
            b.len()
        )));
    }
    let to_string = |v: &[u8], what: &str| -> Result<String, BackupError> {
        std::str::from_utf8(v)
            .map(str::to_string)
            .map_err(|_| BackupError::Decode(format!("segment {what} is not UTF-8")))
    };
    Ok(Some((
        TakenSection::Known(Segment {
            plane,
            index,
            prev_commitment: to_string(prev, "prev_commitment")?,
            self_node_id_hex: to_string(self_id, "self_node_id_hex")?,
            attestation: (!att.is_empty()).then(|| att.to_vec()),
            records,
        }),
        tail,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::segment;

    #[test]
    fn a_segment_roundtrips_through_its_section_framing() {
        for plane in [Plane::Node, Plane::Clinical] {
            let seg = segment(plane, 3, 4);
            let mut out = Vec::new();
            put_segment(&mut out, &seg);
            let (taken, rest) = take_section(&out).expect("no error").expect("not torn");
            assert!(rest.is_empty());
            match taken {
                TakenSection::Known(back) => assert_eq!(back, seg),
                TakenSection::Unknown(u) => panic!("known plane decoded as unknown: {u:?}"),
            }
        }
    }

    /// An unsigned segment still names itself. The signing key may be unavailable at
    /// capture, and an unavailable key must never BLOCK a backup — it travels flagged.
    /// `self_node_id_hex` is what closes the operator-typo footgun, exactly as
    /// `SelfMarker::Unsigned` does on a CAIRNB2 medium.
    #[test]
    fn an_unsigned_segment_still_carries_its_self_id() {
        let seg = Segment {
            attestation: None,
            ..segment(Plane::Clinical, 0, 2)
        };
        let mut out = Vec::new();
        put_segment(&mut out, &seg);
        let (taken, _) = take_section(&out).unwrap().unwrap();
        match taken {
            TakenSection::Known(back) => {
                assert_eq!(back.attestation, None, "unsigned stays unsigned");
                assert_eq!(back.self_node_id_hex, "abcd", "and still names itself");
            }
            other => panic!("{other:?}"),
        }
    }

    /// An unrecognised plane tag is NAMED, never skipped. The header layout is fixed
    /// regardless of plane, so index and record count are still readable — and reporting
    /// them is what lets a caller say "12 clinical records I could not read" rather than
    /// silently restoring a medium that is missing a plane.
    #[test]
    fn an_unknown_plane_tag_is_named_not_skipped() {
        let seg = segment(Plane::Clinical, 5, 12);
        let mut out = Vec::new();
        put_segment(&mut out, &seg);
        out[4] = 99; // the plane tag is the first byte of the segment, after the u32 length
        let (taken, rest) = take_section(&out).unwrap().unwrap();
        assert!(
            rest.is_empty(),
            "an unknown section is consumed whole, by its length"
        );
        match taken {
            TakenSection::Unknown(u) => {
                assert_eq!((u.plane_tag, u.index, u.record_count), (99, 5, 12));
            }
            other => panic!("expected Unknown, got {other:?}"),
        }
    }

    /// A torn append yields `Ok(None)` — "nothing complete here" — not an error. This is
    /// the property that makes an append-only medium safe to write in place: a crash mid
    /// append costs the last increment and nothing else.
    #[test]
    fn a_torn_tail_reports_incomplete_rather_than_corrupt() {
        let seg = segment(Plane::Clinical, 1, 3);
        let mut out = Vec::new();
        put_segment(&mut out, &seg);
        for cut in [0usize, 1, 3, 4, 10, out.len() - 1] {
            assert!(
                take_section(&out[..cut])
                    .expect("a short tail is not an error")
                    .is_none(),
                "a tail cut at {cut} must read as incomplete, never as corrupt"
            );
        }
    }

    /// A length prefix larger than the cap is CORRUPTION, not a torn tail. The two send an
    /// operator to different places — "your last backup was interrupted, run it again"
    /// versus "this medium is damaged" — so they must never collapse into one verdict.
    #[test]
    fn an_absurd_section_length_is_corruption_not_a_torn_tail() {
        let mut out = Vec::new();
        put_segment(&mut out, &segment(Plane::Node, 0, 1));
        out[..4].copy_from_slice(&u32::MAX.to_be_bytes());
        let err = take_section(&out).expect_err("must be an error, not Ok(None)");
        assert!(err.to_string().contains("cap"), "must name the cap: {err}");
    }
}
