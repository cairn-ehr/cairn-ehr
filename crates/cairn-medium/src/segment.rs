//! CAIRNB3 — the append-only, plane-tagged segment and the records it carries.
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
//! A RECORD is one event as the medium carries it. Its fields are deliberately the same
//! five `cairn-sync`'s `EventsResponse` carries on the wire (`events`, `attestations`,
//! `attester_keys`, `wrapped_deks`, `seqs`), because slice 2b addresses the medium
//! through that same protocol. A medium carrying less would be a lookalike, not a peer —
//! and a restore through `apply_remote_event` would silently lose the attestation a
//! suppressing event needs in order to be admitted at all.

use crate::chunk::{put_chunk, take_chunk};
use crate::error::BackupError;

// NOTE ON THE `expect(dead_code)` ATTRIBUTES BELOW (task 5 of the slice-2a plan): this
// module still lands one task ahead of ITS caller. `put_segment`/`take_section` are the
// entry points a production caller will use, and nothing in this crate calls them yet —
// the CAIRNB3 container task wires them into `container.rs`. Task 4's identical
// attributes on `put_record`/`take_record` are gone: this task's own `put_segment` calls
// `put_record` and `take_section` calls `take_record`, which makes them live, and `expect`
// (unlike `allow`) fires as an error the moment the lint it names stops applying — so
// leaving those two in place would itself be a build error now. `expect` rather than
// `allow` is deliberate here too: it is self-cleaning — once the container task lands,
// the lint stops firing, the `expect` becomes an "unfulfilled expectation" error, and the
// build forces its removal instead of it going stale silently. The attribute goes ONLY on
// the two unreferenced entry points (`put_segment`, `take_section`) — dead-code liveness
// propagates from an allowed/expected root to whatever it calls, so `MAX_SECTION_BYTES`,
// `Plane`, `Segment`, `UnknownSegment` and `TakenSection` are already covered through them
// and must NOT carry their own `expect`, or that second attribute finds nothing left to
// suppress and itself becomes an "unfulfilled expectation" error.

/// Flags byte: which optional fields follow the `signed_bytes` chunk.
const FLAG_ATTESTATION: u8 = 0b001;
const FLAG_ATTESTER_KEY: u8 = 0b010;
const FLAG_DEK: u8 = 0b100;
/// Every bit this build understands. A record setting anything outside this mask was
/// written by a newer Cairn and is REFUSED — see `take_record`.
const KNOWN_FLAGS: u8 = FLAG_ATTESTATION | FLAG_ATTESTER_KEY | FLAG_DEK;

/// One event on the medium, in the shape the sync wire carries it.
///
/// `source_seq` is the CAPTURING node's local insertion order for this event — the
/// medium's cursor. It is stored per record rather than per segment so the medium can
/// answer a cursored request the way a serving node does, and so an interrupted restore
/// can resume from where it stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediumRecord {
    /// The event itself: COSE_Sign1 bytes, verbatim, never re-serialized.
    pub signed_bytes: Vec<u8>,
    /// The human attestation token, when one travelled. `None` (no token) and
    /// `Some(vec![])` (an empty token) are DIFFERENT facts and must stay distinguishable:
    /// the apply door refuses a suppressing event with no token fail-closed, and reports
    /// an invalid token differently.
    pub attestation: Option<Vec<u8>>,
    /// The attester's public key, when one travelled.
    pub attester_key: Option<Vec<u8>>,
    /// This event's DEK, wrapped to the capturing node's unwrap public key. `None`
    /// whenever no custody travels: the event is unsealed, this node holds no DEK for it,
    /// or it has been shredded here.
    pub dek_wrapped: Option<Vec<u8>>,
    /// The capturing node's local `seq` for this event.
    pub source_seq: i64,
}

/// Encode one record. Pure.
pub(crate) fn put_record(out: &mut Vec<u8>, r: &MediumRecord) {
    put_chunk(out, &r.signed_bytes);
    let mut flags = 0u8;
    if r.attestation.is_some() {
        flags |= FLAG_ATTESTATION;
    }
    if r.attester_key.is_some() {
        flags |= FLAG_ATTESTER_KEY;
    }
    if r.dek_wrapped.is_some() {
        flags |= FLAG_DEK;
    }
    out.push(flags);
    for v in [&r.attestation, &r.attester_key, &r.dek_wrapped]
        .into_iter()
        .flatten()
    {
        put_chunk(out, v);
    }
    out.extend_from_slice(&r.source_seq.to_be_bytes());
}

/// Read one optional field iff its flag bit is set, advancing `rest`. Pure and reusable —
/// the three optional fields differ only in which bit gates them, and three inline copies
/// is three places for one of them to be forgotten.
fn take_optional(flags: u8, bit: u8, rest: &mut &[u8]) -> Result<Option<Vec<u8>>, BackupError> {
    if flags & bit == 0 {
        return Ok(None);
    }
    let (v, r) = take_chunk(rest)?;
    *rest = r;
    Ok(Some(v.to_vec()))
}

/// Decode one record, returning it and the remainder. Errors (never panics) on a
/// truncated record or an unrecognised flag bit.
pub(crate) fn take_record(rest: &[u8]) -> Result<(MediumRecord, &[u8]), BackupError> {
    let (signed_bytes, rest) = take_chunk(rest)?;
    let (&flags, mut rest) = rest.split_first().ok_or_else(|| {
        BackupError::Decode("truncated record: no flags byte after signed_bytes".into())
    })?;
    // Fail closed on an unknown bit. A newer writer setting bit 3 has put a field here we
    // cannot locate, so everything after it — including this record's source_seq and every
    // following record — would decode as garbage. Refusing names what we did not
    // understand; decoding the prefix would silently drop it.
    if flags & !KNOWN_FLAGS != 0 {
        return Err(BackupError::Decode(format!(
            "record sets unknown flag bit(s) {:04b} (this build understands {KNOWN_FLAGS:03b}); \
             the medium was written by a newer Cairn — upgrade this node before reading it",
            flags & !KNOWN_FLAGS
        )));
    }
    let attestation = take_optional(flags, FLAG_ATTESTATION, &mut rest)?;
    let attester_key = take_optional(flags, FLAG_ATTESTER_KEY, &mut rest)?;
    let dek_wrapped = take_optional(flags, FLAG_DEK, &mut rest)?;
    if rest.len() < 8 {
        return Err(BackupError::Decode(format!(
            "truncated record: {} byte(s) where an 8-byte source_seq was expected",
            rest.len()
        )));
    }
    let (seq, rest) = rest.split_at(8);
    let source_seq = i64::from_be_bytes(seq.try_into().expect("8 bytes"));
    Ok((
        MediumRecord {
            signed_bytes: signed_bytes.to_vec(),
            attestation,
            attester_key,
            dek_wrapped,
            source_seq,
        },
        rest,
    ))
}

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

    /// Runtime-derived bytes for a fixture field. NEVER a literal: a byte-array literal in
    /// a crypto context trips CodeQL's `rust/hard-coded-cryptographic-value` (house rule 6,
    /// issue #146), and a wrapped DEK is exactly such a context.
    fn bytes(seed: u8, len: usize) -> Vec<u8> {
        (0..len).map(|i| seed.wrapping_add(i as u8)).collect()
    }

    fn record(flags: u8) -> MediumRecord {
        MediumRecord {
            signed_bytes: bytes(1, 40),
            attestation: (flags & 0b001 != 0).then(|| bytes(2, 16)),
            attester_key: (flags & 0b010 != 0).then(|| bytes(3, 32)),
            dek_wrapped: (flags & 0b100 != 0).then(|| bytes(4, 48)),
            source_seq: 7,
        }
    }

    /// Every combination of the three optional fields survives a round trip. All eight,
    /// not a sample: the flags byte is a bitfield, and a codec that drops one bit is
    /// exactly the defect that would lose custody on one class of event and no other.
    #[test]
    fn every_flag_combination_roundtrips() {
        for flags in 0..8u8 {
            let r = record(flags);
            let mut out = Vec::new();
            put_record(&mut out, &r);
            let (back, rest) = take_record(&out).expect("decodes");
            assert_eq!(back, r, "flags {flags:03b} did not round-trip");
            assert!(
                rest.is_empty(),
                "flags {flags:03b} left {} trailing bytes",
                rest.len()
            );
        }
    }

    /// A node-plane record is a clinical record with every optional field absent. One
    /// shape serves both planes; a second shape would be a second place for a check to
    /// go stale (the #173 twin-dispatch lesson).
    #[test]
    fn a_node_plane_record_carries_no_custody() {
        let r = record(0b000);
        assert_eq!(
            (
                r.attestation.as_ref(),
                r.attester_key.as_ref(),
                r.dek_wrapped.as_ref()
            ),
            (None, None, None)
        );
        let mut out = Vec::new();
        put_record(&mut out, &r);
        assert_eq!(take_record(&out).unwrap().0, r);
    }

    /// An ABSENT optional field decodes as `None`, never as `Some(vec![])`.
    ///
    /// This is the assertion most likely to pass vacuously, and it is load-bearing: at the
    /// apply door "no attestation travelled" is refused fail-closed for a suppressing
    /// event, while "an empty attestation travelled" is a token that fails validation for a
    /// different reason and reports differently. Conflating them turns a fail-closed gate
    /// into a confusing one.
    #[test]
    fn an_absent_field_is_none_and_an_empty_one_is_some_empty() {
        let mut absent = record(0b000);
        absent.source_seq = -1; // the codec does not police the range; any i64 must round-trip
        let mut out = Vec::new();
        put_record(&mut out, &absent);
        assert_eq!(take_record(&out).unwrap().0.attestation, None);

        let present_but_empty = MediumRecord {
            attestation: Some(Vec::new()),
            ..record(0b000)
        };
        let mut out2 = Vec::new();
        put_record(&mut out2, &present_but_empty);
        assert_eq!(
            take_record(&out2).unwrap().0.attestation,
            Some(Vec::new()),
            "an empty attestation is a DIFFERENT fact from an absent one"
        );
    }

    /// An unrecognised flag bit means a newer writer put fields here we cannot parse.
    /// Refuse loudly rather than decode the prefix we understand and silently drop the
    /// rest — a silently-truncated record is #500's failure shape at record scale.
    #[test]
    fn an_unknown_flag_bit_is_refused_by_name() {
        let mut out = Vec::new();
        put_record(&mut out, &record(0b000));
        // The flags byte sits immediately after the signed_bytes chunk.
        let flags_at = 4 + 40;
        out[flags_at] = 0b1000;
        let err = take_record(&out).expect_err("must refuse");
        let msg = err.to_string();
        assert!(
            msg.contains("flag"),
            "the error must name the flags byte: {msg}"
        );
        assert!(
            msg.contains("1000") || msg.contains('8'),
            "and the unknown bits: {msg}"
        );
    }

    /// A truncated record is reported, never panics.
    #[test]
    fn a_truncated_record_is_an_error_not_a_panic() {
        let mut out = Vec::new();
        put_record(&mut out, &record(0b111));
        for cut in [1usize, 5, 20, out.len() - 1] {
            assert!(take_record(&out[..cut]).is_err(), "cut at {cut} must error");
        }
    }

    fn segment(plane: Plane, index: u32, n: usize) -> Segment {
        Segment {
            plane,
            index,
            prev_commitment: if index == 0 {
                String::new()
            } else {
                "beef".into()
            },
            self_node_id_hex: "abcd".into(),
            attestation: Some(bytes(9, 64)),
            records: (0..n).map(|_| record(0b111)).collect(),
        }
    }

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
