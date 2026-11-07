//! "Co-sign as reviewed" (sign-in slice 3c-3a, design §3.3): a supervisor countersigns the parts
//! their supervisees signed, on a note that did NOT need a co-signature.
//!
//! On paper a supervisor reads a registrar's entry and initials it as reviewed, though nothing
//! obliged them to. Here that is one `sign-off` vouch per such part, `co-signed` where the floor
//! admits it (`vouch::planned_role`), so it reads as any co-sign does (" · co-signed by …").
//!
//! It is its own gesture, not "Sign off this note" under a second label: it never vouches
//! another writer's part, a part its writer never signed (even one a colleague signed off since),
//! a part the presser does not supervise, or a part that owes a co-sign (clearing a mandatory
//! co-sign is Sign off's job, `cosign::awaits_cosign`).
//!
//! WHO the presser is comes from the signing KEY (`vouch::vouch_as_shown` reads its actor), never
//! from the who's-here claim, which is not even an input here (ADR-0092 decision 16): a clinician
//! named in who's-here but signing with a colleague's key reviews as that colleague, or not at all.
use tokio_postgres::Client;
use uuid::Uuid;

use super::cosign::{cosign_required, writer_signature};
use super::read::NoteRow;
use super::vouch::{
    counts_as_unsigned, has_signed, part_is_signed, planned_role, same_actor, vouch_as_shown,
    SignOffInput, SignOffRefusal,
};
use crate::medication::AuthorParams;
use cairn_event::note_attestation::VouchRole;

/// Part `p` is one `presser` may co-sign as reviewed. All of:
/// * it is signed, does not count as unsigned (a repudiated key), and owes no co-sign;
/// * its WRITER signed it (`cosign::writer_signature`: its own signature, or the drafted
///   writer's later vouch — a colleague's sign-off of a part gone in unsigned is not the
///   writer's signature, so there is nothing of a supervisee's to review), that writer is not
///   the presser, and has the presser among their supervisors NOW;
/// * no supervisor of the writer has vouched it yet under a standing key (one review is enough,
///   whoever gave it), and the presser has not signed it in any way.
fn reviewable(p: &NoteRow, presser: &str) -> bool {
    let supervises = writer_signature(p).is_some_and(|w| {
        !same_actor(&w.actor, presser)
            && p.writer_supervisors.iter().any(|s| same_actor(s, presser))
    });
    let reviewed = p
        .vouches
        .iter()
        .any(|v| v.by_supervisor && !v.by.repudiated);
    part_is_signed(p)
        && !counts_as_unsigned(p)
        && !cosign_required(p)
        && supervises
        && !reviewed
        && !has_signed(p, presser)
}

/// What "Co-sign as reviewed" vouches for `presser`: each `reviewable` part, in the role the
/// floor admits (`vouch::planned_role`). Empty means there is nothing for them to co-sign. Pure.
pub fn review_vouches(parts: &[NoteRow], presser: &str) -> Vec<(Uuid, VouchRole)> {
    parts
        .iter()
        .filter(|p| reviewable(p, presser))
        .map(|p| (p.event_id, planned_role(p, presser)))
        .collect()
}

/// "Co-sign as reviewed" is offered to `actor` on this note.
pub fn can_review(parts: &[NoteRow], actor: &str) -> bool {
    !review_vouches(parts, actor).is_empty()
}

/// Co-sign the compound note `input.root` as reviewed, as `author`: [`review_vouches`]' parts,
/// one vouch each, in ONE transaction, only if the note holds exactly the parts shown
/// (`Changed` otherwise) — `vouch_as_shown`, the transaction Sign off uses. `NothingToSign` is an
/// empty plan: the key's person supervises no one whose part here awaits a review.
pub async fn review_note(
    client: &mut Client,
    input: &SignOffInput<'_>,
    author: &AuthorParams<'_>,
) -> anyhow::Result<Result<Vec<Uuid>, SignOffRefusal>> {
    vouch_as_shown(client, input, author, review_vouches).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::note::vouch::fixture::{part, repudiated, student, vouch, who};
    use cairn_event::note_attestation::VouchAct;

    /// Part `n` signed by `writer`, whose supervisors now are `sups`.
    fn supervised(n: u128, writer: &str, sups: &[&str]) -> NoteRow {
        let mut p = part(n, Some(who(writer, writer)));
        p.writer_supervisors = sups.iter().map(|s| s.to_string()).collect();
        p
    }

    #[test]
    fn review_vouches_only_the_pressers_supervisees_parts() {
        let mut unsigned = part(4, None);
        unsigned.drafted_actor = Some("bea".into());
        unsigned.writer_supervisors = vec!["sue".into()];
        let parts = vec![
            supervised(1, "bea", &["sue"]),
            supervised(2, "cal", &["zed"]), // another writer, not Sue's supervisee
            supervised(3, "sue", &[]),      // Sue's own part
            unsigned,                       // Bea's, never signed
            supervised(5, "dan", &[]),      // no supervisor at all
        ];
        assert_eq!(
            review_vouches(&parts, "sue"),
            vec![(Uuid::from_u128(1), VouchRole::CoSigned)]
        );
        assert!(can_review(&parts, "sue"));
        assert!(!can_review(&parts, "amy"), "Amy supervises no one here");
        assert!(!can_review(&parts, ""), "an unknown key supervises no one");
    }

    #[test]
    fn review_skips_a_part_that_needs_a_co_sign() {
        let mut required = supervised(1, "bea", &["sue"]);
        required.needs_cosigning = true;
        let mut sam = part(2, Some(student("sam", "Sam")));
        sam.writer_supervisors = vec!["sue".into()];
        let mut dead = part(3, Some(repudiated("bea", "Bea")));
        dead.writer_supervisors = vec!["sue".into()];
        assert!(review_vouches(&[required.clone(), sam, dead], "sue").is_empty());
        // The control: the same part, owing nothing, is offered.
        required.needs_cosigning = false;
        assert_eq!(review_vouches(&[required], "sue").len(), 1);
    }

    /// A part gone in unsigned on the quarantine timer, written by Bea (Sue's supervisee), that
    /// colleague Cal signed off: Bea never signed it, so it is not Sue's to review (design §3.3,
    /// "never ... an unsigned part"). Once Bea signs it herself, it is.
    #[test]
    fn a_colleagues_sign_off_of_a_timer_part_is_not_the_writers_signature() {
        let mut p = part(1, None);
        p.drafted_actor = Some("bea".into());
        p.writer_supervisors = vec!["sue".into()];
        p.vouches.push(vouch(
            10,
            VouchAct::SignOff,
            VouchRole::Attested,
            who("cal", "Cal"),
        ));
        assert!(review_vouches(&[p.clone()], "sue").is_empty());

        p.vouches.push(vouch(
            11,
            VouchAct::SignOff,
            VouchRole::Attested,
            who("bea", "Bea"),
        ));
        assert_eq!(review_vouches(&[p], "sue").len(), 1);
    }

    #[test]
    fn once_any_supervisor_reviewed_it_no_one_is_offered_it() {
        let mut p = supervised(1, "bea", &["sue", "zed"]);
        let mut by_zed = vouch(
            10,
            VouchAct::SignOff,
            VouchRole::CoSigned,
            who("zed", "Zed"),
        );
        by_zed.by_supervisor = true;
        p.vouches.push(by_zed);
        assert!(review_vouches(&[p.clone()], "sue").is_empty());

        // A review under a since-repudiated key reviewed nothing.
        p.vouches[0].by.repudiated = true;
        assert_eq!(review_vouches(&[p.clone()], "sue").len(), 1);

        // Sue vouched it before she was Bea's supervisor: she has signed it, so not again.
        let mut q = supervised(2, "bea", &["sue"]);
        q.vouches.push(vouch(
            11,
            VouchAct::SignOff,
            VouchRole::CoSigned,
            who("sue", "Sue"),
        ));
        assert!(review_vouches(&[q], "sue").is_empty());
    }
}
