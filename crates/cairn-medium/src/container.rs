//! Magic dispatch and the on-disk framing of every medium revision: the CAIRNB1/CAIRNB2 magic
//! headers, the marker-kind discriminant, the [`Container`] parse result, and the
//! serialize/parse pair that round-trips a medium image. This is the "outermost" format layer —
//! it decides which revision a byte slice claims to be and hands off the marker block to
//! `marker::put_marker` and the event frames to `chunk`'s primitive.

use crate::chunk::{put_chunk, take_chunk};
use crate::error::BackupError;
use crate::marker::{put_marker, SelfMarker};

/// Magic header for the original marker-less medium (ADR-0026 slice B). Kept for backward
/// compatibility: such a medium parses to events with `self_marker == None`.
pub const MEDIUM_MAGIC_V1: &[u8] = b"CAIRNB1\n";
/// Magic header for the self-marked medium (issue #53). Carries a marker block before the
/// event frames. Distinct from the keystore's `CAIRNK1` so the artifacts can never be confused.
pub const MEDIUM_MAGIC_V2: &[u8] = b"CAIRNB2\n";

/// Marker kind discriminant bytes (first byte of the CAIRNB2 marker block). `pub(crate)`
/// because `marker::put_marker` writes the same bytes this module's `parse_container` reads —
/// the two must agree on the wire, so they share one definition rather than two.
pub(crate) const KIND_NONE: u8 = 0;
pub(crate) const KIND_UNSIGNED: u8 = 1;
pub(crate) const KIND_SIGNED: u8 = 2;

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

/// Serialize a full CAIRNB2 container: magic ++ marker block ++ event frames. Pure. The event
/// order is preserved for legibility but is set-union-independent on restore (convergence is
/// by content-address).
pub fn serialize_container(marker: Option<&SelfMarker>, events: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::with_capacity(MEDIUM_MAGIC_V2.len() + 1 + 32 * events.len());
    out.extend_from_slice(MEDIUM_MAGIC_V2);
    put_marker(&mut out, marker);
    for e in events {
        put_chunk(&mut out, e);
    }
    out
}

/// Parse a medium image into its marker + event set. Handles BOTH formats: a CAIRNB2 medium
/// yields its marker; a legacy CAIRNB1 medium yields `self_marker: None`. Errors (never
/// panics) on bad magic, an unknown marker kind, or a truncated frame.
pub fn parse_container(bytes: &[u8]) -> Result<Container, BackupError> {
    if let Some(rest) = bytes.strip_prefix(MEDIUM_MAGIC_V2) {
        let (&kind, mut rest) = rest
            .split_first()
            .ok_or_else(|| BackupError::Decode("CAIRNB2 medium missing marker kind".into()))?;
        let self_marker = match kind {
            KIND_NONE => None,
            KIND_UNSIGNED => {
                let (id, r) = take_chunk(rest)?;
                rest = r;
                let id = std::str::from_utf8(id)
                    .map_err(|_| BackupError::Decode("unsigned marker is not UTF-8".into()))?;
                Some(SelfMarker::Unsigned(id.to_string()))
            }
            KIND_SIGNED => {
                let (att, r) = take_chunk(rest)?;
                rest = r;
                Some(SelfMarker::Signed(att.to_vec()))
            }
            other => {
                return Err(BackupError::Decode(format!("unknown marker kind {other}")));
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
    } else {
        Err(BackupError::Decode(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::marker::build_self_attestation;
    use crate::testkit::{enroll, kid, node_id, sk};

    #[test]
    fn container_roundtrips_unsigned_marker_and_events() {
        let k = sk();
        let g = enroll(&k, "Self");
        let events = vec![g.clone()];
        let marker = SelfMarker::Unsigned(node_id(&g));
        let image = serialize_container(Some(&marker), &events);
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
        let image = serialize_container(Some(&SelfMarker::Signed(att.clone())), &[g]);
        let got = parse_container(&image).unwrap();
        assert_eq!(got.self_marker, Some(SelfMarker::Signed(att)));
    }

    #[test]
    fn container_roundtrips_no_marker() {
        let k = sk();
        let events = vec![enroll(&k, "Self")];
        let image = serialize_container(None, &events);
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
        put_chunk(&mut image, &g);
        let got = parse_container(&image).unwrap();
        assert_eq!(got.self_marker, None, "legacy medium has no marker");
        assert_eq!(got.events, vec![g]);
    }

    #[test]
    fn parse_rejects_missing_magic_and_unknown_kind() {
        assert!(matches!(
            parse_container(b"not a medium"),
            Err(BackupError::Decode(_))
        ));
        // CAIRNB2 with an out-of-range marker kind.
        let mut bad = MEDIUM_MAGIC_V2.to_vec();
        bad.push(99);
        assert!(matches!(parse_container(&bad), Err(BackupError::Decode(_))));
    }
}
