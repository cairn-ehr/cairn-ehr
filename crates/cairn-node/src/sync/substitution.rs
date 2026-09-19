//! #619 / ADR-0073 — is a refused node event a SUBSTITUTION?
//!
//! # Why the pull loop has to ask
//!
//! The node-plane pull loop (`super::pull_into`) routes a refusal by SQLSTATE. Every floor refusal
//! is a bare `RAISE EXCEPTION` — P0001, which db/001's header makes a CONTRACT — and a P0001 on a
//! verifiable event is skipped-and-advanced, because on the node plane it is almost always
//! SCOPING: an event authored by a node this one does not peer with, which heals on a later full
//! sweep once trust or code arrives. (#268's own comment explains why penning that steady-state
//! traffic would flood the pen.)
//!
//! A substitution breaks that premise. It is a second, DIFFERENT event under an `event_id` this node
//! already holds, and it can never apply here — the id is taken — so "it heals on a later sweep" is
//! false for it. It is also evidence: a peer served two different signed events under one id, which
//! an honest, bug-free peer never does. So it is PENNED — durable, loud until a human acks it.
//!
//! # Why by STATE, and not by SQLSTATE or message text
//!
//! The refusal cannot carry its own code: db/001 forbids `USING ERRCODE`, because both pull loops
//! route on P0001, and a distinct code would turn `cairn-sync`'s clinical pen into a freeze.
//! Matching the door's sentence would make English prose part of the protocol. What IS unambiguous
//! is the table: `node_event` is append-only, so a row holding this id under a different content
//! address is true now and stays true. The question is asked of the table, after the refusal.
//!
//! That also makes the answer independent of WHICH check refused. A rival from an untrusted author
//! is refused by the trust check before the door ever reaches its substitution guard — and it is
//! still a rival under a held id, still never applies, and is still penned.

use tokio_postgres::Client;

use crate::db_diagnosis::LocalDbFault;

/// The reason to pen `offered` as a substitution, or `None` when it is not one. **Pure.**
///
/// * `held` — the content address `node_event` already holds under `event_id`, if any.
/// * `offered` — the content address of the bytes a peer just served (`event_address`).
///
/// `None` when nothing is held — a fresh id is not a substitution, whatever refused it, and the
/// ordinary skip-and-advance applies — or when `held` EQUALS `offered`: the same event again, an
/// idempotent re-offer, which is set-union working. `Some` only for a different event under a
/// held id; the reason starts `substitution:` so a `cairn-node quarantine` reader can tell it from
/// an unverifiable row, and names both addresses so both events can be found.
pub fn substitution_reason(event_id: &str, held: Option<&[u8]>, offered: &[u8]) -> Option<String> {
    let held = held?;
    if held == offered {
        return None;
    }
    Some(format!(
        "substitution: this node already holds event_id {event_id} with different content \
         (held {}, offered {}) — a peer served a second, different event under an id already \
         taken, so it can never apply here. Find out why that peer did, then ack this row",
        hex::encode(held),
        hex::encode(offered)
    ))
}

/// What `node_event` holds under `event_id`: its content address, or `None` when nothing is.
///
/// An `event_id` that is not a UUID cannot be held (the column is `uuid`), so it answers `None`
/// WITHOUT a query. Parsing here rather than casting in SQL is deliberate: a cast would turn a
/// malformed id into a database error, which the caller must treat as a FREEZE — and a refused
/// event with a malformed id (an oversized one, say, refused before the door parsed it) would
/// then wedge the cursor forever.
pub async fn held_content_address(db: &Client, event_id: &str) -> anyhow::Result<Option<Vec<u8>>> {
    let Ok(id) = uuid::Uuid::parse_str(event_id) else {
        return Ok(None);
    };
    let row = db
        .query_opt(
            "SELECT content_address FROM node_event WHERE node_event_id = $1::text::uuid",
            &[&id.to_string()],
        )
        .await
        .map_err(|e| {
            LocalDbFault::new(
                "reading the content address this node holds under an event_id",
                e,
            )
        })?;
    Ok(row.map(|r| r.get(0)))
}

#[cfg(test)]
mod tests {
    use super::substitution_reason;

    /// A content address, derived at runtime (house rule 6a) and discriminated by a `lineage`,
    /// never a seed/salt/nonce (6b): nothing here is cryptographic.
    fn address(lineage: u8) -> Vec<u8> {
        let mut a = vec![0x12, 0x20];
        a.extend((0..32u8).map(|i| i.wrapping_mul(7).wrapping_add(lineage)));
        a
    }

    #[test]
    fn nothing_held_is_not_a_substitution() {
        assert_eq!(substitution_reason("id", None, &address(1)), None);
    }

    #[test]
    fn the_same_event_again_is_not_a_substitution() {
        let a = address(1);
        assert_eq!(
            substitution_reason("id", Some(a.as_slice()), &a),
            None,
            "an idempotent re-offer is set-union working, never a refusal of this kind"
        );
    }

    #[test]
    fn a_different_event_under_a_held_id_is_one_and_the_reason_names_both() {
        let (held, offered) = (address(1), address(2));
        let reason = substitution_reason("0199aa-contested", Some(held.as_slice()), &offered)
            .expect("a different address under a held id IS a substitution");
        assert!(
            reason.starts_with("substitution:"),
            "the pen row's reason must say what KIND of refusal it is: {reason}"
        );
        assert!(
            reason.contains("0199aa-contested"),
            "names the id: {reason}"
        );
        assert!(
            reason.contains(&hex::encode(&held)) && reason.contains(&hex::encode(&offered)),
            "names both addresses, so an operator can find both events: {reason}"
        );
    }
}
