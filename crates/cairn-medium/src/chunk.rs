//! The `[u32 big-endian length][bytes]` primitive every other module frames with: `marker`'s
//! self-marker block, `container`'s event frames, and (from slice 2a) `segment`'s appended
//! records all reduce to a sequence of these chunks. Pure — no I/O, no allocation beyond the
//! `Vec` being written into or the slice being read from — so a caller can round-trip it with
//! nothing larger than a byte slice.

use crate::error::BackupError;

/// Upper bound on a single length-prefixed chunk on the medium (event frame OR marker). A
/// signed node-event is a few hundred bytes; 8 MiB caps a corrupt length prefix so a bit-flip
/// can never force a multi-GiB allocation during parse. Mirrors the wire frame cap in `sync.rs`.
pub(crate) const MAX_CHUNK_BYTES: usize = 8 * 1024 * 1024;

/// Append a `[u32 big-endian length][bytes]` chunk. The cap is asserted (debug) so a future
/// change that lifts the upstream size bound can never silently truncate a length prefix.
pub(crate) fn put_chunk(out: &mut Vec<u8>, bytes: &[u8]) {
    debug_assert!(
        bytes.len() <= MAX_CHUNK_BYTES,
        "chunk exceeds the medium frame cap"
    );
    out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(bytes);
}

/// Read one `[u32 length][bytes]` chunk, returning (chunk, remainder). Errors (never panics)
/// on a truncated or over-cap frame — a partial/corrupt medium is reported, not accepted.
pub(crate) fn take_chunk(rest: &[u8]) -> Result<(&[u8], &[u8]), BackupError> {
    if rest.len() < 4 {
        return Err(BackupError::Decode(format!(
            "truncated medium: {} byte(s) without a complete length prefix",
            rest.len()
        )));
    }
    let len = u32::from_be_bytes([rest[0], rest[1], rest[2], rest[3]]) as usize;
    if len > MAX_CHUNK_BYTES {
        return Err(BackupError::Decode(format!(
            "medium frame length {len} exceeds {MAX_CHUNK_BYTES}-byte cap (corrupt)"
        )));
    }
    let end = 4 + len;
    if rest.len() < end {
        return Err(BackupError::Decode(format!(
            "truncated medium: frame claims {len} bytes, only {} remain",
            rest.len() - 4
        )));
    }
    Ok((&rest[4..end], &rest[end..]))
}

#[cfg(test)]
mod tests {
    use crate::container::{parse_container, serialize_container};
    use crate::error::BackupError;
    use crate::testkit::{enroll, sk};

    #[test]
    fn parse_rejects_a_truncated_frame() {
        let k = sk();
        let mut image = serialize_container(None, &[enroll(&k, "Self")]);
        image.pop(); // last frame now claims more bytes than remain
        assert!(matches!(
            parse_container(&image),
            Err(BackupError::Decode(_))
        ));
    }
}
