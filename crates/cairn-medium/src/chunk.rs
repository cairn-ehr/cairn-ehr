//! The `[u32 big-endian length][bytes]` primitive every other module frames with: `marker`'s
//! self-marker block, `container`'s event frames, and (from slice 2a) `segment`'s appended
//! records all reduce to a sequence of these chunks. Pure — no I/O, no allocation beyond the
//! `Vec` being written into or the slice being read from — so a caller can round-trip it with
//! nothing larger than a byte slice.

use crate::error::BackupError;

/// Upper bound on a single length-prefixed chunk on the medium (event frame OR marker). A
/// signed node-event is a few hundred bytes; 8 MiB caps a corrupt length prefix so a bit-flip
/// can never force a multi-GiB allocation during parse. Mirrors the wire frame cap in
/// `cairn-node/src/sync.rs` (note `cairn-sync`'s `MAX_FRAME_BYTES` is a different, larger
/// bound: it caps a whole unpaginated BATCH response, not one event).
pub(crate) const MAX_CHUNK_BYTES: usize = 8 * 1024 * 1024;

/// Append a `[u32 big-endian length][bytes]` chunk, refusing an over-cap chunk at the source.
///
/// FALLIBLE, and it must be (#500 slice 2a review). This was a `debug_assert!` — so in a
/// RELEASE build an over-cap chunk was written successfully and then could NEVER be read
/// back, because `take_chunk` refuses anything over the cap as corruption. Write succeeds,
/// read fails, permanently: the medium reports healthy until the disaster, and then the
/// WHOLE FILE fails to parse, from the magic header on, because one frame in the middle is
/// unreadable. In a debug build it was worse in a different way — the assert panicked the
/// backup process, the one process that must always be able to produce something.
///
/// `segment::put_segment` already refuses an over-cap SECTION this way and its doc claimed
/// this function did the same; it did not. Now it does, and the two are honestly symmetric.
pub(crate) fn put_chunk(out: &mut Vec<u8>, bytes: &[u8]) -> Result<(), BackupError> {
    if bytes.len() > MAX_CHUNK_BYTES {
        return Err(BackupError::Encode(format!(
            "refusing to write a {}-byte medium frame: it exceeds the {MAX_CHUNK_BYTES}-byte \
             cap, and `take_chunk` would reject it as corruption on every future read — the \
             whole medium would become unparseable",
            bytes.len()
        )));
    }
    // The cap above is what makes this `as u32` lossless: 8 MiB is far below u32::MAX, so the
    // length prefix can never wrap and silently reframe the rest of the medium.
    out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(bytes);
    Ok(())
}

/// Read one `[u32 length][bytes]` chunk, returning (chunk, remainder). Errors (never panics)
/// on a truncated or over-cap frame — a partial/corrupt medium is reported, not accepted.
pub(crate) fn take_chunk(rest: &[u8]) -> Result<(&[u8], &[u8]), BackupError> {
    if rest.len() < 4 {
        return Err(BackupError::Damaged(format!(
            "truncated medium: {} byte(s) without a complete length prefix",
            rest.len()
        )));
    }
    let len = u32::from_be_bytes([rest[0], rest[1], rest[2], rest[3]]) as usize;
    if len > MAX_CHUNK_BYTES {
        return Err(BackupError::Damaged(format!(
            "medium frame length {len} exceeds {MAX_CHUNK_BYTES}-byte cap (corrupt)"
        )));
    }
    let end = 4 + len;
    if rest.len() < end {
        return Err(BackupError::Damaged(format!(
            "truncated medium: frame claims {len} bytes, only {} remain",
            rest.len() - 4
        )));
    }
    Ok((&rest[4..end], &rest[end..]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::{parse_container, serialize_container};
    use crate::error::BackupError;
    use crate::testkit::{enroll, sk};

    #[test]
    fn parse_rejects_a_truncated_frame() {
        let k = sk();
        let mut image =
            serialize_container(None, &[enroll(&k, "Self")]).expect("fixture fits the cap");
        image.pop(); // last frame now claims more bytes than remain
        let err = parse_container(&image).expect_err("a truncated frame must be refused");
        assert!(
            matches!(err, BackupError::Damaged(ref m) if m.contains("only")),
            "a truncated frame is DAMAGE and must say so: {err:?}"
        );
    }

    /// An over-cap chunk is refused at WRITE time, in every build — not asserted away in
    /// debug and written unreadable in release. Without this, the write succeeds and the
    /// whole medium becomes unparseable on the next read.
    #[test]
    fn put_chunk_refuses_an_over_cap_frame_at_the_source() {
        let mut out = Vec::new();
        let oversized = vec![0u8; MAX_CHUNK_BYTES + 1];
        let err = put_chunk(&mut out, &oversized).expect_err("over-cap must be refused");
        assert!(
            matches!(err, BackupError::Encode(_)),
            "refusing to WRITE is an Encode fault, not a property of a medium: {err:?}"
        );
        assert!(
            out.is_empty(),
            "nothing may be written before the refusal — a bare length prefix would wedge \
             every future read of this medium"
        );
        // Exactly at the cap is fine: the bound is inclusive.
        assert!(put_chunk(&mut out, &vec![0u8; MAX_CHUNK_BYTES]).is_ok());
    }
}
