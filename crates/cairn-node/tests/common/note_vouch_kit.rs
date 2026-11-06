//! The vouch-suite fixture (sign-in slice 3c-2): the note parts a vouch can name, and a vouch
//! submitted the way the node will submit one (seal, sign with the human's key, attest with the
//! same key, then the 4-argument strict door).
//!
//! Include it with `#[path = "common/note_vouch_kit.rs"] mod note_vouch_kit;`. The including
//! binary must ALSO declare `mod common;` (it reads `common::db_msg` and the note fixture).
//! Kept out of `common/mod.rs` so its helpers do not join `identity_scaffolding_shared.rs`'s
//! derived list (the medication_kit precedent).
#![allow(dead_code)] // each including suite uses a different subset

use crate::common;
use cairn_event::note_attestation::{
    note_attestation_payload, render_note_attestation_twin, vouch_contributors, NoteVouch,
    VouchAct, VouchRole, NOTE_ATTESTATION_EVENT_TYPE, NOTE_ATTESTATION_SCHEMA_VERSION,
};
use cairn_event::{event_address, sign_attestation, ClockGrade, EventBody, Hlc, SigningKey};
use cairn_node::db;
use cairn_node::medication::sealed_submit::seal_and_sign;
use cairn_node::medication::AuthorParams;
use cairn_node::note::commit::{commit_overdue, CommitOutcome};
use cairn_node::note::draft::{save_part, SaveOutcome};
use cairn_node::note::sign::{sign_note, SignNoteInput};
use tokio_postgres::Client;
use uuid::Uuid;

pub const ORIGIN: &str = "note-vouch-kit-node";
const TERMINAL: &str = "ward-3-pc";

/// Everything one vouch says, and who says it.
pub struct Vouch<'a> {
    /// The chart the vouch is filed under (the floor demands the target's own).
    pub patient: Uuid,
    pub target: Uuid,
    /// Lower hex of the target's content address.
    pub address: String,
    pub act: VouchAct,
    pub role: VouchRole,
    pub sk: &'a SigningKey,
    pub kid: &'a str,
}

/// The CLEAR body of `v`, exactly as the wire constraint fixes it: signed and attested by the
/// one human it names, `t_effective` none (as `build_attestation_body`).
pub fn vouch_body(event_id: Uuid, hlc: Hlc, v: &Vouch<'_>) -> EventBody {
    let target = v.target.to_string();
    let nv = NoteVouch {
        target: &target,
        target_address_hex: &v.address,
        act: v.act,
        role: v.role,
    };
    EventBody {
        event_id: event_id.to_string(),
        patient_id: v.patient.to_string(),
        event_type: NOTE_ATTESTATION_EVENT_TYPE.into(),
        schema_version: NOTE_ATTESTATION_SCHEMA_VERSION.into(),
        hlc,
        t_effective: None,
        signer_key_id: v.kid.into(),
        contributors: vouch_contributors(v.kid, v.role),
        payload: note_attestation_payload(&nv),
        attachments: vec![],
        plaintext_twin: Some(render_note_attestation_twin(&nv)),
        clock_grade: ClockGrade::SelfAsserted,
        safety: None,
    }
}

/// Seal, sign and submit `v` through the strict door with its human token and DEK. `with_token
/// = false` drops the token (a raw-SQL client's shape). Returns the vouch's event id, or the
/// door's sentence.
pub async fn submit_vouch_as(c: &Client, v: &Vouch<'_>, with_token: bool) -> Result<Uuid, String> {
    let id = Uuid::now_v7();
    let hlc = db::next_hlc(c, ORIGIN).await.unwrap();
    let (signed, dek) = seal_and_sign(vouch_body(id, hlc, v), v.sk).unwrap();
    let ca = event_address(&signed);
    let token = sign_attestation(&ca, v.kid, v.role.as_wire(), v.sk).unwrap();
    let vk = v.sk.verifying_key().to_bytes().to_vec();
    let (token, vk) = if with_token {
        (Some(token), Some(vk))
    } else {
        (None, None)
    };
    c.execute(
        "SELECT submit_event($1, $2, $3, $4)",
        &[&signed, &token, &vk, &dek.as_bytes().as_slice()],
    )
    .await
    .map(|_| id)
    .map_err(|e| common::db_msg(&e))
}

/// [`submit_vouch_as`] with the token, as the node always sends it.
pub async fn submit_vouch(c: &Client, v: &Vouch<'_>) -> Result<Uuid, String> {
    submit_vouch_as(c, v, true).await
}

/// Lower hex of an event's `content_address`.
pub async fn address_hex(c: &Client, event: Uuid) -> String {
    hex::encode(common::content_address_of(c, event).await)
}

/// A note part `notewriter` signed with their own key (`{signer, authored}`): signed by a
/// human. Returns its event id.
pub async fn signed_part(
    c: &mut Client,
    node_sk: &SigningKey,
    author: &AuthorParams<'_>,
    chart: Uuid,
) -> Uuid {
    let input = SignNoteInput {
        draft: Uuid::now_v7(),
        patient: chart,
        terminal: TERMINAL,
        writer: Some("notewriter"),
        narrative: "chest clear",
        stated: None,
        node_origin: ORIGIN,
        addendum_to: None,
    };
    sign_note(c, node_sk, &input, author)
        .await
        .unwrap()
        .expect("the part is signed")
        .event_id
}

/// A note part `notewriter` drafted and nobody signed: the quarantine timer committed it, so it
/// is NODE-signed with `{writer, drafted} + {node, recorded}`. Returns its event id.
pub async fn timer_part(c: &mut Client, node_sk: &SigningKey, chart: Uuid) -> Uuid {
    let draft = Uuid::now_v7();
    let saved = save_part(
        c,
        draft,
        chart,
        TERMINAL,
        Some("notewriter"),
        "obs stable",
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(saved, SaveOutcome::Saved);
    // Overdue by moving the typing back past the quarantine, never by sleeping (note_commit.rs).
    c.execute(
        "WITH h AS (UPDATE note_draft SET last_keystroke = last_keystroke - interval '181 min', \
                           recording_started = recording_started - interval '181 min' \
                     WHERE draft_id = $1::text::uuid) \
         UPDATE note_draft_part SET last_keystroke = last_keystroke - interval '181 min', \
                recording_started = recording_started - interval '181 min' \
          WHERE draft_id = $1::text::uuid",
        &[&draft.to_string()],
    )
    .await
    .unwrap();
    match commit_overdue(c, node_sk, draft, ORIGIN).await.unwrap() {
        CommitOutcome::Committed(done) => done.event_id,
        other => panic!("the timer commits the part: {other:?}"),
    }
}

/// The projected row of one vouch: (patient, target, target_address hex, act, role, signer).
pub async fn projected(
    c: &Client,
    vouch: Uuid,
) -> Option<(String, String, String, String, String, String)> {
    c.query_opt(
        "SELECT patient_id::text, target::text, encode(target_address, 'hex'), act, role, \
                signer_key_id \
           FROM clinical_note_attestation WHERE event_id = $1::text::uuid",
        &[&vouch.to_string()],
    )
    .await
    .unwrap()
    .map(|r| (r.get(0), r.get(1), r.get(2), r.get(3), r.get(4), r.get(5)))
}
