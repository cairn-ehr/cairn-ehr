//! Reading a note part's signer and its later vouches (sign-in slice 3c-2).
//!
//! A vouch (`clinical.note-attestation.asserted`, db/071) is a human saying, later and in a
//! separate signed event, "I stand behind that note part". The chart must never claim more about
//! who signed a part than the record proves, so only a vouch that passes BOTH checks is read:
//!
//! * its token is verified — `cairn_note_vouched_attester(vouch)` returns the vouch's own signer
//!   key (db/020 can store a peer's token unverified; such a vouch vouches for nothing), and
//! * it names the part's CURRENT bytes — `target_address` equals the part's own
//!   `event_log.content_address`, so a vouch for a different version of the part is not read —
//!   and it is filed under the part's OWN chart (`a.patient_id = n.patient_id`): the strict door
//!   demands that, but the remote door checks structure only, so a peer's vouch filed under
//!   another chart is admitted there and must not sign this part.
//!
//! The vouches are read as one column of the note statement itself (`vouches_column!`), so they
//! come from the same snapshot as the notes they belong to: a vouch can never be attached to a
//! read of its note that the database did not see together with it.
//!
//! Who a key belongs to is answered by db/071's doors (`cairn_note_key_actor`,
//! `cairn_note_key_role`) and db/004's `cairn_key_confirmation`, never by reading `actor_event`
//! (the window role may not).
use anyhow::Context;
use cairn_event::note_attestation::{VouchAct, VouchRole};
use serde::Deserialize;
use uuid::Uuid;

use super::view::UNKNOWN_SIGNER;

/// A person who signed or vouched for a part, as this node knows their key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signer {
    /// The key's actor, lower hex (a re-keyed member is still the same actor). EMPTY when this
    /// node does not know the key: an empty actor matches no one, itself included
    /// (`vouch::same_actor`), so an unknown key can never be taken for the presser or a writer.
    pub actor: String,
    /// Their display name (`cairn_note_signer_name`), or [`UNKNOWN_SIGNER`].
    pub name: String,
    /// The actor was enrolled with the pinned role `student`.
    pub student: bool,
    /// The key's enrolment is not yet confirmed (`unconfirmed`), or was approved by a key since
    /// repudiated (`doubtful`).
    pub unconfirmed: bool,
    /// The key was repudiated by a superuser since: its signature counts for nothing (ADR-0093).
    /// Exclusive with `unconfirmed`, as `cairn_key_confirmation` returns one word.
    pub repudiated: bool,
}

/// One verified vouch for a part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartVouch {
    pub event_id: Uuid,
    pub act: VouchAct,
    pub role: VouchRole,
    pub by: Signer,
}

/// Build a [`Signer`] from what the doors say about one key. Pure. `None` from a door means the
/// node does not know: no actor (matches no one), the unknown-signer words, not a student, and
/// not marked unconfirmed (`cairn_key_confirmation` says `unknown`, not `unconfirmed`).
pub fn signer_of(
    actor: Option<String>,
    name: Option<String>,
    pinned_role: Option<&str>,
    confirmation: Option<&str>,
) -> Signer {
    Signer {
        actor: actor.unwrap_or_default(),
        name: name.unwrap_or_else(|| UNKNOWN_SIGNER.into()),
        student: pinned_role == Some("student"),
        unconfirmed: matches!(confirmation, Some("unconfirmed" | "doubtful")),
        repudiated: confirmation == Some("repudiated"),
    }
}

/// The note statement's vouch column: a JSON array of the part's verified vouches, unordered
/// (ordered in Rust, ADR-0045). Correlated on the note statement's `n` (`clinical_note`) and `e`
/// (`event_log`) aliases. A macro, not a `const`, so `read.rs` can splice it into `NOTES_SQL` with
/// `concat!` and both stay compile-time strings.
macro_rules! vouches_column {
    () => {
        "(SELECT COALESCE(jsonb_agg(jsonb_build_object( \
                    'event_id', a.event_id::text, 'act', a.act, 'role', a.role, \
                    'hlc_wall', a.hlc_wall, 'hlc_counter', a.hlc_counter, \
                    'actor', cairn_note_key_actor(a.signer_key_id), \
                    'name', cairn_note_signer_name(a.signer_key_id), \
                    'pinned_role', cairn_note_key_role(a.signer_key_id), \
                    'confirmation', cairn_key_confirmation(a.signer_key_id))), '[]'::jsonb)::text \
            FROM clinical_note_attestation a \
           WHERE a.target = n.event_id \
             AND a.patient_id = n.patient_id \
             AND a.target_address = e.content_address \
             AND cairn_note_vouched_attester(a.event_id) = a.signer_key_id)"
    };
}
pub(super) use vouches_column;

/// The vouch column on its own, for the guard test below and any reader that wants the text.
pub const VOUCHES_SQL: &str = vouches_column!();

/// One element of the vouch column, as the database wrote it.
#[derive(Deserialize)]
struct VouchJson {
    event_id: String,
    act: String,
    role: String,
    hlc_wall: Option<i64>,
    hlc_counter: Option<i32>,
    actor: Option<String>,
    name: Option<String>,
    pinned_role: Option<String>,
    confirmation: Option<String>,
}

/// The act a stored word names. The applier projects only the two words this build knows, so
/// any other word is an ERROR, never a guess (as `read::by_discarder` treats a discard's words).
fn act_of(word: &str) -> anyhow::Result<VouchAct> {
    [VouchAct::Addendum, VouchAct::SignOff]
        .into_iter()
        .find(|a| a.as_wire() == word)
        .with_context(|| format!("a vouch's act is {word:?}, which this build does not know"))
}

/// The role a stored word names; any other word is an error (see [`act_of`]).
fn role_of(word: &str) -> anyhow::Result<VouchRole> {
    [VouchRole::Attested, VouchRole::CoSigned]
        .into_iter()
        .find(|r| r.as_wire() == word)
        .with_context(|| format!("a vouch's role is {word:?}, which this build does not know"))
}

/// Parse one part's vouch column into its vouches, in `(hlc_wall, hlc_counter, event_id)` order
/// — the order the byline lists them in and `signed_off_by` reads them in. Pure.
pub fn parse_vouches(column: &str) -> anyhow::Result<Vec<PartVouch>> {
    let raw: Vec<VouchJson> = serde_json::from_str(column).context("a part's vouch column")?;
    let mut keyed = Vec::with_capacity(raw.len());
    for v in raw {
        let event_id: Uuid = v.event_id.parse()?;
        keyed.push((
            (v.hlc_wall, v.hlc_counter, event_id),
            PartVouch {
                event_id,
                act: act_of(&v.act)?,
                role: role_of(&v.role)?,
                by: signer_of(
                    v.actor,
                    v.name,
                    v.pinned_role.as_deref(),
                    v.confirmation.as_deref(),
                ),
            },
        ));
    }
    keyed.sort_by_key(|(order, _)| *order);
    Ok(keyed.into_iter().map(|(_, v)| v).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn element(id: u128, act: &str, role: &str, wall: i64, counter: i32) -> serde_json::Value {
        serde_json::json!({
            "event_id": Uuid::from_u128(id).to_string(), "act": act, "role": role,
            "hlc_wall": wall, "hlc_counter": counter, "actor": "ab", "name": "Dr Zed",
            "pinned_role": "registrar", "confirmation": "confirmed",
        })
    }

    #[test]
    fn vouches_are_read_in_hlc_order() {
        let col = serde_json::json!([
            element(3, "sign-off", "co-signed", 20, 0),
            element(1, "addendum", "attested", 10, 5),
            element(2, "sign-off", "attested", 10, 5),
        ])
        .to_string();
        let ids: Vec<Uuid> = parse_vouches(&col)
            .unwrap()
            .iter()
            .map(|v| v.event_id)
            .collect();
        assert_eq!(ids, [1, 2, 3].map(Uuid::from_u128));
        assert!(parse_vouches("[]").unwrap().is_empty());
    }

    #[test]
    fn a_word_this_build_does_not_know_is_an_error() {
        for (act, role) in [("sign-off", "authored"), ("xyz", "attested")] {
            let col = serde_json::json!([element(1, act, role, 1, 0)]).to_string();
            assert!(parse_vouches(&col).is_err(), "{act} {role}");
        }
    }

    #[test]
    fn a_key_the_node_does_not_know_matches_no_actor_and_is_named_so() {
        let s = signer_of(None, None, None, Some("unknown"));
        assert_eq!(
            s,
            Signer {
                actor: String::new(),
                name: UNKNOWN_SIGNER.into(),
                student: false,
                unconfirmed: false,
                repudiated: false,
            }
        );
        assert!(signer_of(None, None, Some("student"), Some("doubtful")).student);
        assert!(signer_of(None, None, None, Some("doubtful")).unconfirmed);
        assert!(signer_of(None, None, None, Some("unconfirmed")).unconfirmed);
        let r = signer_of(None, None, None, Some("repudiated"));
        assert!(r.repudiated && !r.unconfirmed);
    }

    /// Review Focus 3: only a VERIFIED vouch naming the part's OWN bytes, filed under the part's
    /// own chart, is read.
    #[test]
    fn only_a_verified_vouch_for_the_parts_own_bytes_is_read() {
        assert!(VOUCHES_SQL.contains("a.target_address = e.content_address"));
        assert!(VOUCHES_SQL.contains("a.patient_id = n.patient_id"));
        assert!(VOUCHES_SQL.contains("cairn_note_vouched_attester(a.event_id) = a.signer_key_id"));
        assert!(!VOUCHES_SQL.contains("attester_key"), "{VOUCHES_SQL}");
    }
}
