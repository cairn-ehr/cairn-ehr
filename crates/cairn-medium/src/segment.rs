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

// NOTE ON THE `expect(dead_code)` ATTRIBUTES BELOW (task 4 of the slice-2a plan): this
// module lands one task ahead of its caller. `put_record`/`take_record` are only
// exercised by this file's own tests until the very next task (`Plane`/`Segment`/section
// framing) wires them into `put_segment`/`take_section` in this same module. `expect`
// rather than `allow` is deliberate: it is self-cleaning — once that caller lands, the
// lint stops firing, the `expect` itself becomes an error ("this lint expectation is
// unfulfilled"), and the build forces its removal instead of it going stale silently.
// The attribute goes ONLY on the two unreferenced entry points (`put_record`,
// `take_record`) — dead-code liveness propagates from an allowed/expected root to
// whatever it calls, so the flag consts and `take_optional` are already covered through
// them and must NOT carry their own `expect`, or that second attribute finds nothing
// left to suppress and itself becomes an "unfulfilled expectation" error.

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
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "wired up by the next task's put_segment/take_section"
    )
)]
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
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "wired up by the next task's put_segment/take_section"
    )
)]
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
}
