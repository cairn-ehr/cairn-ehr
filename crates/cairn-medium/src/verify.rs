//! Signature verification of an event set, reusing the existing self-described-signature
//! invariant (no DB, no external key needed — every signed event names its own verifying key).
//! This is the read side of the verify-before-write discipline: `serialize_and_verify_container`
//! is called BEFORE an image is written over a live medium, so a serialization/signing
//! regression can never overwrite a good medium with an unrestorable one.
//!
//! This module answers ONE question — "do these bytes verify" — a flat, whole-set check with
//! no notion of segments, chains, or planes. [`crate::chain`] answers the different question
//! "what can this whole MEDIUM be trusted for" (task 8 review, #500): see its module docs.

use crate::container::{parse_medium, serialize_container};
use crate::error::BackupError;
use crate::marker::SelfMarker;
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
        return Err(BackupError::Decode(format!(
            "refusing to write a medium that fails its own self-verification \
             ({} of {} events intact, first bad at index {:?})",
            report.intact, report.total, report.first_bad
        )));
    }
    Ok(serialize_container(marker, events))
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
        let mut image = serialize_container(None, &events);
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
        assert!(matches!(
            serialize_and_verify_container(None, &events),
            Err(BackupError::Decode(_))
        ));
    }
}
