//! Sign-in slice 3c-2 (final review) — a PEER's vouch, admitted by the remote door, and what the
//! chart then reads.
//!
//! The remote door checks a vouch's structure only (ADR-0092 decision 6, offline-first): it may
//! arrive before its note, and it may name a part of another chart than it is filed under. So
//! the read, not the door, decides whether a vouch counts (decision 9). This file pins both ends:
//!
//! * Review Focus 4 end to end: a vouch that lands BEFORE its note is admitted without error (so
//!   a puller has nothing to pen) and shows nothing; once the note lands, the part's byline
//!   carries it, with no re-offer and no pen row.
//! * A vouch counts only when filed under its part's OWN chart (`a.patient_id = n.patient_id`):
//!   one a peer filed under another chart changes no byline.
//!
//! Unsealed bodies through the 1- and 3-argument remote door, as `note_addendum_ahead_of_target.rs`
//! explains: the lenient arm admits an unsealed `clinical.*` body and runs the full floor on it.
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard` taken BEFORE connecting.
//! Key material is minted at runtime (house rule 6).
mod common;
#[path = "common/note_vouch_kit.rs"]
mod note_vouch_kit;

use cairn_event::contributor::with_human_author;
use cairn_event::note::{EscapedNarrative, NoteAssertion, StatedTime};
use cairn_event::note_attestation::{VouchAct, VouchRole};
use cairn_event::{event_address, sign, Hlc};
use cairn_node::db;
use cairn_node::note::compound::{compounds, CompoundNote};
use cairn_node::note::read::list_patient_notes;
use cairn_node::note::view::part_byline;
use common::note_setup;
use note_vouch_kit::{address_hex, member, projected, vouch_body, Member, Vouch};
use tokio_postgres::Client;
use uuid::Uuid;

/// An HLC from the peer, `counter` apart so two in one test never tie.
fn peer_hlc(counter: i32) -> Hlc {
    Hlc {
        wall: chrono::Utc::now().timestamp_millis(),
        counter,
        node_origin: "peer".into(),
    }
}

/// `peer`'s sign-off vouch for `target` (whose content address is `address`), filed under
/// `patient`, applied through the 3-argument remote door with its human token. Returns its id.
async fn remote_vouch(
    c: &Client,
    peer: &Member,
    patient: Uuid,
    target: Uuid,
    address: String,
    counter: i32,
) -> Uuid {
    let v = Vouch {
        patient,
        target,
        address,
        act: VouchAct::SignOff,
        role: VouchRole::Attested,
        sk: &peer.sk,
        kid: &peer.kid,
    };
    let id = Uuid::now_v7();
    common::apply_remote_attested(
        c,
        &peer.sk,
        vouch_body(id, peer_hlc(counter), &v),
        &peer.sk,
        &peer.kid,
    )
    .await
    .unwrap_or_else(|e| panic!("the remote door admits the vouch: {}", common::db_msg(&e)));
    id
}

/// The compound `root` as the chart `patient` reads it, or `None` when it is not read.
async fn note(c: &Client, patient: Uuid, root: Uuid) -> Option<CompoundNote> {
    let notes = list_patient_notes(c, patient).await.unwrap().notes;
    compounds(notes).into_iter().find(|n| n.root() == root)
}

macro_rules! db_test {
    ($name:ident, |$c:ident| $body:block) => {
        #[tokio::test]
        async fn $name() {
            let Some(base) = common::cs() else {
                eprintln!("skipped: set CAIRN_TEST_PG");
                return;
            };
            let _guard = db::test_serial_guard(&base).await.unwrap();
            #[allow(unused_mut)]
            let mut $c = db::connect_and_load_schema(&base).await.unwrap();
            $body
        }
    };
}

// Review Focus 4, end to end: the vouch first, then its note, then the read.
db_test!(a_vouch_ahead_of_its_note_counts_once_the_note_lands, |c| {
    let (_, _, _, _, chart) = note_setup(&c).await;
    // The peer's two clinicians, enrolled here by the owner ceremony (actors do not travel).
    let writer = member(&c, "peerwriter", "Dr Writer", "peer-writer").await;
    let peer = member(&c, "peer", "Dr Peer", "peer-clinician").await;
    // The peer's note, signed now and held back: its bytes fix the address the vouch names.
    let a = NoteAssertion {
        narrative: EscapedNarrative::of_typed("seen at the clinic"),
        stated: StatedTime::NotStated,
        recording_started: "2026-11-01T01:00:00Z",
        recording_ended: "2026-11-01T01:05:00Z",
        addendum_to: None,
    };
    let n = Uuid::now_v7();
    let body = with_human_author(
        cairn_node::note::sign::note_body(n, chart, &writer.kid, peer_hlc(0), &a),
        &writer.kid,
    );
    let n_bytes = sign(&body, &writer.sk).unwrap().signed_bytes;
    let n_address = hex::encode(event_address(&n_bytes));

    // 1. The vouch lands first: admitted, projected, and the chart shows no such note.
    let v = remote_vouch(&c, &peer, chart, n, n_address.clone(), 1).await;
    assert!(projected(&c, v).await.is_some(), "the vouch is projected");
    assert!(
        note(&c, chart, n).await.is_none(),
        "no note to show it on yet"
    );

    // 2. The note lands; the vouch is NOT offered again.
    c.execute("SELECT apply_remote_event($1)", &[&n_bytes])
        .await
        .unwrap_or_else(|e| panic!("the note applies: {}", common::db_msg(&e)));
    assert_eq!(
        address_hex(&c, n).await,
        n_address,
        "the vouch named these bytes"
    );

    // 3. The read carries the vouch from the moment the note is there.
    let read = note(&c, chart, n).await.expect("the note is read");
    assert_eq!(read.parts()[0].vouches.len(), 1);
    assert_eq!(
        part_byline(&read.parts()[0]).shown,
        "signed by Dr Writer · signed off by Dr Peer"
    );
    // No pen row: the door admitted the vouch without error, so a puller had nothing to pen
    // (the pen is `cairn-sync`'s, written on a refusal only; asserted here for the record).
    let penned: i64 = c
        .query_one(
            "SELECT count(*) FROM sync_quarantine q JOIN event_log e \
                ON e.content_address = q.content_digest WHERE e.event_id = $1::text::uuid",
            &[&v.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(penned, 0);
});

// Decision 9: a vouch counts only under its part's own chart. The remote door admits one a peer
// filed under another chart (structure only), and the read must not let it sign this part.
db_test!(a_vouch_filed_under_another_chart_changes_no_byline, |c| {
    let (node_sk, node_kid, ada_sk, ada_kid, chart) = note_setup(&c).await;
    let author = cairn_node::medication::AuthorParams {
        human_sk: &ada_sk,
        human_kid: &ada_kid,
    };
    let p = note_vouch_kit::signed_part(&mut c, &node_sk, &author, chart).await;
    let other = Uuid::now_v7();
    common::submit_registration(&c, &node_sk, &node_kid, other, 0).await;
    let peer = member(&c, "peer", "Dr Peer", "peer-clinician").await;
    let before = part_byline(&note(&c, chart, p).await.unwrap().parts()[0]).shown;

    let misfiled = remote_vouch(&c, &peer, other, p, address_hex(&c, p).await, 1).await;
    assert_eq!(
        projected(&c, misfiled).await.map(|r| r.0),
        Some(other.to_string()),
        "projected, under the other chart"
    );
    let read = note(&c, chart, p).await.unwrap();
    assert!(
        read.parts()[0].vouches.is_empty(),
        "{:?}",
        read.parts()[0].vouches
    );
    assert_eq!(part_byline(&read.parts()[0]).shown, before);

    // Positive control: the same peer's vouch filed under the part's own chart is read.
    remote_vouch(&c, &peer, chart, p, address_hex(&c, p).await, 2).await;
    assert_eq!(
        part_byline(&note(&c, chart, p).await.unwrap().parts()[0]).shown,
        format!("{before} · signed off by Dr Peer")
    );
});
