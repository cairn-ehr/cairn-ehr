//! Who has signed what in a note, and "Sign off this note" (sign-in slice 3c-2).
//!
//! A part of a note is SIGNED when its own event carries a verified human signature
//! (`NoteRow::signed_by`) or when someone later vouched for it (`NoteRow::vouches`, a
//! `clinical.note-attestation.asserted` event — see `vouch_read`). The pure rules below answer
//! the questions the window asks of a compound note: does it still need a supervisor's
//! sign-off, who signed it off, and — when a clinician presses "Sign off this note" — which
//! parts must they vouch for, and how.
//!
//! People are compared by ACTOR, never by key: a re-keyed member is still the same person. A key
//! this node does not know has an EMPTY actor, which matches no one, itself included
//! ([`same_actor`]) — an unknown key can never be taken for the presser, a writer, or the
//! person who signed a note off.
//!
//! [`sign_off_note`] is the gesture's node side: it re-reads the note in its own transaction and
//! refuses (`Changed`) unless the note holds exactly the parts the window showed, so a part that
//! landed or vanished after the clinician read the note is never signed unseen.
use std::collections::HashSet;

use cairn_event::note_attestation::{
    note_attestation_payload, render_note_attestation_twin, vouch_contributors, NoteVouch,
    VouchAct, VouchRole, NOTE_ATTESTATION_EVENT_TYPE, NOTE_ATTESTATION_SCHEMA_VERSION,
};
use cairn_event::{event_address, sign_attestation, EventBody, Hlc};
use tokio_postgres::Client;
use uuid::Uuid;

use super::compound::compounds;
use super::read::{list_patient_notes, NoteRow};
use super::vouch_read::Signer;
use crate::medication::sealed_submit::{prepare_sealed_as, Prepared};
use crate::medication::AuthorParams;

/// Two actor ids name the same person. An empty id (a key this node does not know) never does.
pub fn same_actor(a: &str, b: &str) -> bool {
    !a.is_empty() && a == b
}

/// Everyone who has signed part `p`: its own signer first, then each vouch's signer in order.
pub fn signed_for(p: &NoteRow) -> Vec<&Signer> {
    p.signed_by
        .iter()
        .chain(p.vouches.iter().map(|v| &v.by))
        .collect()
}

/// Whether `actor` is among the people who signed `p`.
fn has_signed(p: &NoteRow, actor: &str) -> bool {
    signed_for(p).iter().any(|s| same_actor(&s.actor, actor))
}

/// A part is signed when its own event is, or anyone vouched for it since.
pub fn part_is_signed(p: &NoteRow) -> bool {
    p.signed_by.is_some() || !p.vouches.is_empty()
}

/// Who signed the whole note off: the first non-student, in the root's [`signed_for`] order, who
/// has signed EVERY part. `None` when no one has (the note still needs a sign-off).
pub fn signed_off_by(parts: &[NoteRow]) -> Option<&Signer> {
    let root = parts.first()?;
    signed_for(root)
        .into_iter()
        .find(|s| !s.student && parts.iter().all(|p| has_signed(p, &s.actor)))
}

/// The note still needs someone to sign it off as a whole.
pub fn needs_sign_off(parts: &[NoteRow]) -> bool {
    signed_off_by(parts).is_none()
}

/// What "Sign off this note" vouches, part by part, for the presser `presser_actor`: nothing for
/// a part they have already signed; `CoSigned` for a part someone else signed; `Attested` for a
/// part nobody signed. Empty means there is nothing left for them to sign.
///
/// A part signed by a key this node cannot place (an empty actor) is `Attested` too: db/071's
/// floor co-signs only a part whose signer it knows as a human's, so a co-sign would be refused
/// and take the whole sign-off with it. An attested sign-off on a signed part is admitted and
/// reads honestly, "signed off by V".
pub fn sign_off_vouches(parts: &[NoteRow], presser_actor: &str) -> Vec<(Uuid, VouchRole)> {
    parts
        .iter()
        .filter(|p| !has_signed(p, presser_actor))
        .map(|p| {
            let role = match &p.signed_by {
                Some(s) if !s.actor.is_empty() => VouchRole::CoSigned,
                _ => VouchRole::Attested,
            };
            (p.event_id, role)
        })
        .collect()
}

/// The parts `actor` drafted that nobody has signed yet: what their next addendum's signature
/// also covers (Task 4 of the slice).
pub fn addendum_covers(parts: &[NoteRow], actor: &str) -> Vec<Uuid> {
    parts
        .iter()
        .filter(|p| !part_is_signed(p))
        .filter(|p| {
            p.drafted_actor
                .as_deref()
                .is_some_and(|d| same_actor(d, actor))
        })
        .map(|p| p.event_id)
        .collect()
}

/// The CLEAR vouch body for `v`, filed on `chart` (the target's own chart), signed and vouched by
/// the one human `kid`. `t_effective` is none, as the medication attestation's. Pure.
fn vouch_body(event_id: Uuid, chart: Uuid, hlc: Hlc, v: &NoteVouch, kid: &str) -> EventBody {
    EventBody {
        event_id: event_id.to_string(),
        patient_id: chart.to_string(),
        event_type: NOTE_ATTESTATION_EVENT_TYPE.into(),
        schema_version: NOTE_ATTESTATION_SCHEMA_VERSION.into(),
        hlc,
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: vouch_contributors(kid, v.role),
        payload: note_attestation_payload(v),
        attachments: vec![],
        plaintext_twin: Some(render_note_attestation_twin(v)),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    }
}

/// Vouch for note part `part` (on chart `chart`) as `author`, INSIDE the caller's transaction, so
/// a refused vouch rolls the whole gesture back. The vouch names the part's current
/// `content_address`, read here. It is sealed through `prepare_sealed_as` (the safety rung,
/// seal-then-sign and the unwrap-key check stay one path), and its human token binds `role` to
/// the sealed bytes; the strict door (db/071's floor) then checks it against the part. One HLC
/// tick per vouch. Returns the vouch's event id.
pub(crate) async fn vouch_in_tx(
    tx: &tokio_postgres::Transaction<'_>,
    author: &AuthorParams<'_>,
    part: Uuid,
    chart: Uuid,
    act: VouchAct,
    role: VouchRole,
    node_origin: &str,
) -> anyhow::Result<Uuid> {
    let address: Vec<u8> = tx
        .query_opt(
            "SELECT content_address FROM event_log WHERE event_id = $1::text::uuid",
            &[&part.to_string()],
        )
        .await?
        .ok_or_else(|| anyhow::anyhow!("note part {part} is not on this node; nothing to vouch"))?
        .get(0);
    let target = part.to_string();
    let target_address_hex = hex::encode(address);
    let v = NoteVouch {
        target: &target,
        target_address_hex: &target_address_hex,
        act,
        role,
    };
    let event_id = Uuid::now_v7();
    let hlc = crate::db::next_hlc(tx, node_origin).await?;
    let body = vouch_body(event_id, chart, hlc, &v, author.human_kid);
    let Prepared {
        signed_bytes, dek, ..
    } = prepare_sealed_as(tx, body, author.human_sk).await?;
    let token = sign_attestation(
        &event_address(&signed_bytes),
        author.human_kid,
        role.as_wire(),
        author.human_sk,
    )?;
    let attester_vk = author.human_sk.verifying_key().to_bytes().to_vec();
    tx.execute(
        "SELECT submit_event($1, $2, $3, $4)",
        &[
            &signed_bytes,
            &token,
            &attester_vk,
            &dek.as_bytes().as_slice(),
        ],
    )
    .await?;
    Ok(event_id)
}

/// What the window sends with "Sign off this note".
pub struct SignOffInput<'a> {
    /// The chart on screen.
    pub patient: Uuid,
    /// The compound's root, as shown.
    pub root: Uuid,
    /// Every part id the window showed for that compound.
    pub shown_parts: &'a [Uuid],
    /// This node's id, the HLC's origin.
    pub node_origin: &'a str,
}

/// Why a sign-off wrote nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignOffRefusal {
    /// The note now holds other parts than the window showed (or is gone).
    Changed,
    /// The presser has already signed every part.
    NothingToSign,
}

/// The compound holds exactly the parts the window showed: the same ids, and as many. Pure.
fn shows_exactly(parts: &[NoteRow], shown: &[Uuid]) -> bool {
    let held: HashSet<Uuid> = parts.iter().map(|p| p.event_id).collect();
    let sent: HashSet<Uuid> = shown.iter().copied().collect();
    shown.len() == parts.len() && held == sent
}

/// Sign off the compound note `input.root` as `author`: one `sign-off` vouch per part they have
/// not signed ([`sign_off_vouches`]), each filed under its part's own chart, in ONE transaction.
///
/// The note is re-read here, inside that transaction, exactly as the window reads the opened
/// chart (`list_patient_notes`: its linked charts, the doubted-link rule), so what is signed is
/// what the window could show. If that compound's parts are not exactly `input.shown_parts`,
/// nothing is written (`Changed`). `Ok(Ok(ids))` names the vouches written; `Err` is a database
/// failure or a floor refusal, which wrote nothing — unless the commit's answer was lost, which
/// `commit_failure` marks.
pub async fn sign_off_note(
    client: &mut Client,
    input: &SignOffInput<'_>,
    author: &AuthorParams<'_>,
) -> anyhow::Result<Result<Vec<Uuid>, SignOffRefusal>> {
    // Every early return drops `tx` uncommitted, which rolls it back.
    let tx = client.transaction().await?;
    let presser: Option<String> = tx
        .query_one("SELECT cairn_note_key_actor($1)", &[&author.human_kid])
        .await?
        .get(0);
    let presser = presser.ok_or_else(|| {
        anyhow::anyhow!(
            "the signing key {} belongs to no enrolled person on this node; nothing was signed",
            author.human_kid
        )
    })?;
    let notes = list_patient_notes(&tx, input.patient).await?.notes;
    let Some(note) = compounds(notes)
        .into_iter()
        .find(|c| c.root() == input.root)
    else {
        return Ok(Err(SignOffRefusal::Changed));
    };
    if !shows_exactly(note.parts(), input.shown_parts) {
        return Ok(Err(SignOffRefusal::Changed));
    }
    let plan = sign_off_vouches(note.parts(), &presser);
    if plan.is_empty() {
        return Ok(Err(SignOffRefusal::NothingToSign));
    }
    let mut ids = Vec::with_capacity(plan.len());
    for (part, role) in plan {
        // `plan` came from these very parts, so the part is always found.
        let chart = note
            .parts()
            .iter()
            .find(|p| p.event_id == part)
            .map(|p| p.chart)
            .expect("a planned part is one of the note's parts");
        let id = vouch_in_tx(
            &tx,
            author,
            part,
            chart,
            VouchAct::SignOff,
            role,
            input.node_origin,
        )
        .await?;
        ids.push(id);
    }
    tx.commit()
        .await
        .map_err(|e| crate::db_diagnosis::commit_failure(ids[0], e.into()))?;
    Ok(Ok(ids))
}

/// Builders for the note module's pure tests: a part, a signer, a vouch.
#[cfg(test)]
pub(crate) mod fixture {
    use super::*;
    use crate::note::view::{NoteGrade, PartWriter};
    use crate::note::vouch_read::PartVouch;
    use chrono::{TimeZone, Utc};

    /// A known, confirmed, non-student signer with actor `actor` and name `name`.
    pub fn who(actor: &str, name: &str) -> Signer {
        Signer {
            actor: actor.into(),
            name: name.into(),
            student: false,
            unconfirmed: false,
        }
    }

    /// A student signer.
    pub fn student(actor: &str, name: &str) -> Signer {
        Signer {
            student: true,
            ..who(actor, name)
        }
    }

    /// Vouch `n` by `by`.
    pub fn vouch(n: u128, act: VouchAct, role: VouchRole, by: Signer) -> PartVouch {
        PartVouch {
            event_id: Uuid::from_u128(n),
            act,
            role,
            by,
        }
    }

    /// Part `n`: signed by `signer` (its grade `Signed`) or, for `None`, gone in unsigned.
    pub fn part(n: u128, signer: Option<Signer>) -> NoteRow {
        let t = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        NoteRow {
            event_id: Uuid::from_u128(n),
            chart: Uuid::from_u128(900),
            text: format!("t{n}"),
            t_effective: t,
            effective_basis: "stated".into(),
            recording_started: t,
            grade: match &signer {
                Some(s) => NoteGrade::Signed { by: s.name.clone() },
                None => NoteGrade::WentInUnsigned,
            },
            addendum_to: None,
            writer: PartWriter::TheSigner,
            signed_by: signer,
            drafted_actor: None,
            vouches: vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::*;
    use super::*;

    fn ids(n: &[u128]) -> Vec<Uuid> {
        n.iter().map(|&n| Uuid::from_u128(n)).collect()
    }

    #[test]
    fn a_note_signed_wholly_by_its_author_needs_no_sign_off() {
        let ada = who("a1", "Dr Ada");
        let parts = vec![part(1, Some(ada.clone())), part(2, Some(ada.clone()))];
        assert_eq!(signed_off_by(&parts), Some(&ada));
        assert!(!needs_sign_off(&parts));
        assert!(!needs_sign_off(&parts[..1]), "a one-part note too");
    }

    #[test]
    fn a_students_note_needs_sign_off_until_co_signed() {
        let mut parts = vec![part(1, Some(student("s1", "Sam")))];
        assert!(needs_sign_off(&parts), "a student never signs a note off");
        parts[0].vouches.push(vouch(
            10,
            VouchAct::SignOff,
            VouchRole::CoSigned,
            who("z1", "Dr Zed"),
        ));
        assert_eq!(
            signed_off_by(&parts).map(|s| s.name.as_str()),
            Some("Dr Zed")
        );
        assert!(!needs_sign_off(&parts));
    }

    #[test]
    fn two_clinicians_parts_need_sign_off() {
        let parts = vec![
            part(1, Some(who("a1", "Dr Ada"))),
            part(2, Some(who("b1", "Nurse Bo"))),
        ];
        assert!(needs_sign_off(&parts));
        assert!(needs_sign_off(&[part(1, None)]), "an unsigned part too");
    }

    /// An unknown key (empty actor) signs nothing off and matches no presser.
    #[test]
    fn a_key_this_node_does_not_know_matches_no_one() {
        let parts = vec![part(1, Some(who("", "a key this node does not know")))];
        assert!(needs_sign_off(&parts));
        assert_eq!(
            sign_off_vouches(&parts, "").len(),
            1,
            "not even an empty presser"
        );
    }

    /// The floor co-signs only a part a key it can place as a human's signed, so a part signed by
    /// a key this node cannot place is ATTESTED ("signed off by"), never co-signed.
    #[test]
    fn a_part_signed_by_a_key_this_node_cannot_place_is_attested_not_co_signed() {
        let parts = vec![
            part(1, Some(who("", "a key this node does not know"))),
            part(2, Some(student("s1", "Sam"))),
        ];
        assert_eq!(
            sign_off_vouches(&parts, "z1"),
            vec![
                (Uuid::from_u128(1), VouchRole::Attested),
                (Uuid::from_u128(2), VouchRole::CoSigned),
            ]
        );
    }

    #[test]
    fn sign_off_co_signs_others_signed_parts_attests_unsigned_ones_and_skips_ones_own() {
        let parts = vec![
            part(1, Some(student("s1", "Sam"))),
            part(2, None),
            part(3, Some(who("z1", "Dr Zed"))),
        ];
        assert_eq!(
            sign_off_vouches(&parts, "z1"),
            vec![
                (Uuid::from_u128(1), VouchRole::CoSigned),
                (Uuid::from_u128(2), VouchRole::Attested),
            ]
        );
        // A part the presser already vouched for is theirs too.
        let mut vouched = parts.clone();
        vouched[1].vouches.push(vouch(
            9,
            VouchAct::SignOff,
            VouchRole::Attested,
            who("z1", "Dr Zed"),
        ));
        assert_eq!(
            sign_off_vouches(&vouched, "z1"),
            vec![(Uuid::from_u128(1), VouchRole::CoSigned)]
        );
    }

    #[test]
    fn addendum_covers_only_the_actors_unsigned_parts() {
        let mut mine_unsigned = part(1, None);
        mine_unsigned.drafted_actor = Some("n1".into());
        let mut theirs_unsigned = part(2, None);
        theirs_unsigned.drafted_actor = Some("n2".into());
        let mut mine_vouched = part(3, None);
        mine_vouched.drafted_actor = Some("n1".into());
        mine_vouched.vouches.push(vouch(
            9,
            VouchAct::Addendum,
            VouchRole::Attested,
            who("n1", "Nurse N"),
        ));
        let mut mine_signed = part(4, Some(who("n1", "Nurse N")));
        mine_signed.drafted_actor = Some("n1".into());
        let parts = vec![
            mine_unsigned,
            theirs_unsigned,
            mine_vouched,
            mine_signed,
            part(5, None),
        ];
        assert_eq!(addendum_covers(&parts, "n1"), ids(&[1]));
        assert!(
            addendum_covers(&parts, "").is_empty(),
            "an unknown actor covers nothing"
        );
    }

    #[test]
    fn a_later_addendum_by_another_brings_the_offer_back() {
        let ada = who("a1", "Dr Ada");
        let mut parts = vec![part(1, Some(ada.clone()))];
        assert!(!needs_sign_off(&parts));
        parts.push(part(2, Some(who("b1", "Nurse Bo"))));
        assert!(needs_sign_off(&parts));
        assert_eq!(
            sign_off_vouches(&parts, "a1"),
            vec![(Uuid::from_u128(2), VouchRole::CoSigned)]
        );
    }

    #[test]
    fn the_shown_parts_must_be_exactly_the_notes() {
        let parts = vec![part(1, None), part(2, None)];
        assert!(shows_exactly(&parts, &ids(&[2, 1])));
        assert!(!shows_exactly(&parts, &ids(&[1])), "a part not shown");
        assert!(!shows_exactly(&parts, &ids(&[1, 2, 3])), "a part gone");
        assert!(
            !shows_exactly(&parts, &ids(&[1, 1, 2])),
            "the same count, too"
        );
        assert!(!shows_exactly(&parts, &ids(&[1, 1])), "and the same set");
    }
}
