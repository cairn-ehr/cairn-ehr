//! Length-prefixed framing for the clinical plane: `[u32 big-endian length][payload]`.
//!
//! Moved verbatim from `cairn-sync/src/main.rs` (slice 2b). The DECISION — cap before
//! allocating, refuse at the source, u32 truncation unreachable — lives in the shared
//! `cairn_event::framing` core (#212); this module owns the clinical plane's CAP and its
//! refusal messages, because the cap is a per-plane policy (the node plane's is 8 MiB).

use std::io::{self, Read, Write};

/// Read-side frame cap (issue #202, porting the cairn-node `MAX_FRAME_BYTES`
/// discipline). The 4-byte length prefix is attacker-controlled on both wire ends —
/// the server reads request frames from ANY client that can reach the port (WireGuard
/// is the assumed perimeter, not authentication), and the puller reads response frames
/// from its peer — so an unchecked prefix lets one hostile/corrupt u32 demand a 4 GiB
/// allocation. Unlike the node plane (one frame per event, 8 MiB), a response here
/// carries a whole BATCH of events, so the cap is batch-scale: 64 MiB holds ~20k
/// typical events (~1.5 KiB signed, hex-doubled on the wire) with room to spare.
///
/// Since slice 2b the cap is a BACKSTOP rather than the sizing constraint. A paged
/// request ([`crate::DEFAULT_PAGE_EVENTS`] = 500 events, ≈2 MiB) lands 32× under it.
/// Two paths can still reach it: a request that omits `limit`, and the legacy
/// `EventsAfter` arm, which is unpaginated by construction. A response on either that
/// outgrows the cap fails LOUDLY with this cap named in the error, and the remedy is
/// to ask for pages — #101 item 1, closed by slice 2b; items 2 and 3 stay open, so
/// #101 does too.
pub const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;

pub fn write_frame(s: &mut impl Write, b: &[u8]) -> io::Result<()> {
    // Refuse at the SOURCE, mirroring read_frame's cap (PR #225 review): an over-cap
    // frame would cross the wire in full only to be refused by the peer's read cap,
    // with nothing in the SERVING node's log to say why its peer stopped converging.
    // The decision (cap + u32-truncation-unreachable) lives in the shared
    // cairn_event::framing core (#212); refusing before the prefix is written stays
    // here — a bare length prefix with no body would wedge the reader.
    // A response that outgrows the cap is an UNPAGED one (slice 2b closed #101 item
    // 1), so the refusal names the remedy an operator can actually apply rather than
    // an issue number to go and read.
    let prefix =
        cairn_event::framing::encode_len_prefix(b.len(), MAX_FRAME_BYTES).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "refusing to send: {e}. This response is UNPAGED: have the puller \
                     ask for pages (`--page N`), or upgrade a peer that is still \
                     serving unpaginated responses."
                ),
            )
        })?;
    s.write_all(&prefix)?;
    s.write_all(b)?;
    s.flush()
}

pub fn read_frame(s: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut len = [0u8; 4];
    s.read_exact(&mut len)?;
    // Refuse BEFORE allocating: the prefix is untrusted input (see MAX_FRAME_BYTES);
    // the decision is the shared cairn_event::framing core (#212).
    let n = cairn_event::framing::decode_len_prefix(len, MAX_FRAME_BYTES)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
    let mut buf = vec![0u8; n];
    s.read_exact(&mut buf)?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_frame_refuses_an_over_cap_length_prefix() {
        // A length prefix is attacker-controlled on BOTH sides of the wire: the
        // server reads request frames from any client that can reach the port
        // (WireGuard is the assumed perimeter, not authentication), and the puller
        // reads response frames from its peer. A hostile/corrupt u32 prefix of up
        // to 4 GiB must be refused BEFORE the read buffer is allocated — as
        // InvalidData with a legible message, never a doomed multi-GiB allocation
        // that surfaces as an opaque UnexpectedEof.
        let mut hostile = std::io::Cursor::new(u32::MAX.to_be_bytes().to_vec());
        let err = read_frame(&mut hostile).expect_err("an over-cap prefix must be refused");
        assert_eq!(
            err.kind(),
            std::io::ErrorKind::InvalidData,
            "cap refusal must be InvalidData, got: {err}"
        );
        assert!(
            err.to_string().contains("cap"),
            "the refusal names the cap so an operator can tell it from line noise: {err}"
        );

        // The boundary is exact: one byte over the cap is refused too.
        let over = (MAX_FRAME_BYTES as u32 + 1).to_be_bytes();
        let mut s = std::io::Cursor::new(over.to_vec());
        assert_eq!(
            read_frame(&mut s).unwrap_err().kind(),
            std::io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn read_frame_round_trips_an_in_cap_frame() {
        // The cap must never break a legitimate exchange: an in-cap frame still
        // round-trips byte-identically through write_frame/read_frame.
        let payload = vec![0xAB_u8; 1024];
        let mut wire = Vec::new();
        write_frame(&mut wire, &payload).unwrap();
        let mut r = std::io::Cursor::new(wire);
        assert_eq!(read_frame(&mut r).unwrap(), payload);
    }

    #[test]
    // The asserts ARE on constants — deliberately: this is a standing bounds guard
    // on MAX_FRAME_BYTES itself (same class as required_pgx_floor_is_itself_a_valid
    // _triple), so a future edit of the const outside the window these asserts fix
    // fails a named test instead of silently shipping.
    #[allow(clippy::assertions_on_constants)]
    fn frame_cap_holds_a_realistic_event_batch() {
        // A response here carries a whole BATCH of events, so the node plane's
        // per-event 8 MiB cap cannot be ported verbatim. What this cap has to bound
        // is an UNLIMITED request — one that omits `limit`, plus the legacy
        // `EventsAfter` arm — which ships the whole log suffix as ONE hex-encoded
        // JSON frame. So it must sit far above a realistic harness batch (~1.5
        // KiB/event, hex doubling → ~3 KiB/event on the wire) while still bounding a
        // hostile 4 GiB prefix. A PAGED request is nowhere near it: 500 events is
        // ≈2 MiB. If an unlimited response outgrows the cap it fails LOUDLY with the
        // cap message, and the remedy is to ask for pages.
        assert!(
            MAX_FRAME_BYTES >= 16 * 1024 * 1024,
            "cap must hold a realistic unlimited batch (a paged one is ≈2 MiB)"
        );
        assert!(
            MAX_FRAME_BYTES <= 256 * 1024 * 1024,
            "cap must still bound a hostile 4 GiB prefix to a refusable size"
        );
    }

    #[test]
    fn write_frame_refuses_an_over_cap_frame() {
        // PR #225 review: the read cap alone is asymmetric — a serving node whose
        // log outgrew MAX_FRAME_BYTES would serialize and SHIP the whole over-cap
        // response, which then fails only at the peer's read cap: the bytes cross
        // the wire for nothing and the serving operator's own log shows no error.
        // Refusing at the source puts the failure next to its cause (and past
        // u32::MAX the length prefix would silently truncate — the write cap makes
        // that unreachable). Nothing may hit the wire before the refusal: a bare
        // length prefix with no body would wedge the reading peer.
        let payload = vec![0u8; MAX_FRAME_BYTES + 1];
        let mut wire = Vec::new();
        let err = write_frame(&mut wire, &payload).expect_err("an over-cap frame must be refused");
        assert_eq!(
            err.kind(),
            std::io::ErrorKind::InvalidData,
            "cap refusal must be InvalidData, got: {err}"
        );
        assert!(
            err.to_string().contains("cap"),
            "the refusal names the cap so the operator can tell it from an I/O fault: {err}"
        );
        assert!(
            wire.is_empty(),
            "nothing may be written before the refusal (a bare prefix would wedge the peer)"
        );

        // The boundary is exact: a frame of exactly MAX_FRAME_BYTES still ships.
        let at_cap = vec![0u8; MAX_FRAME_BYTES];
        let mut wire = Vec::new();
        write_frame(&mut wire, &at_cap).expect("an at-cap frame must still ship");
        assert_eq!(wire.len(), 4 + MAX_FRAME_BYTES);
    }
}
