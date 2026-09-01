//! Signature verification of an event set, reusing the existing self-described-signature
//! invariant (no DB, no external key needed — every signed event names its own verifying key).
//! This is the read side of the verify-before-write discipline: `serialize_and_verify_container`
//! is called BEFORE an image is written over a live medium, so a serialization/signing
//! regression can never overwrite a good medium with an unrestorable one.
//!
//! This module answers ONE question — "do these bytes verify" — a flat, whole-set check with
//! no notion of segments, chains, or planes. [`crate::chain`] answers the different question
//! "what can this whole MEDIUM be trusted for" (task 8 review, #500): see its module docs.

use crate::container::{parse_medium, serialize_container, serialize_v3};
use crate::error::BackupError;
use crate::marker::SelfMarker;
use crate::segment::Segment;
use cairn_event::verify_self_described;

/// What a verification pass found. `intact` events verified their signature; `first_bad` is the
/// index of the first that did NOT. A medium is sound iff every event is intact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyReport {
    pub total: usize,
    pub intact: usize,
    pub first_bad: Option<usize>,
}

impl VerifyReport {
    /// Every event verified — the backup is restorable as-is.
    pub fn all_intact(&self) -> bool {
        self.first_bad.is_none() && self.intact == self.total
    }
}

/// Verify ONE signed event the way restore will: its self-described Ed25519 key must sign the
/// COSE body, and the body's claimed signer must match that key. A flipped byte → `false`.
pub fn verify_event(signed: &[u8]) -> bool {
    verify_self_described(signed).is_ok()
}

/// Verify every event in a set. Deterministic; no DB, no external key.
pub fn verify_events(events: &[Vec<u8>]) -> VerifyReport {
    let mut intact = 0;
    let mut first_bad = None;
    for (i, e) in events.iter().enumerate() {
        if verify_event(e) {
            intact += 1;
        } else if first_bad.is_none() {
            first_bad = Some(i);
        }
    }
    VerifyReport {
        total: events.len(),
        intact,
        first_bad,
    }
}

/// Parse a medium image and verify every event in one step. A `Decode` error means the
/// container is structurally broken; an `Ok(report)` with `!all_intact()` means it parsed but
/// carries a tampered/corrupt event.
pub fn verify_medium_bytes(bytes: &[u8]) -> Result<VerifyReport, BackupError> {
    Ok(verify_events(&parse_medium(bytes)?))
}

/// Serialize a container and self-verify its event set in one step, returning the verified
/// bytes — or an error if the set fails its own signature check. Runs BEFORE the image is
/// written over the live medium (verify-before-write), so a serialization/signing regression
/// can never overwrite a good medium with an unrestorable one.
pub fn serialize_and_verify_container(
    marker: Option<&SelfMarker>,
    events: &[Vec<u8>],
) -> Result<Vec<u8>, BackupError> {
    let report = verify_events(events);
    if !report.all_intact() {
        return Err(BackupError::Encode(format!(
            "refusing to write a medium that fails its own self-verification \
             ({} of {} events intact, first bad at index {:?})",
            report.intact, report.total, report.first_bad
        )));
    }
    serialize_container(marker, events)
}

/// CAIRNB3's verify-before-write, mirroring [`serialize_and_verify_container`].
///
/// WHY THIS EXISTS (#500 slice 2a review): CAIRNB2 refused to write a set that failed its own
/// signature check, and CAIRNB3 — the revision that will actually carry clinical events — had
/// no equivalent. That gap is worse than it sounds, because a segment attestation is
/// computed OVER THE CONTENT ADDRESS of whatever bytes it is handed. Feed the writer corrupt
/// bytes (a torn page, a bad `bytea` read, a bug in the capture query) and it signs a
/// genuinely valid attestation over the corruption: `chain_report` then reports the segment
/// fully intact and signed, and only a separate `verify_records` pass would ever notice.
/// Refusing at the door means a medium can never be written carrying a record this node could
/// not itself verify.
pub fn serialize_and_verify_v3(segments: &[Segment]) -> Result<Vec<u8>, BackupError> {
    for seg in segments {
        refuse_unverifiable_records(seg)?;
    }
    serialize_v3(segments)
}

/// Append one segment to an existing CAIRNB3 image, refusing any record whose signature does
/// not verify. The verify-before-write half of [`crate::container::append_segment`].
///
/// Checks THIS SEGMENT's records only, which is what keeps the append O(new records) — the
/// complexity guarantee that makes an append-only medium worth having. It deliberately does
/// NOT re-verify the chain link: the caller already parsed the medium to learn
/// `prev_commitment` and the next index, and re-reading the whole image here would silently
/// turn every append into an O(medium size) operation.
pub fn verify_and_append_segment(medium: &mut Vec<u8>, seg: &Segment) -> Result<(), BackupError> {
    refuse_unverifiable_records(seg)?;
    crate::container::append_segment(medium, seg)
}

/// Shared guard for both CAIRNB3 write paths: every record must carry a signature this node
/// can verify right now. One definition, so the two doors can never drift apart.
fn refuse_unverifiable_records(seg: &Segment) -> Result<(), BackupError> {
    let report = verify_events(
        &seg.records
            .iter()
            .map(|r| r.signed_bytes.clone())
            .collect::<Vec<_>>(),
    );
    if let Some(bad) = report.first_bad {
        return Err(BackupError::Encode(format!(
            "refusing to write {:?} segment {}: its record {} fails signature verification \
             ({} of {} intact). A segment attestation commits to the CONTENT ADDRESS of \
             whatever it is given, so signing this would produce a genuinely valid \
             attestation over corrupt bytes — a segment that reports itself intact forever",
            seg.plane, seg.index, bad, report.intact, report.total
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::MEDIUM_MAGIC_V2;
    use crate::testkit::{enroll, sk};

    #[test]
    fn verify_pinpoints_a_tampered_event_through_the_container() {
        let k = sk();
        let events: Vec<Vec<u8>> = (0..3).map(|_| enroll(&k, "n")).collect();
        let mut image = serialize_container(None, &events).expect("fixture fits the cap");
        assert!(verify_medium_bytes(&image).unwrap().all_intact());
        // Corrupt a byte well inside the body region (after magic + marker-kind byte).
        let idx = MEDIUM_MAGIC_V2.len() + 24;
        image[idx] ^= 0xff;
        assert!(
            !verify_medium_bytes(&image).unwrap().all_intact(),
            "a bit-flip must fail verify"
        );
    }

    #[test]
    fn serialize_and_verify_refuses_a_tampered_set() {
        let k = sk();
        let mut events: Vec<Vec<u8>> = (0..3).map(|_| enroll(&k, "n")).collect();
        let mid = events[1].len() / 2;
        events[1][mid] ^= 0xff;
        let err = serialize_and_verify_container(None, &events)
            .expect_err("a set that fails its own verification must not be written");
        assert!(
            matches!(err, BackupError::Encode(_)),
            "refusing to WRITE is an Encode fault, not a property of a medium on disk: {err:?}"
        );
    }

    /// The HAPPY path of verify-before-write, which had no test at all.
    ///
    /// Without it, `serialize_and_verify_container` could return `Ok(Vec::new())` — or refuse
    /// everything — and the whole suite stayed green (mutation audit, #500 slice 2a review).
    /// This is the function whose output is written over the live medium, so "it produced the
    /// right bytes" is the assertion that matters most and was the one missing.
    #[test]
    fn serialize_and_verify_emits_exactly_what_serialize_would() {
        let k = sk();
        let events: Vec<Vec<u8>> = (0..3).map(|_| enroll(&k, "n")).collect();
        let marker = SelfMarker::Unsigned("abcd".into());
        let verified = serialize_and_verify_container(Some(&marker), &events)
            .expect("a clean set must be written");
        assert!(
            !verified.is_empty(),
            "verify-before-write must emit the medium, not an empty vec"
        );
        assert_eq!(
            verified,
            serialize_container(Some(&marker), &events).expect("fits the cap"),
            "the verified path must emit the SAME bytes as the plain serializer — it adds a \
             refusal, not a different format"
        );
        // And what it wrote parses back to exactly what went in.
        let back = crate::container::parse_container(&verified).expect("its own output parses");
        assert_eq!(back.events, events);
        assert_eq!(back.self_marker, Some(marker));
    }

    /// CAIRNB3's verify-before-write refuses a segment carrying a record this node cannot
    /// verify — the gap that let corrupt bytes be signed into a genuinely valid attestation.
    #[test]
    fn v3_verify_before_write_refuses_an_unverifiable_record() {
        use crate::segment::{Plane, Segment};
        let k = sk();
        let mut rec = crate::record::MediumRecord {
            signed_bytes: enroll(&k, "n"),
            attestation: None,
            attester_key: None,
            dek_wrapped: None,
            source_seq: 1,
        };
        let clean = Segment {
            plane: Plane::Node,
            index: 0,
            prev_commitment: String::new(),
            self_node_id_hex: "abcd".into(),
            attestation: None,
            records: vec![rec.clone()],
        };
        assert!(
            serialize_and_verify_v3(std::slice::from_ref(&clean)).is_ok(),
            "a segment whose records verify must be written"
        );

        let mid = rec.signed_bytes.len() / 2;
        rec.signed_bytes[mid] ^= 0xff;
        let dirty = Segment {
            records: vec![rec],
            ..clean.clone()
        };
        let err = serialize_and_verify_v3(std::slice::from_ref(&dirty))
            .expect_err("a corrupt record must never be signed into a segment");
        assert!(
            matches!(err, BackupError::Encode(_)),
            "refusing to write is an Encode fault: {err:?}"
        );

        // The append door applies the identical guard — one definition, two doors.
        let mut medium = serialize_and_verify_v3(std::slice::from_ref(&clean)).unwrap();
        let before = medium.len();
        assert!(verify_and_append_segment(&mut medium, &dirty).is_err());
        assert_eq!(
            medium.len(),
            before,
            "a refused append must leave the medium byte-identical"
        );
    }
}
