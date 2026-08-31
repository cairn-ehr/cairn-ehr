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

/// Upper bound on one section. A section holds ONE capture increment, which slice 2b
/// bounds by a batch limit; 256 MiB is generous for that and still caps a corrupt length
/// prefix. Note the real protection against a bogus prefix is that decoding works over an
/// in-memory slice and refuses a length exceeding what remains — this cap is what
/// separates "damaged medium" from "interrupted backup" (see `take_section`).
const MAX_SECTION_BYTES: usize = 256 * 1024 * 1024;

/// The smallest a single encoded record can possibly be: a `[u32 len]` chunk holding a
/// zero-length `signed_bytes` (4 bytes) + a flags byte (1) + an 8-byte `source_seq` (8) =
/// 13. Used only to bound the pre-allocation hint in `take_section` below — see its doc
/// comment for why (I2, #500 final review).
const MIN_RECORD_BYTES: usize = 13;

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
    /// Which node wrote this segment, as PLAINTEXT.
    ///
    /// UNTRUSTED — read this doc before consuming it. This field is never bound to the
    /// attestation's own SIGNED `self_node_id_hex` (nothing in this crate checks the two
    /// agree), so on a SIGNED segment the plaintext field can disagree with the verified one
    /// and nothing here would notice. Nothing reads this field for identity today, but a
    /// future caller easily could by reaching for the obviously-named field instead of the
    /// verified return value — documented now, before that caller exists (minor, #500 final
    /// review). The ONLY trustworthy identification is the return value of
    /// [`crate::attest::verify_segment_attestation`] (one segment) or
    /// [`crate::chain::self_id_from_chain`] (a whole medium) — never this field directly.
    ///
    /// It exists so an UNSIGNED segment still names itself — present even when unsigned,
    /// which is what closes the operator-typo footgun; empty before enrolment, when there is
    /// no identity to name yet.
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

/// Encode one segment as a length-prefixed section.
///
/// The outer `[u32 len]` is what makes a torn append detectable without parsing the
/// segment: a reader that has fewer than `len` bytes left knows the append was cut short,
/// and stops cleanly at the last complete section.
///
/// I6 (#500 final review): the cap used to be a `debug_assert!` only, so in a RELEASE build
/// a capture exceeding `MAX_SECTION_BYTES` was written successfully and then could NEVER be
/// read back — `take_section` (above) refuses anything over the cap as corruption. Write
/// succeeds, read fails, permanently. `chunk.rs`'s `put_chunk` already refuses an over-cap
/// chunk honestly rather than merely asserting it away in debug; this mirrors that "refuse
/// at the source" discipline at the section level: a caller that would produce an unreadable
/// medium is told so at write time, on the spot, in every build — not months later on
/// restore.
pub(crate) fn put_segment(out: &mut Vec<u8>, seg: &Segment) -> Result<(), BackupError> {
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
    if body.len() > MAX_SECTION_BYTES {
        return Err(BackupError::Encode(format!(
            "refusing to write a {}-byte medium section: it exceeds the \
             {MAX_SECTION_BYTES}-byte cap and could never be read back (`take_section` \
             rejects anything over the cap as corruption) — split the capture into smaller \
             batches instead",
            body.len()
        )));
    }
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(&body);
    Ok(())
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

    // I2 (#500 final review): `record_count` is a raw, unauthenticated `u32` read straight
    // off the medium before a single record is parsed. Reserving `record_count as usize`
    // capacity EAGERLY (as this line once did) lets one flipped bit turn `record_count` into
    // ~4 billion, requesting ~447 GB for a `Vec<MediumRecord>` — and Rust's allocator
    // ABORTS THE WHOLE PROCESS on an allocation failure; it is not a catchable `Result`. A
    // single bit flip on a healthy medium would kill `verify-backup`/`restore` outright,
    // mid-disaster. `chunk.rs`'s `MAX_CHUNK_BYTES` already states this crate's discipline
    // one file over ("a bit-flip can never force a multi-GiB allocation during parse") — it
    // just was not carried here. Bound the CAPACITY HINT by the bytes actually remaining:
    // `record_count` records can never fit in fewer than `record_count * MIN_RECORD_BYTES`
    // bytes, so reserving more than `b.len() / MIN_RECORD_BYTES` is always wasted. The real
    // count is still enforced honestly by the loop below, which returns a clean
    // `BackupError` the moment it runs out of bytes — this line only ever changes how much
    // we pre-reserve, never what we accept.
    let capacity_hint = (record_count as usize).min(b.len() / MIN_RECORD_BYTES);
    let mut records = Vec::with_capacity(capacity_hint);
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
            put_segment(&mut out, &seg).unwrap();
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
        put_segment(&mut out, &seg).unwrap();
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
        put_segment(&mut out, &seg).unwrap();
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
        put_segment(&mut out, &seg).unwrap();
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
        put_segment(&mut out, &segment(Plane::Node, 0, 1)).unwrap();
        out[..4].copy_from_slice(&u32::MAX.to_be_bytes());
        let err = take_section(&out).expect_err("must be an error, not Ok(None)");
        assert!(err.to_string().contains("cap"), "must name the cap: {err}");
    }

    /// I2 (#500 final review): `record_count` is corrupted to an absurd value (`u32::MAX`)
    /// while the body behind it is short. Before this fix, `take_section` would try to
    /// EAGERLY reserve `record_count` (~4 billion) `MediumRecord`s of capacity — hundreds of
    /// gigabytes — and Rust's allocator ABORTS THE WHOLE PROCESS on a failure that large,
    /// not a catchable error. This test does not reproduce that abort (deliberately: doing
    /// so means attempting the very allocation this fix exists to prevent, which is not a
    /// safe thing to provoke in a test run). It pins the FIXED behaviour instead: the
    /// capacity hint is bounded by the bytes actually remaining, so parsing proceeds
    /// straight to the record loop, which runs out of bytes on the very first record and
    /// returns a clean `BackupError` — never a crash.
    #[test]
    fn an_absurd_record_count_over_a_short_body_errors_rather_than_aborting() {
        // Hand-build a minimal section body: plane tag + index + three EMPTY chunks
        // (prev_commitment / self_node_id_hex / attestation) + an absurd record_count, with
        // NO record bytes behind it at all.
        let mut body = Vec::new();
        body.push(Plane::Clinical.tag());
        body.extend_from_slice(&0u32.to_be_bytes()); // index
        put_chunk(&mut body, b""); // prev_commitment
        put_chunk(&mut body, b"abcd"); // self_node_id_hex
        put_chunk(&mut body, b""); // attestation: none
        body.extend_from_slice(&u32::MAX.to_be_bytes()); // record_count: absurd

        let mut out = Vec::new();
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        out.extend_from_slice(&body);

        let err = take_section(&out)
            .expect_err("an absurd record_count over a short body must return an Err, never abort");
        assert!(
            err.to_string().contains("truncated") || err.to_string().contains("byte"),
            "must be the honest parse failure, not something else: {err}"
        );
    }

    /// I6 (#500 final review): before this fix, `put_segment`'s cap check was a
    /// `debug_assert!` only — in a RELEASE build, a segment over `MAX_SECTION_BYTES` was
    /// written successfully and then could NEVER be read back (`take_section` rejects it as
    /// corruption). Write succeeds, read fails, permanently. `put_segment` must now refuse
    /// at the source, in every build, naming the cap in the error.
    ///
    /// A single RECORD cannot carry enough bytes to trip the SECTION cap on its own — each
    /// record's `signed_bytes` is itself capped at `chunk::MAX_CHUNK_BYTES` (8 MiB) by
    /// `put_chunk`'s own cap check, which would fire first and prove nothing about this
    /// one. So this pushes the SECTION over its cap with many chunk-cap-sized records
    /// instead, exactly the way a real oversized capture would.
    #[test]
    fn put_segment_refuses_an_over_cap_section_at_the_source() {
        // Overhead per record with no optional fields: a 4-byte chunk length prefix on
        // signed_bytes, 1 flags byte, and an 8-byte source_seq.
        let per_record_len = crate::chunk::MAX_CHUNK_BYTES + 4 + 1 + 8;
        // Deliberately more than enough to cross MAX_SECTION_BYTES, not just barely over.
        let n = MAX_SECTION_BYTES / per_record_len + 2;
        let records: Vec<MediumRecord> = (0..n as i64)
            .map(|i| MediumRecord {
                // The exact content does not matter, only its size — a cheap repeated-byte
                // fill is fine (not a crypto value; house rule 6 does not apply to filler).
                signed_bytes: vec![7u8; crate::chunk::MAX_CHUNK_BYTES],
                attestation: None,
                attester_key: None,
                dek_wrapped: None,
                source_seq: i,
            })
            .collect();
        let seg = Segment {
            plane: Plane::Clinical,
            index: 0,
            prev_commitment: String::new(),
            self_node_id_hex: "abcd".into(),
            attestation: None,
            records,
        };
        let mut out = Vec::new();
        let err = put_segment(&mut out, &seg).expect_err("must refuse an over-cap section");
        assert!(
            err.to_string().contains("cap"),
            "the refusal must NAME the cap: {err}"
        );
        assert!(
            out.is_empty(),
            "a refused section must write nothing at all, not a partial write"
        );
    }
}
