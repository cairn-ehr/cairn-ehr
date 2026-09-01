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

/// Which plane a segment's records belong to. The two known planes share ONE record shape
/// and ONE codec; the tag is how a reader knows which door the records are destined for.
///
/// `Unknown` is NOT an error case — it is how this build represents a plane added by a NEWER
/// Cairn. It carries the raw tag so the segment can still be chained, its records still
/// signature-checked, and the gap named honestly to an operator (#500 slice 2a review).
/// Before `Unknown` existed, an unrecognised plane was dropped out of the segment list
/// entirely, which broke the single global chain for every segment AFTER it (a spurious
/// `ChainBroken` — "this medium is damaged" — about a perfectly healthy medium) and, when it
/// was the LAST segment, let the whole medium report sound while an entire plane was missing.
/// That is invariant 6's own stated failure shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plane {
    /// `node_event` — enrolments, pairings, supersedes. Records carry no custody.
    Node,
    /// `event_log` — the clinical, demographic, identity, registration and erasure streams.
    Clinical,
    /// A plane tag this build does not know, carried verbatim. Written by a newer Cairn.
    Unknown(u8),
}

impl Plane {
    /// The on-disk tag. A WIRE CONSTANT — see `wire_pins.rs`, which pins these literals,
    /// because a mirrored swap of `tag`/`from_tag` is invisible to every round-trip test yet
    /// routes every clinical event to the `node_event` door and vice versa.
    pub fn tag(self) -> u8 {
        match self {
            Plane::Node => 1,
            Plane::Clinical => 2,
            Plane::Unknown(t) => t,
        }
    }

    /// Total, by construction: an unrecognised tag becomes [`Plane::Unknown`] rather than
    /// `None`, so a caller cannot accidentally skip it. Naming what we did not understand is
    /// the difference between honest degradation and a medium that parses cleanly while
    /// missing a plane.
    pub fn from_tag(t: u8) -> Plane {
        match t {
            1 => Plane::Node,
            2 => Plane::Clinical,
            other => Plane::Unknown(other),
        }
    }

    /// The stable string written inside a segment attestation's signed payload — a wire
    /// constant, like [`Plane::tag`]. `None` for an unknown plane: this build cannot know
    /// what a newer Cairn calls its own plane. Verification therefore binds the numeric
    /// [`Plane::tag`], which IS knowable for every plane, and treats the label as an extra
    /// human-legible conjunct checked only when we know it.
    pub fn label(self) -> Option<&'static str> {
        match self {
            Plane::Node => Some("node"),
            Plane::Clinical => Some("clinical"),
            Plane::Unknown(_) => None,
        }
    }

    /// True for a plane this build can actually route records to.
    pub fn is_known(self) -> bool {
        !matches!(self, Plane::Unknown(_))
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
    ///
    /// SELF-DECLARED. `chain::chain_report` checks it against the segment's actual file
    /// position and raises `SegmentFault::IndexMismatch` when they disagree — without that
    /// check the field is attacker-controlled on an unsigned segment, and every fault
    /// "located" by it could point an operator at a segment that does not exist.
    pub index: u32,
    /// The preceding segment's commitment; empty for index 0.
    pub prev_commitment: String,
    /// Which node wrote this segment, as PLAINTEXT.
    ///
    /// UNTRUSTED — read this doc before consuming it. This field is never bound to the
    /// attestation's own SIGNED `self_node_id_hex` (nothing in this crate checks the two
    /// agree), so on a SIGNED segment the plaintext field can disagree with the verified one
    /// and nothing here would notice. The ONLY trustworthy identification is the return
    /// value of [`crate::attest::verify_segment_attestation`] (one segment) or
    /// [`crate::chain::self_id_from_chain`] (a whole medium) — never this field directly.
    /// `chain`'s tests pin that distinction with a fixture whose plaintext field deliberately
    /// disagrees with the attested id, so a future refactor cannot quietly start returning
    /// this one.
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
    /// At least one. An EMPTY segment is refused at write and reported as a fault on read —
    /// see `put_segment`.
    pub records: Vec<MediumRecord>,
}

/// Encode one segment as a length-prefixed section.
///
/// The outer `[u32 len]` is what makes a torn append detectable without parsing the
/// segment: a reader that has fewer than `len` bytes left knows the append was cut short,
/// and stops cleanly at the last complete section.
///
/// REFUSES AT THE SOURCE, in every build, on two conditions:
///
/// 1. **Over the section cap.** This was once a `debug_assert!`, so in a RELEASE build a
///    capture exceeding `MAX_SECTION_BYTES` was written successfully and could then NEVER be
///    read back — `take_section` refuses anything over the cap as corruption. Write succeeds,
///    read fails, permanently. `chunk::put_chunk` now applies the identical discipline one
///    layer down (it did not when this comment first claimed it did — #500 slice 2a review).
///
/// 2. **Empty.** A segment with no records is meaningless as an append increment — a capture
///    with nothing new to write should write NO segment — and it is actively dangerous:
///    `attest::segment_commitment(&[])` is the multihash of the empty string, the SAME
///    constant on every medium ever written. So a segment whose predecessor was empty carries
///    a `prev_commitment` identical across all media, and invariant 3's promise ("a segment
///    spliced from another medium fails on its predecessor") silently evaporates for it: a
///    genuine segment lifts cleanly from medium X onto medium Y whenever both have an empty
///    segment at the same index. Refusing to write one is what keeps that splice defence
///    total (#500 slice 2a review).
pub(crate) fn put_segment(out: &mut Vec<u8>, seg: &Segment) -> Result<(), BackupError> {
    if seg.records.is_empty() {
        return Err(BackupError::Encode(format!(
            "refusing to write an EMPTY {:?} segment at index {}: an empty segment's \
             commitment is the same constant on every medium, which would let a later \
             segment be spliced in from a different medium undetected — a capture with \
             nothing new to write must write no segment at all",
            seg.plane, seg.index
        )));
    }
    let mut body = Vec::new();
    body.push(seg.plane.tag());
    body.extend_from_slice(&seg.index.to_be_bytes());
    put_chunk(&mut body, seg.prev_commitment.as_bytes())?;
    put_chunk(&mut body, seg.self_node_id_hex.as_bytes())?;
    put_chunk(&mut body, seg.attestation.as_deref().unwrap_or(&[]))?;
    // Bounded by the section cap below (2^32 records at >=13 bytes each is ~55 GB, far past
    // it), but stated as a checked conversion rather than a silent `as`: if the cap ever
    // moves, this fails loudly instead of writing a wrapped count that reframes the section.
    let record_count = u32::try_from(seg.records.len()).map_err(|_| {
        BackupError::Encode(format!(
            "refusing to write a segment with {} records: the on-disk record count is a u32",
            seg.records.len()
        ))
    })?;
    body.extend_from_slice(&record_count.to_be_bytes());
    for r in &seg.records {
        put_record(&mut body, r)?;
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
///   - `Ok(Some(..))` — a complete section, whatever its plane;
///   - `Ok(None)` — a TORN TAIL: fewer bytes remain than the section claims, which is what
///     an interrupted append looks like. The caller keeps everything before it and flags
///     the tail. Remedy: run the backup again.
///   - `Err(..)` — CORRUPTION: a length prefix beyond the cap, or a malformed body. The
///     remedy is different ("this medium is damaged"), so the verdicts never collapse.
///
/// An unrecognised plane tag is NOT one of the failure cases: it decodes into a normal
/// [`Segment`] carrying [`Plane::Unknown`]. The record codec is plane-independent — one
/// shape, one codec, by design — so a newer Cairn's plane is fully readable AS BYTES even
/// though this build cannot route it. Keeping it in the segment list is what lets the single
/// global chain traverse it; dropping it (as this function once did) silently broke the chain
/// for every segment after it.
pub(crate) fn take_section(rest: &[u8]) -> Result<Option<(Segment, &[u8])>, BackupError> {
    if rest.len() < 4 {
        return Ok(None); // not even a complete length prefix — torn
    }
    let len = u32::from_be_bytes([rest[0], rest[1], rest[2], rest[3]]) as usize;
    if len > MAX_SECTION_BYTES {
        return Err(BackupError::Damaged(format!(
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
        .ok_or_else(|| BackupError::Damaged("empty medium section: no plane tag".into()))?;
    if b.len() < 4 {
        return Err(BackupError::Damaged(
            "medium section truncated: no segment index after the plane tag".into(),
        ));
    }
    let (idx, b) = b.split_at(4);
    let index = u32::from_be_bytes(idx.try_into().expect("4 bytes"));
    let (prev, b) = take_chunk(b)?;
    let (self_id, b) = take_chunk(b)?;
    let (att, b) = take_chunk(b)?;
    if b.len() < 4 {
        return Err(BackupError::Damaged(
            "medium section truncated: no record count".into(),
        ));
    }
    let (count_bytes, mut b) = b.split_at(4);
    let record_count = u32::from_be_bytes(count_bytes.try_into().expect("4 bytes"));

    // NOTHING DERIVED FROM `record_count` IS EVER ALLOCATED. It is a raw, unauthenticated
    // `u32` read straight off the medium before a single record is parsed, so a flipped bit
    // makes it ~4 billion. Reserving that much capacity asks for ~447 GB, and Rust ABORTS THE
    // PROCESS on an allocation failure — it is not a catchable `Result` — so one bit flip on
    // an otherwise healthy medium would kill `verify-backup`/`restore` outright, mid-disaster.
    //
    // Earlier versions bounded the hint (by the bytes remaining, then by a fixed ceiling)
    // rather than removing it. Bounding is fragile in two ways: the bytes-remaining bound
    // alone still permitted a ~2 GB reservation on a full-size section, and ANY arithmetic
    // from an untrusted length into an allocation is a pattern a reader — and CodeQL's
    // `rust/uncontrolled-allocation-size` — has to re-derive as safe every time it is read.
    // `Vec::push` amortises to the same O(n) without it, and the loop below is what enforces
    // the real count honestly, returning a clean `BackupError` the moment the bytes run out.
    // The allocation now grows only as records are actually decoded, so it can never exceed
    // what the medium truly contains.
    let mut records = Vec::new();
    for _ in 0..record_count {
        let (r, next) = take_record(b)?;
        records.push(r);
        b = next;
    }
    if !b.is_empty() {
        return Err(BackupError::Damaged(format!(
            "medium section has {} trailing byte(s) after its {record_count} record(s)",
            b.len()
        )));
    }
    let to_string = |v: &[u8], what: &str| -> Result<String, BackupError> {
        std::str::from_utf8(v)
            .map(str::to_string)
            .map_err(|_| BackupError::Damaged(format!("segment {what} is not UTF-8")))
    };
    Ok(Some((
        Segment {
            plane: Plane::from_tag(plane_tag),
            index,
            prev_commitment: to_string(prev, "prev_commitment")?,
            self_node_id_hex: to_string(self_id, "self_node_id_hex")?,
            attestation: (!att.is_empty()).then(|| att.to_vec()),
            records,
        },
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
            let (back, rest) = take_section(&out).expect("no error").expect("not torn");
            assert!(rest.is_empty());
            assert_eq!(back, seg);
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
        let (back, _) = take_section(&out).unwrap().unwrap();
        assert_eq!(back.attestation, None, "unsigned stays unsigned");
        assert_eq!(back.self_node_id_hex, "abcd", "and still names itself");
    }

    /// An unrecognised plane tag decodes into a FULL segment carrying `Plane::Unknown` —
    /// records and all — not a stub, and never a dropped segment.
    ///
    /// This is the fix for the defect that let a newer Cairn's medium read as damaged
    /// (#500 slice 2a review). The record codec is plane-independent, so every record of a
    /// plane this build cannot route is still fully readable AS BYTES. Keeping them is what
    /// lets the single global chain traverse the segment: `chain_report` computes the next
    /// `prev_commitment` from a segment's records, so a segment whose records were thrown
    /// away breaks the chain for everything after it.
    #[test]
    fn an_unknown_plane_tag_keeps_its_records_so_the_chain_can_traverse_it() {
        let seg = segment(Plane::Clinical, 5, 12);
        let mut out = Vec::new();
        put_segment(&mut out, &seg).unwrap();
        out[4] = 99; // the plane tag is the first byte of the body, after the u32 length
        let (back, rest) = take_section(&out).unwrap().unwrap();
        assert!(
            rest.is_empty(),
            "an unknown section is consumed whole, by its length"
        );
        assert_eq!(
            back.plane,
            Plane::Unknown(99),
            "the raw tag is carried verbatim"
        );
        assert!(!back.plane.is_known());
        assert_eq!(back.index, 5);
        assert_eq!(
            back.records.len(),
            12,
            "every record of an unknown plane must survive the parse — throwing them away is \
             what broke the chain for every segment after it"
        );
        assert_eq!(
            back.records, seg.records,
            "and they must be byte-identical to what was written"
        );
        // The tag round-trips: an unknown plane can be re-serialised without loss, so a tool
        // that copies a medium does not silently rewrite a newer Cairn's plane.
        assert_eq!(back.plane.tag(), 99);
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
        assert!(
            matches!(err, BackupError::Damaged(_)),
            "an over-cap length is DAMAGE, whose remedy is the opposite of a torn tail's: {err:?}"
        );
        assert!(err.to_string().contains("cap"), "must name the cap: {err}");
    }

    /// A body-level malformation is DAMAGE (`Err`), never a torn tail (`Ok(None)`).
    ///
    /// The torn-tail test above cuts the OUTER bytes, so it always returns at the
    /// length-prefix check and never reaches the body parser at all. These three cases hold
    /// the other half of invariant 4: a section whose outer length is intact but whose body
    /// is malformed must not be mistaken for an interrupted append, because "run the backup
    /// again" would then append after real damage and orphan everything between.
    #[test]
    fn a_malformed_section_body_is_damage_not_a_torn_tail() {
        // The three malformed bodies, each with an HONEST outer length so the only fault is
        // inside: (name, body bytes).
        let mut no_index = vec![Plane::Node.tag()];
        no_index.extend_from_slice(&[0u8; 3]); // 3 bytes where a 4-byte index belongs

        let mut no_count = vec![Plane::Node.tag()];
        no_count.extend_from_slice(&0u32.to_be_bytes());
        put_chunk(&mut no_count, b"").unwrap(); // prev_commitment
        put_chunk(&mut no_count, b"").unwrap(); // self_node_id_hex
        put_chunk(&mut no_count, b"").unwrap(); // attestation
                                                // ...and then nothing where the record count belongs.

        let mut bad_utf8 = vec![Plane::Node.tag()];
        bad_utf8.extend_from_slice(&0u32.to_be_bytes());
        put_chunk(&mut bad_utf8, &[0xff, 0xfe]).unwrap(); // prev_commitment: not UTF-8
        put_chunk(&mut bad_utf8, b"").unwrap();
        put_chunk(&mut bad_utf8, b"").unwrap();
        bad_utf8.extend_from_slice(&0u32.to_be_bytes()); // record count 0

        for (what, body) in [
            ("no segment index", no_index),
            ("no record count", no_count),
            ("non-UTF-8 prev_commitment", bad_utf8),
        ] {
            let mut out = Vec::new();
            out.extend_from_slice(&(body.len() as u32).to_be_bytes());
            out.extend_from_slice(&body);
            match take_section(&out) {
                Err(BackupError::Damaged(_)) => {}
                other => panic!(
                    "{what}: a malformed body must be Err(Damaged) — a torn tail's remedy \
                     (\"run the backup again\") would append after real damage. Got {other:?}"
                ),
            }
        }
    }

    /// Trailing bytes INSIDE a section — a declared `record_count` lower than the records
    /// actually present — are damage.
    ///
    /// Without this guard the medium parses clean while records are silently dropped: on a
    /// SIGNED segment the attestation's `record_count` conjunct would catch it, but on an
    /// UNSIGNED segment nothing would, and if the shortfall is in the last segment the
    /// watermark is then computed over a silently-shortened record set. "A medium that
    /// parses cleanly while missing records" is invariant 6's stated failure shape.
    #[test]
    fn trailing_bytes_inside_a_section_are_damage() {
        // `attestation: None` keeps the body layout easy to point at: the empty
        // prev_commitment, the 4-byte self id and the empty attestation are all fixed width.
        let seg = Segment {
            attestation: None,
            ..segment(Plane::Clinical, 0, 3)
        };
        let mut out = Vec::new();
        put_segment(&mut out, &seg).unwrap();
        // Rewrite the record count from 3 to 2, leaving the third record's bytes in place as
        // an unaccounted-for tail inside an otherwise well-formed section.
        //   outer len 4 | tag 1 | index 4 | chunk("") 4 | chunk("abcd") 8 | chunk("") 4
        let count_at = 4 + 1 + 4 + 4 + (4 + "abcd".len()) + 4;
        out[count_at..count_at + 4].copy_from_slice(&2u32.to_be_bytes());
        let err = take_section(&out).expect_err("a short record count must be refused");
        assert!(
            matches!(err, BackupError::Damaged(ref m) if m.contains("trailing")),
            "must name the unaccounted bytes: {err:?}"
        );
    }

    /// `record_count` is corrupted to an absurd value (`u32::MAX`) while the body behind it
    /// is short. An earlier `take_section` reserved `record_count` `MediumRecord`s EAGERLY —
    /// hundreds of gigabytes — and Rust's allocator ABORTS THE WHOLE PROCESS on a failure that
    /// large, not a catchable error, so one flipped bit killed `verify-backup`/`restore`
    /// outright. Nothing derived from `record_count` is allocated any more (see `take_section`).
    ///
    /// This test does not reproduce that abort — deliberately: doing so means attempting the
    /// very allocation the fix exists to prevent. It pins the FIXED behaviour instead: parsing
    /// proceeds straight to the record loop, which runs out of bytes on the very first record
    /// and returns a clean `BackupError`.
    #[test]
    fn an_absurd_record_count_over_a_short_body_errors_rather_than_aborting() {
        let mut body = Vec::new();
        body.push(Plane::Clinical.tag());
        body.extend_from_slice(&0u32.to_be_bytes()); // index
        put_chunk(&mut body, b"").unwrap(); // prev_commitment
        put_chunk(&mut body, b"abcd").unwrap(); // self_node_id_hex
        put_chunk(&mut body, b"").unwrap(); // attestation: none
        body.extend_from_slice(&u32::MAX.to_be_bytes()); // record_count: absurd

        let mut out = Vec::new();
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        out.extend_from_slice(&body);

        let err = take_section(&out)
            .expect_err("an absurd record_count over a short body must Err, never abort");
        assert!(
            matches!(err, BackupError::Damaged(ref m) if m.contains("truncated")),
            "must be the honest parse failure, named: {err:?}"
        );
    }

    /// `put_segment` refuses an over-cap section at the source, in every build, naming the
    /// cap — never writing one that `take_section` could only ever reject as corruption.
    ///
    /// A single RECORD cannot carry enough bytes to trip the SECTION cap on its own — each
    /// record's `signed_bytes` is itself capped at `chunk::MAX_CHUNK_BYTES` (8 MiB) by
    /// `put_chunk`'s own cap check, which would fire first and prove nothing about this
    /// one. So this pushes the SECTION over its cap with many chunk-cap-sized records
    /// instead, exactly the way a real oversized capture would.
    #[test]
    fn put_segment_refuses_an_over_cap_section_at_the_source() {
        let per_record_len = crate::chunk::MAX_CHUNK_BYTES + 4 + 1 + 8;
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
            matches!(err, BackupError::Encode(_)),
            "refusing to WRITE is an Encode fault: {err:?}"
        );
        assert!(
            err.to_string().contains("cap"),
            "the refusal must NAME the cap: {err}"
        );
        assert!(
            out.is_empty(),
            "a refused section must write nothing at all, not a partial write"
        );
    }

    /// An EMPTY segment is refused at the source.
    ///
    /// `attest::segment_commitment(&[])` is the multihash of the empty string — the SAME
    /// value on every medium ever written. So if an empty segment could be written, the
    /// segment AFTER it would carry a `prev_commitment` identical across all media, and a
    /// genuine segment could be spliced in from a different medium with its plane, index and
    /// predecessor all matching and its attestation verifying. That is exactly the splice
    /// invariant 3 promises is impossible, so the promise is kept by never writing the
    /// segment that would break it.
    #[test]
    fn put_segment_refuses_an_empty_segment() {
        let seg = Segment {
            records: vec![],
            ..segment(Plane::Clinical, 0, 1)
        };
        let mut out = Vec::new();
        let err = put_segment(&mut out, &seg).expect_err("an empty segment must be refused");
        assert!(
            matches!(err, BackupError::Encode(_)),
            "refusing to WRITE is an Encode fault: {err:?}"
        );
        assert!(
            err.to_string().contains("EMPTY"),
            "the refusal must say what was wrong: {err}"
        );
        assert!(out.is_empty(), "nothing may be written");
    }
}
