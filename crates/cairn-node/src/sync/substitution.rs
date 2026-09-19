//! #619 / ADR-0073 — is a refused node event a SUBSTITUTION?
//!
//! # Why the pull loop has to ask
//!
//! The node-plane pull loop (`super::pull_into`) routes a refusal by SQLSTATE. Every DELIBERATE
//! floor refusal is a bare `RAISE EXCEPTION` — P0001, which is a CONTRACT: db/001 states it for this
//! loop in the comment above `cairn_decode_hex_or_raise` (#228), and `cairn-sync`'s
//! `refusal_is_deliberate` has relied on it since #267. (Some refusals are not deliberate in this
//! sense — a `::uuid` cast raising 22P02, a CHECK raising 23514 — and freeze the cursor instead;
//! #621 and ADR-0072's erratum E1.) A P0001 on a verifiable event is skipped-and-advanced, because
//! on the node plane it is almost always SCOPING: an event authored by a node this one does not
//! peer with, which heals on a later full sweep once trust or code arrives. (Why penning that
//! steady-state traffic would flood the pen is recorded in `docs/spec/sync.md` §6.3's #268 note,
//! and weighed again in ADR-0073's alternatives.)
//!
//! A substitution breaks that premise. It is a second, DIFFERENT event under an `event_id` this node
//! already holds, and it can never apply here — the id is taken — so "it heals on a later sweep" is
//! false for it. It is also evidence that two different signed events exist under one id: some
//! signer minted an id already in use, or a relay re-wrapped a signed event — the COSE unprotected
//! header lies outside the signature (#620) — by bug or on purpose. So it is PENNED — durable, loud
//! until a human acks it.
//!
//! # Why by STATE, and not by SQLSTATE or message text
//!
//! The refusal cannot carry its own code. Both pull loops route on P0001: for this loop, db/001's
//! comment above `cairn_decode_hex_or_raise` (#228) calls it a contract and forbids `USING ERRCODE`
//! on that helper's refusals, because any other code freezes the cursor; for the clinical loop,
//! `cairn-sync`'s `refusal_is_deliberate` pens a verifiable event's refusal only when it is P0001,
//! since #267. And `cairn_refuse_substitution` is shared with the two clinical doors, so a distinct
//! code there would turn `cairn-sync`'s clinical pen into a freeze.
//!
//! Matching the door's sentence would make English prose part of the protocol. What IS unambiguous
//! is the table: `node_event` is append-only, so a row holding this id under a different content
//! address is true now and stays true. The question is asked of the table, after the refusal.
//!
//! That also makes the answer independent of WHICH check raised the P0001. A rival from an
//! untrusted author is refused by the trust check before the door ever reaches its substitution
//! guard — and it is still a rival under a held id, still never applies, and is still penned. (A
//! refusal with any OTHER SQLSTATE never reaches this question: the loop freezes on it first.)

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

/// Read `event_id` EXACTLY as Postgres's `::uuid` cast reads it, or `None` where that cast
/// would raise. **Pure.**
///
/// # Why not `uuid::Uuid::parse_str`
///
/// The doors take the id with `(b ->> 'event_id')::uuid`, and the signed body's `event_id` is a
/// free string that nothing forces into one spelling. Postgres accepts more spellings than the
/// `uuid` crate: a hyphen after ANY group of four hex digits, braced or not
/// (`a0ee-bc99-9c0b-4ef8-bb6d-6bb9-bd38-0a11`). With the crate's parser, a rival spelled that
/// way was refused by the door — the door read it as the held id — yet looked "not held" here,
/// so the pull loop skipped it as routine scoping and the pen never saw it. Whoever minted the
/// rival could switch the pen off by choosing a spelling (PR #623 review, finding 1). The only
/// safe answer is the door's own grammar, so this is a line-for-line mirror of Postgres's
/// `string_to_uuid` (src/backend/utils/adt/uuid.c):
///
/// 1. an optional opening `{`, which then requires a closing `}` at the very end;
/// 2. exactly 32 hex digits, either case, read as 16 bytes;
/// 3. after every second byte (every four digits) except the last, at most ONE optional `-`;
/// 4. nothing else — no whitespace, no `urn:uuid:` prefix, no trailing text.
///
/// `tests/node_event_id_spellings.rs` asks a live server the same question over the same
/// spellings, so if Postgres's grammar ever changes, that test fails rather than the pen going
/// quietly blind again.
pub fn uuid_as_postgres_reads_it(event_id: &str) -> Option<uuid::Uuid> {
    let mut rest = event_id.as_bytes();
    let braced = rest.first() == Some(&b'{');
    if braced {
        rest = &rest[1..];
    }
    let mut bytes = [0u8; 16];
    for (i, byte) in bytes.iter_mut().enumerate() {
        // Two hex digits make one byte. `get(..2)` is `None` when the input runs out early.
        let pair = std::str::from_utf8(rest.get(..2)?).ok()?;
        if !pair.bytes().all(|c| c.is_ascii_hexdigit()) {
            return None; // `from_str_radix` alone would also accept a leading `+`
        }
        *byte = u8::from_str_radix(pair, 16).ok()?;
        rest = &rest[2..];
        // Step 3: one optional hyphen after bytes 1, 3, 5, … 13 — never after the last (15).
        if i % 2 == 1 && i < 15 && rest.first() == Some(&b'-') {
            rest = &rest[1..];
        }
    }
    if braced {
        rest = rest.strip_prefix(b"}")?;
    }
    rest.is_empty().then(|| uuid::Uuid::from_bytes(bytes))
}

/// What `node_event` holds under `event_id`: its content address, or `None` when nothing is.
///
/// The id is read with [`uuid_as_postgres_reads_it`] — the door's grammar, not the `uuid`
/// crate's, for the reason given there. An id Postgres cannot read cannot be held (the column is
/// `uuid`), so it answers `None` WITHOUT a query. Parsing here rather than casting in SQL is
/// deliberate: a cast would turn a malformed id into a database error, which the caller must
/// treat as a FREEZE — and a refused event with a malformed id (an oversized one, say, refused
/// before the door parsed it) would then wedge the cursor forever. A guarded SQL cast
/// (`CASE WHEN pg_input_is_valid(…) THEN $1::uuid END`) would not avoid that either: planning a
/// query for its actual parameter value may constant-fold the cast and raise anyway.
pub async fn held_content_address(db: &Client, event_id: &str) -> anyhow::Result<Option<Vec<u8>>> {
    let Some(id) = uuid_as_postgres_reads_it(event_id) else {
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
    use super::{substitution_reason, uuid_as_postgres_reads_it};

    /// The one UUID every spelling below names, in its canonical form.
    const CANONICAL: &str = "a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11";

    /// Every spelling Postgres 18's `::uuid` ACCEPTS reads as the same UUID here. Each row was
    /// taken from a live `SELECT '<spelling>'::uuid`; `tests/node_event_id_spellings.rs` re-asks
    /// the live server the same question, so a drift between the two parsers fails CI.
    #[test]
    fn every_spelling_postgres_accepts_reads_as_the_same_uuid() {
        for spelling in [
            "a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11",
            "A0EEBC99-9C0B-4EF8-BB6D-6BB9BD380A11",
            "a0eebc999c0b4ef8bb6d6bb9bd380a11",
            "{a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11}",
            // The two the `uuid` crate rejects — the spellings behind the review finding: a
            // hyphen after ANY group of four hex digits, braced or not.
            "a0ee-bc99-9c0b-4ef8-bb6d-6bb9-bd38-0a11",
            "{a0eebc99-9c0b4ef8-bb6d6bb9-bd380a11}",
        ] {
            assert_eq!(
                uuid_as_postgres_reads_it(spelling).map(|u| u.to_string()),
                Some(CANONICAL.to_string()),
                "Postgres reads {spelling:?} as {CANONICAL}, so the lookup must too"
            );
        }
    }

    /// Every spelling Postgres 18's `::uuid` REJECTS is `None` here: the door raised 22P02 on it
    /// (a freeze, before this lookup is ever reached), so it names no row the door could hold.
    #[test]
    fn every_spelling_postgres_rejects_reads_as_none() {
        for spelling in [
            "a0eebc999c0b4ef8bb6d6bb9bd380a11-",
            "-a0eebc999c0b4ef8bb6d6bb9bd380a11",
            "a0eebc99--9c0b4ef8bb6d6bb9bd380a11",
            "a0e-ebc999c0b4ef8bb6d6bb9bd380a11",
            " a0eebc999c0b4ef8bb6d6bb9bd380a11",
            "a0eebc999c0b4ef8bb6d6bb9bd380a11 ",
            "{a0eebc999c0b4ef8bb6d6bb9bd380a11",
            "a0eebc999c0b4ef8bb6d6bb9bd380a11}",
            "urn:uuid:a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11",
            "a0eebc999c0b4ef8bb6d6bb9bd380a1-1",
            "a0eebc999c0b4ef8bb6d6bb9bd380a",
            "{-a0eebc999c0b4ef8bb6d6bb9bd380a11}",
            "",
            "not-a-uuid",
        ] {
            assert_eq!(
                uuid_as_postgres_reads_it(spelling),
                None,
                "Postgres rejects {spelling:?}, so it names no held row"
            );
        }
    }

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
