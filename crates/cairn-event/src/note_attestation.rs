//! The later vouch for one part of a progress note, `clinical.note-attestation.asserted`
//! (sign-in slice 3c-2). Pure: no clock, no randomness, no I/O.
//!
//! Why it exists: a note part may go in unsigned (the quarantine timer committed it, or its
//! writer was not known). A vouch is a SEPARATE event in which a human says "I stand
//! behind that part": either its own writer signing it later (an `addendum`), or a
//! clinician signing a note off (`sign-off`). The part itself is never edited.
//! The vouch names the part by id AND by content address, so a vouch for a different
//! version of a part can never be mistaken for a vouch for this one.
use serde_json::{json, Value};

pub const NOTE_ATTESTATION_EVENT_TYPE: &str = "clinical.note-attestation.asserted";
pub const NOTE_ATTESTATION_SCHEMA_VERSION: &str = "clinical.note-attestation/1";

/// Why the vouch was made. Frozen wire words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VouchAct {
    /// The part's own writer signing it later (alongside a new addendum).
    Addendum,
    /// A clinician signing off a whole note.
    SignOff,
}

impl VouchAct {
    /// The frozen wire spelling.
    pub fn as_wire(&self) -> &'static str {
        match self {
            VouchAct::Addendum => "addendum",
            VouchAct::SignOff => "sign-off",
        }
    }
}

/// The responsibility the vouching human takes. Frozen wire words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VouchRole {
    /// Any vouch that is not a second signature by someone else over a human's signed envelope:
    /// the first signature of an unsigned part, or a sign-off of a part whose signer this node
    /// cannot place as another human (`vouch::sign_off_vouches`; ADR-0092 decision 11).
    Attested,
    /// The part was already signed by someone else; this is a second signature.
    CoSigned,
}

impl VouchRole {
    /// The frozen wire spelling.
    pub fn as_wire(&self) -> &'static str {
        match self {
            VouchRole::Attested => "attested",
            VouchRole::CoSigned => "co-signed",
        }
    }
}

/// Everything a vouch says: which part, which version of it, and why.
#[derive(Debug, Clone, Copy)]
pub struct NoteVouch<'a> {
    /// The vouched part's event id (a uuid string).
    pub target: &'a str,
    /// Lower hex of the target's `content_address`, multihash prefix included.
    pub target_address_hex: &'a str,
    pub act: VouchAct,
    pub role: VouchRole,
}

/// Build the `clinical.note-attestation/1` payload: exactly three frozen keys.
/// An `addendum` is the writer's own first signature, so it is never `co-signed`;
/// the database floor refuses that pairing, and this asserts it in debug builds.
pub fn note_attestation_payload(v: &NoteVouch) -> Value {
    debug_assert!(
        !(v.act == VouchAct::Addendum && v.role == VouchRole::CoSigned),
        "an addendum vouch carries `attested`, never `co-signed`"
    );
    json!({
        "target": v.target,
        "target_address": v.target_address_hex,
        "act": v.act.as_wire(),
    })
}

/// The contributor list: one human who bears responsibility, held by their own key.
pub fn vouch_contributors(kid: &str, role: VouchRole) -> Value {
    json!([{
        "actor_id": kid,
        "role": role.as_wire(),
        "responsibility": { "held_by": kid },
    }])
}

/// The plaintext legibility twin: fixed copy, no free text.
pub fn render_note_attestation_twin(v: &NoteVouch) -> String {
    let words = match v.act {
        VouchAct::Addendum => "signed later by its writer",
        VouchAct::SignOff => "the note signed off",
    };
    format!(
        "Vouches for note part {}: {} ({}).",
        v.target,
        words,
        v.role.as_wire()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "01900000-0000-7000-8000-000000000001";

    fn vouch(act: VouchAct, role: VouchRole) -> NoteVouch<'static> {
        NoteVouch {
            target: ID,
            target_address_hex: "1220ab",
            act,
            role,
        }
    }

    #[test]
    fn the_payload_is_exactly_three_keys() {
        let v = note_attestation_payload(&vouch(VouchAct::SignOff, VouchRole::CoSigned));
        let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(|k| k.as_str()).collect();
        keys.sort();
        assert_eq!(keys, ["act", "target", "target_address"]);
        assert_eq!(v["target"], ID);
        assert_eq!(v["target_address"], "1220ab");
        assert_eq!(v["act"], "sign-off");
    }

    #[test]
    fn the_contributor_bears_responsibility_held_by_itself() {
        let c = vouch_contributors("kid1", VouchRole::Attested);
        assert_eq!(
            c,
            json!([{"actor_id": "kid1", "role": "attested", "responsibility": {"held_by": "kid1"}}])
        );
        assert_eq!(
            vouch_contributors("k", VouchRole::CoSigned)[0]["role"],
            "co-signed"
        );
    }

    #[test]
    fn the_twin_words_are_exact() {
        assert_eq!(
            render_note_attestation_twin(&vouch(VouchAct::Addendum, VouchRole::Attested)),
            format!("Vouches for note part {ID}: signed later by its writer (attested).")
        );
        assert_eq!(
            render_note_attestation_twin(&vouch(VouchAct::SignOff, VouchRole::CoSigned)),
            format!("Vouches for note part {ID}: the note signed off (co-signed).")
        );
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "never `co-signed`")]
    fn an_addendum_act_is_never_co_signed() {
        note_attestation_payload(&vouch(VouchAct::Addendum, VouchRole::CoSigned));
    }
}
