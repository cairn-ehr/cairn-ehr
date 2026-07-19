//! Length-prefixed wire framing — the ONE implementation of the untrusted-length
//! discipline (issue #212, drift pair 3; supersedes the hand-mirrored copies noted
//! in #202).
//!
//! Both wire planes frame every message as `[len: u32 BE][payload…]`, and on both
//! the length is **attacker-controlled input**: the node plane reads frames from a
//! pinned-but-possibly-compromised peer, the clinical plane from any client that
//! can reach the port (WireGuard is the assumed perimeter, not authentication).
//! The two rules that make that safe are exactly the ones that must never drift
//! between the two hand-written I/O wrappers:
//!
//! 1. **Refuse before allocating** (read side): a hostile/corrupt prefix of up to
//!    4 GiB must be rejected by comparing against the plane's cap BEFORE `vec![0; n]`.
//! 2. **Refuse at the source** (write side): an over-cap frame must never be sent —
//!    it would cross the wire in full only to be refused by the peer's read cap,
//!    with nothing in the sender's log to say why the peer stopped converging.
//!    Checking at the source also makes u32 truncation (> 4 GiB payloads)
//!    unreachable.
//!
//! The **cap is a per-plane policy**, deliberately NOT shared: the node plane caps
//! at 8 MiB (envelopes are tiny), the clinical plane at 64 MiB (an unpaginated full
//! sweep — issue #101). Each caller passes its own cap and owns its refusal
//! message; this module owns the decision.

/// A frame length that exceeds the caller's cap (or, on encode, the u32 wire
/// format itself). Carries the numbers so the caller can compose a legible,
/// plane-specific refusal message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameOverCap {
    /// The offending length: the payload length on encode, the decoded prefix on decode.
    pub len: usize,
    /// The cap it was checked against.
    pub cap: usize,
}

impl std::fmt::Display for FrameOverCap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "frame length {} exceeds {}-byte cap", self.len, self.cap)
    }
}

/// Write-side: the 4-byte BE length prefix for `len` payload bytes, or a refusal
/// if `len` is over `cap` (or over `u32::MAX`, which the wire format cannot carry —
/// folded into the same refusal so a cap mis-set above 4 GiB can never silently
/// truncate).
pub fn encode_len_prefix(len: usize, cap: usize) -> Result<[u8; 4], FrameOverCap> {
    if len > cap || len > u32::MAX as usize {
        return Err(FrameOverCap { len, cap });
    }
    Ok((len as u32).to_be_bytes())
}

/// Read-side: decode a received 4-byte BE prefix into a payload length, or a
/// refusal if it exceeds `cap`. Callers MUST call this before allocating the
/// read buffer — that ordering is the entire point (rule 1 above).
pub fn decode_len_prefix(prefix: [u8; 4], cap: usize) -> Result<usize, FrameOverCap> {
    let n = u32::from_be_bytes(prefix) as usize;
    if n > cap {
        return Err(FrameOverCap { len: n, cap });
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_at_and_below_the_cap_boundary() {
        // The cap must never break a legitimate exchange; the boundary is inclusive.
        let cap = 1024;
        for len in [0usize, 1, 1023, 1024] {
            let prefix = encode_len_prefix(len, cap).expect("in-cap length must encode");
            assert_eq!(decode_len_prefix(prefix, cap).unwrap(), len);
        }
    }

    #[test]
    fn encode_refuses_one_byte_over_the_cap() {
        assert_eq!(
            encode_len_prefix(1025, 1024),
            Err(FrameOverCap {
                len: 1025,
                cap: 1024
            })
        );
    }

    #[test]
    fn decode_refuses_one_byte_over_the_cap() {
        let prefix = 1025u32.to_be_bytes();
        assert_eq!(
            decode_len_prefix(prefix, 1024),
            Err(FrameOverCap {
                len: 1025,
                cap: 1024
            })
        );
    }

    #[test]
    fn decode_refuses_a_hostile_u32_max_prefix_under_both_plane_caps() {
        // The classic attack shape: a corrupt/hostile 4 GiB prefix demanding a
        // doomed allocation. Must be refused under every real plane cap.
        let hostile = u32::MAX.to_be_bytes();
        for cap in [8 * 1024 * 1024usize, 64 * 1024 * 1024] {
            let err = decode_len_prefix(hostile, cap).unwrap_err();
            assert_eq!(err.len, u32::MAX as usize);
            assert_eq!(err.cap, cap);
        }
    }

    #[test]
    fn encode_refuses_a_payload_the_u32_wire_format_cannot_carry() {
        // Even under a (mis-set) cap above 4 GiB, encode must refuse rather than
        // silently truncate the prefix.
        #[cfg(target_pointer_width = "64")]
        {
            let too_big = u32::MAX as usize + 1;
            assert!(encode_len_prefix(too_big, usize::MAX).is_err());
        }
    }

    #[test]
    fn the_refusal_names_both_numbers_legibly() {
        // Operators read this out of a log line; it must name the length AND the cap.
        let msg = FrameOverCap { len: 5, cap: 4 }.to_string();
        assert!(
            msg.contains('5') && msg.contains('4') && msg.contains("cap"),
            "{msg}"
        );
    }
}
