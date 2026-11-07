//! The co-sign rules (sign-in slice 3c-3a): does a signed part still wait for a co-signature,
//! and whose vouch clears it. Pure, over a [`NoteRow`] that `supervision::annotate` has stamped.
//!
//! Why this exists: on paper a student's or a probationer's entry waits for a countersignature,
//! and lifting the probation later does not countersign yesterday's entries. So:
//!
//! * A co-sign is OWED ([`cosign_required`]) when the part is signed and its writer's signature
//!   ([`writer_signature`]) is a student's, or the supervision record said the writer needed
//!   co-signing when the part entered the log (`NoteRow::needs_cosigning`, judged at the PART's
//!   time, never now).
//! * It is CLEARED ([`clears_cosign`]) by a vouch under a standing key, of a known actor other
//!   than the writer's, by someone who supervised the writer when they vouched
//!   (`PartVouch::by_supervisor`, any role). Where the writer had NO supervisor at that time, the
//!   rule before this slice applies (the fallback): any non-student's `co-signed` vouch clears it.
//!
//! With every annotation at its default (no supervision rows at all) these rules are exactly
//! the pre-slice rule, a student's part waiting until a non-student co-signs it.
//! `view::part_byline` words [`awaits_cosign`] as " — not co-signed", and
//! `vouch::needs_sign_off` keeps "Sign off this note" offered while it is true.
use super::read::NoteRow;
use super::vouch::{counts_as_unsigned, part_is_signed, same_actor};
use super::vouch_read::{PartVouch, Signer};
use cairn_event::note_attestation::VouchRole;

/// The signature a co-sign countersigns: the part's own `signed_by` when it has one, else the
/// first vouch by the part's drafted writer (`drafted_actor`, by actor) under a key that still
/// stands — a part that went in unsigned and was signed later by its writer (design §3.2 Q4).
/// `None` when the writer never signed it (a colleague's sign-off is not the writer's signature).
/// Agrees with `supervision::writer_actor`, which annotation judged the record for.
pub fn writer_signature(p: &NoteRow) -> Option<&Signer> {
    p.signed_by.as_ref().or_else(|| {
        let drafted = p.drafted_actor.as_deref()?;
        p.vouches
            .iter()
            .map(|v| &v.by)
            .find(|s| !s.repudiated && same_actor(&s.actor, drafted))
    })
}

/// Part `p` owes a co-sign: it is signed (and does not count as unsigned for a repudiated key,
/// whose " — counts as unsigned" says more), and its writer signed it as a student, or under a
/// co-signing requirement as of the part's time.
pub fn cosign_required(p: &NoteRow) -> bool {
    part_is_signed(p)
        && !counts_as_unsigned(p)
        && writer_signature(p).is_some_and(|s| s.student || p.needs_cosigning)
}

/// Vouch `v` clears `p`'s co-sign. It must be under a standing key (a repudiated one counts
/// nothing, ADR-0093), of a known actor (an empty one might be anyone, the writer included) that
/// is not the writer's own; then either its signer supervised the writer at the vouch's time
/// (any role), or, where the writer had no supervisor then, it is a non-student's `co-signed`
/// (the floor never checks the student role, so a student's co-sign may be stored; it clears
/// nothing, durable rule 3c-2).
pub fn clears_cosign(p: &NoteRow, v: &PartVouch) -> bool {
    let writer = writer_signature(p).map_or("", |s| s.actor.as_str());
    let placeable = !v.by.repudiated && !v.by.actor.is_empty() && !same_actor(&v.by.actor, writer);
    let fallback = !v.writer_supervised && v.role == VouchRole::CoSigned && !v.by.student;
    placeable && (v.by_supervisor || fallback)
}

/// Part `p` still waits for its co-sign: one is owed and no vouch on it clears it.
pub fn awaits_cosign(p: &NoteRow) -> bool {
    cosign_required(p) && !p.vouches.iter().any(|v| clears_cosign(p, v))
}

#[cfg(test)]
#[path = "cosign_tests.rs"]
mod tests;
