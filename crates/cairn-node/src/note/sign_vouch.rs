//! Sign's second half for an addendum (sign-in slice 3c-2): the same press that signs the new
//! part also vouches for the signer's EARLIER unsigned parts of the note it adds to.
//!
//! On paper a nurse who left a line unsigned comes back, writes another line and signs, and the
//! one signature covers both. Here the earlier line went in unsigned (the quarantine timer
//! committed it, `{writer, drafted}`), so Sign files one `clinical.note-attestation.asserted`
//! (act `addendum`, role `attested`) per such part, inside Sign's own transaction: a refused vouch
//! rolls the whole sign back.
//!
//! Two gates, both required, because a vouch puts the signer's name under words:
//! * the window's strip LISTED the part (`also_vouches`) — never vouch what the signer was not
//!   shown;
//! * the part is the signer's own by KEY ([`addendum_covers`]: unsigned and drafted by the
//!   signer's actor) — never vouch someone else's, even if a stale strip listed it (Nurse B
//!   signing while who's-here still says Nurse A).
//!
//! The note is the one the draft's HEADER adds to, re-read here inside the transaction (ADR-0091
//! decision 7), never one the window names. A missing note, or a signer this node cannot place,
//! vouches nothing: an under-claim, never a refusal of a sign that is otherwise good.
use std::collections::HashSet;

use cairn_event::note_attestation::{VouchAct, VouchRole};
use tokio_postgres::Transaction;
use uuid::Uuid;

use super::compound::compounds;
use super::read::list_patient_notes;
use super::vouch::{addendum_covers, vouch_in_tx};
use crate::medication::AuthorParams;

/// The listed parts that are also covered, in the order listed, each once. Pure.
fn to_vouch(listed: &[Uuid], covered: &[Uuid]) -> Vec<Uuid> {
    let mut seen = HashSet::new();
    listed
        .iter()
        .copied()
        .filter(|id| covered.contains(id) && seen.insert(*id))
        .collect()
}

/// Vouch, as `author`, for each part in `listed` that is the author's own unsigned part of the
/// note rooted at `root` (the draft header's target; `None` = a new note, which covers nothing),
/// as chart `patient` reads it. Runs in Sign's transaction AFTER the new parts are submitted:
/// they are in the note now, but signed, so never covered. Returns the vouches' event ids, in
/// `listed` order. Each vouch is filed under its part's own chart (a linked chart's part too).
pub(super) async fn vouch_own_earlier_parts(
    tx: &Transaction<'_>,
    author: &AuthorParams<'_>,
    patient: Uuid,
    root: Option<Uuid>,
    listed: &[Uuid],
    node_origin: &str,
) -> anyhow::Result<Vec<Uuid>> {
    let Some(root) = root else { return Ok(vec![]) };
    if listed.is_empty() {
        return Ok(vec![]);
    }
    // NULL = a key this node cannot place; "" matches no one (`same_actor`), so nothing is vouched.
    let signer: Option<String> = tx
        .query_one("SELECT cairn_note_key_actor($1)", &[&author.human_kid])
        .await?
        .get(0);
    let notes = list_patient_notes(tx, patient).await?.notes;
    let Some(note) = compounds(notes).into_iter().find(|c| c.root() == root) else {
        return Ok(vec![]);
    };
    let covered = addendum_covers(note.parts(), signer.as_deref().unwrap_or(""));
    let mut ids = Vec::new();
    for part in to_vouch(listed, &covered) {
        // `covered` came from these very parts, so the part is always found.
        let chart = note
            .parts()
            .iter()
            .find(|p| p.event_id == part)
            .map(|p| p.chart)
            .expect("a covered part is one of the note's parts");
        let id = vouch_in_tx(
            tx,
            author,
            part,
            chart,
            VouchAct::Addendum,
            VouchRole::Attested,
            node_origin,
        )
        .await?;
        ids.push(id);
    }
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(n: &[u128]) -> Vec<Uuid> {
        n.iter().map(|&i| Uuid::from_u128(i)).collect()
    }

    #[test]
    fn only_listed_parts_that_are_covered_are_vouched_in_listed_order_once() {
        assert_eq!(
            to_vouch(&ids(&[3, 1, 9, 3]), &ids(&[1, 2, 3])),
            ids(&[3, 1])
        );
        assert!(to_vouch(&[], &ids(&[1])).is_empty(), "unlisted: never");
        assert!(
            to_vouch(&ids(&[1]), &[]).is_empty(),
            "not the signer's: never"
        );
    }
}
