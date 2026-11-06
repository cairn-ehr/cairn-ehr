//! Sign-in slice 3c-2, Task 3 — "Sign off this note" (`note::vouch::sign_off_note`) and the
//! read of vouches (`note::read` + `note::vouch_read`), against a real database.
//!
//! On paper a supervisor reads a student's entry and countersigns beneath it. Here the
//! countersignature is a separate signed event per part (a vouch, db/071): `co-signed` for a part
//! someone else signed, `attested` for a part that went in unsigned, nothing for a part the
//! presser signed themselves. The node re-reads the note inside the sign-off's transaction and
//! signs nothing unless it holds exactly the parts the window showed (Review Focus 2). Two
//! sign-offs at once both stand (Review Focus 5). A vouch whose token is not verified, or that
//! names other bytes than the part's own, is never read (Review Focus 3).
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard` taken BEFORE connecting.
//! Key material is minted at runtime (house rule 6).
mod common;
#[path = "common/note_vouch_kit.rs"]
mod note_vouch_kit;

use cairn_event::contributor::with_human_author;
use cairn_event::note::{EscapedNarrative, NoteAssertion, StatedTime};
use cairn_event::note_attestation::{VouchAct, VouchRole};
use cairn_event::{event_address, generate_key, SigningKey};
use cairn_node::db;
use cairn_node::medication::AuthorParams;
use cairn_node::note::compound::{compound_state, compounds, CompoundNote, CompoundState};
use cairn_node::note::read::list_patient_notes;
use cairn_node::note::sign::{note_body, sign_note, SignNoteInput};
use cairn_node::note::view::part_byline;
use cairn_node::note::vouch::{needs_sign_off, sign_off_note, SignOffInput, SignOffRefusal};
use cairn_node::shred::shred_event;
use common::note_setup;
use note_vouch_kit::{projected, timer_part, vouch_body, Vouch, ORIGIN};
use tokio_postgres::Client;
use uuid::Uuid;

/// A vault member: their key and staff id.
struct Member {
    sk: SigningKey,
    kid: String,
    staff: &'static str,
}

impl Member {
    fn author(&self) -> AuthorParams<'_> {
        AuthorParams {
            human_sk: &self.sk,
            human_kid: &self.kid,
        }
    }
}

/// The suite's fixture: the node, a chart, and three people — `notewriter` (Dr Ada Notewriter,
/// the note fixture's clinician), Sam (a student) and Dr Zed (a registrar).
struct Ward {
    node_sk: SigningKey,
    node_kid: String,
    chart: Uuid,
    ada: Member,
    sam: Member,
    zed: Member,
}

/// Enrol a human actor pinned with `role` (and a handle of their own, so each is a distinct
/// actor) and give them a vault row named `name`, as the window's enrolment does.
async fn member(c: &Client, staff: &'static str, name: &str, role: &str) -> Member {
    let (sk, kid) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('human', jsonb_build_object('role', $2::text, 'handle', $3::text), $1)",
        &[&kid, &role, &staff],
    )
    .await
    .unwrap();
    c.execute(
        "INSERT INTO vault_member (staff_id, display_name, role, actor_id, signing_key_id, \
                                   sealed, enrolled_by, contact_email) \
         SELECT $2, $3, $4, actor_id, $1, '\\x00'::bytea, 'console', 'w@example.org' \
           FROM actor_event WHERE signing_key_id = $1 AND op = 'enroll' LIMIT 1",
        &[&kid, &staff, &name, &role],
    )
    .await
    .unwrap();
    Member { sk, kid, staff }
}

async fn ward(c: &Client) -> Ward {
    let (node_sk, node_kid, ada_sk, ada_kid, chart) = note_setup(c).await;
    Ward {
        node_sk,
        node_kid,
        chart,
        ada: Member {
            sk: ada_sk,
            kid: ada_kid,
            staff: "notewriter",
        },
        sam: member(c, "sam", "Sam", "student").await,
        zed: member(c, "zed", "Dr Zed", "registrar").await,
    }
}

/// `m` signs their own one-part note on `chart` (adding to `to`, if given). Returns its id.
async fn sign_as(c: &mut Client, w: &Ward, m: &Member, chart: Uuid, to: Option<Uuid>) -> Uuid {
    let input = SignNoteInput {
        draft: Uuid::now_v7(),
        patient: chart,
        terminal: "ward-3-pc",
        writer: Some(m.staff),
        narrative: "obs stable",
        stated: None,
        node_origin: ORIGIN,
        addendum_to: to,
    };
    sign_note(c, &w.node_sk, &input, &m.author())
        .await
        .unwrap()
        .expect("the note signs")
        .event_id
}

/// `m` presses "Sign off this note" on the compound `root`, having been shown `shown`.
async fn sign_off(
    c: &mut Client,
    m: &Member,
    patient: Uuid,
    root: Uuid,
    shown: &[Uuid],
) -> Result<Vec<Uuid>, SignOffRefusal> {
    let input = SignOffInput {
        patient,
        root,
        shown_parts: shown,
        node_origin: ORIGIN,
    };
    sign_off_note(c, &input, &m.author()).await.unwrap()
}

/// The compound `root` as the chart `patient` reads it.
async fn note(c: &Client, patient: Uuid, root: Uuid) -> CompoundNote {
    let notes = list_patient_notes(c, patient).await.unwrap().notes;
    compounds(notes)
        .into_iter()
        .find(|n| n.root() == root)
        .expect("the note is read")
}

/// How many vouches the projection holds.
async fn vouch_count(c: &Client) -> i64 {
    c.query_one("SELECT count(*) FROM clinical_note_attestation", &[])
        .await
        .unwrap()
        .get(0)
}

macro_rules! db_test {
    ($name:ident, |$base:ident, $c:ident| $body:block) => {
        #[tokio::test]
        async fn $name() {
            let Some($base) = common::cs() else {
                eprintln!("skipped: set CAIRN_TEST_PG");
                return;
            };
            let _guard = db::test_serial_guard(&$base).await.unwrap();
            #[allow(unused_mut)]
            let mut $c = db::connect_and_load_schema(&$base).await.unwrap();
            $body
        }
    };
}

db_test!(signing_off_a_students_note_co_signs_it, |_base, c| {
    let w = ward(&c).await;
    let p = sign_as(&mut c, &w, &w.sam, w.chart, None).await;
    let before = note(&c, w.chart, p).await;
    assert_eq!(
        part_byline(&before.parts()[0]).shown,
        "signed by Sam (student) — not co-signed"
    );
    assert!(needs_sign_off(before.parts()));

    let ids = sign_off(&mut c, &w.zed, w.chart, p, &[p]).await.unwrap();
    assert_eq!(ids.len(), 1);
    let row = projected(&c, ids[0]).await.expect("the vouch is projected");
    assert_eq!(
        (row.1, row.3.as_str(), row.4.as_str(), row.5),
        (p.to_string(), "sign-off", "co-signed", w.zed.kid.clone())
    );

    let after = note(&c, w.chart, p).await;
    assert_eq!(
        part_byline(&after.parts()[0]).shown,
        "signed by Sam (student) · co-signed by Dr Zed"
    );
    assert!(!needs_sign_off(after.parts()));
});

db_test!(
    signing_off_attests_a_part_that_went_in_unsigned,
    |_base, c| {
        let w = ward(&c).await;
        // Ada drafted it and nobody signed: the timer put it in unsigned. Zed adds to it and signs.
        let p = timer_part(&mut c, &w.node_sk, w.chart).await;
        let z = sign_as(&mut c, &w, &w.zed, w.chart, Some(p)).await;
        assert_eq!(
            compound_state(note(&c, w.chart, p).await.parts()),
            CompoundState::IncludesUnsigned
        );

        let ids = sign_off(&mut c, &w.zed, w.chart, p, &[p, z]).await.unwrap();
        assert_eq!(ids.len(), 1, "Zed's own part needs nothing more");
        let row = projected(&c, ids[0]).await.unwrap();
        assert_eq!((row.1, row.4.as_str()), (p.to_string(), "attested"));

        let after = note(&c, w.chart, p).await;
        let shown = part_byline(&after.parts()[0]).shown;
        assert!(
            shown.ends_with("went in unsigned · signed off by Dr Zed"),
            "{shown}"
        );
        assert_eq!(
            compound_state(after.parts()),
            CompoundState::SignedOff("Dr Zed".into())
        );
    }
);

// Review Focus 2: the note changed between showing and signing off.
db_test!(
    a_part_added_after_the_note_was_shown_refuses_changed,
    |_base, c| {
        let w = ward(&c).await;
        let p = sign_as(&mut c, &w, &w.sam, w.chart, None).await;
        let shown = [p];
        let a = sign_as(&mut c, &w, &w.ada, w.chart, Some(p)).await;

        assert_eq!(
            sign_off(&mut c, &w.zed, w.chart, p, &shown).await,
            Err(SignOffRefusal::Changed)
        );
        let gone = Uuid::now_v7();
        assert_eq!(
            sign_off(&mut c, &w.zed, w.chart, gone, &[gone]).await,
            Err(SignOffRefusal::Changed),
            "a note that is not on the chart"
        );
        assert_eq!(vouch_count(&c).await, 0, "nothing was signed");

        // Positive control: shown as it now is, it signs off.
        let ids = sign_off(&mut c, &w.zed, w.chart, p, &[p, a]).await.unwrap();
        assert_eq!(ids.len(), 2);
    }
);

db_test!(
    signing_off_a_note_one_signed_wholly_is_nothing_to_sign,
    |_base, c| {
        let w = ward(&c).await;
        let p = sign_as(&mut c, &w, &w.ada, w.chart, None).await;
        let a = sign_as(&mut c, &w, &w.ada, w.chart, Some(p)).await;
        assert!(!needs_sign_off(note(&c, w.chart, p).await.parts()));
        assert_eq!(
            sign_off(&mut c, &w.ada, w.chart, p, &[p, a]).await,
            Err(SignOffRefusal::NothingToSign)
        );
        assert_eq!(vouch_count(&c).await, 0);
    }
);

// Review Focus 5: two clinicians sign off the same note at once.
db_test!(two_sign_offs_at_once_both_stand, |base, c| {
    let w = ward(&c).await;
    let p = sign_as(&mut c, &w, &w.sam, w.chart, None).await;
    let q = sign_as(&mut c, &w, &w.sam, w.chart, Some(p)).await;
    let mut c2 = db::connect(&base).await.unwrap();
    let shown = [p, q];

    let (zed, ada) = tokio::join!(
        sign_off(&mut c, &w.zed, w.chart, p, &shown),
        sign_off(&mut c2, &w.ada, w.chart, p, &shown),
    );
    assert_eq!(zed.expect("Zed's press stands").len(), 2);
    assert_eq!(ada.expect("Ada's press stands").len(), 2);
    assert_eq!(vouch_count(&c).await, 4, "both sets of vouches are kept");

    let after = note(&c, w.chart, p).await;
    let first = after.parts()[0].vouches[0].by.name.clone();
    let mut names: Vec<&str> = after.parts()[0]
        .vouches
        .iter()
        .map(|v| v.by.name.as_str())
        .collect();
    names.sort();
    assert_eq!(names, ["Dr Ada Notewriter", "Dr Zed"]);
    assert_eq!(
        compound_state(after.parts()),
        CompoundState::SignedOff(first),
        "the first signer in root order"
    );
});

// Review Focus 3: a raw-SQL peer's vouch for stale bytes, or with an unverified token, is
// admitted at the remote door (structure only) but never read.
db_test!(a_forged_vouch_is_not_read, |_base, c| {
    let w = ward(&c).await;
    let p = sign_as(&mut c, &w, &w.sam, w.chart, None).await;
    let peer = member(&c, "peer", "Dr Peer", "peer-clinician").await;
    let real_address = note_vouch_kit::address_hex(&c, p).await;
    let mut counter = 0;
    let mut remote = |address: String| {
        counter += 1;
        let v = Vouch {
            patient: w.chart,
            target: p,
            address,
            act: VouchAct::SignOff,
            role: VouchRole::Attested,
            sk: &peer.sk,
            kid: &peer.kid,
        };
        let hlc = cairn_event::Hlc {
            wall: chrono::Utc::now().timestamp_millis(),
            counter,
            node_origin: "peer".into(),
        };
        let id = Uuid::now_v7();
        (id, vouch_body(id, hlc, &v))
    };
    let apply = |body| common::apply_remote_attested(&c, &peer.sk, body, &peer.sk, &peer.kid);

    let (stale, body) = remote(hex::encode(event_address(b"another version of the part")));
    apply(body).await.expect("the remote door admits it");
    let (unverified, body) = remote(real_address.clone());
    apply(body).await.expect("the remote door admits it");
    // The token is marked as db/020's deferred arm marks one it never verified.
    c.execute(
        "INSERT INTO event_attestation_unvouched (event_id) VALUES ($1::text::uuid)",
        &[&unverified.to_string()],
    )
    .await
    .unwrap();
    assert!(projected(&c, stale).await.is_some() && projected(&c, unverified).await.is_some());

    let read = note(&c, w.chart, p).await;
    assert!(read.parts()[0].vouches.is_empty());
    assert_eq!(
        part_byline(&read.parts()[0]).shown,
        "signed by Sam (student) — not co-signed"
    );

    // Positive control: the same peer's honest vouch is read.
    let (_honest, body) = remote(real_address);
    apply(body).await.unwrap();
    assert_eq!(
        part_byline(&note(&c, w.chart, p).await.parts()[0]).shown,
        "signed by Sam (student) — not co-signed · signed off by Dr Peer"
    );
});

db_test!(
    a_linked_charts_part_is_vouched_on_its_own_chart,
    |_base, c| {
        let w = ward(&c).await;
        let x = Uuid::now_v7();
        common::submit_registration(&c, &w.node_sk, &w.node_kid, x, 0).await;
        let p = sign_as(&mut c, &w, &w.sam, x, None).await;
        common::submit_link_event(&c, &w.node_sk, &w.node_kid, w.chart, x, 10, true).await;

        let ids = sign_off(&mut c, &w.zed, w.chart, p, &[p]).await.unwrap();
        let row = projected(&c, ids[0]).await.unwrap();
        assert_eq!(row.0, x.to_string(), "filed under the part's own chart");
        assert_eq!(
            part_byline(&note(&c, w.chart, p).await.parts()[0]).shown,
            "signed by Sam (student) · co-signed by Dr Zed"
        );
    }
);

// Review Focus 2, the other way a note changes: a part shredded after the note was shown.
db_test!(
    a_part_shredded_after_the_note_was_shown_refuses_changed,
    |_base, c| {
        let w = ward(&c).await;
        let p = sign_as(&mut c, &w, &w.sam, w.chart, None).await;
        let a = sign_as(&mut c, &w, &w.ada, w.chart, Some(p)).await;
        let shown = [p, a];
        assert_eq!(note(&c, w.chart, p).await.parts().len(), 2);
        shred_event(
            &mut c,
            &w.node_sk,
            &w.node_kid,
            ORIGIN,
            a,
            "retention ceiling",
            None,
        )
        .await
        .expect("the shred is admitted");

        assert_eq!(
            sign_off(&mut c, &w.zed, w.chart, p, &shown).await,
            Err(SignOffRefusal::Changed)
        );
        assert_eq!(vouch_count(&c).await, 0, "nothing was signed");
    }
);

// A peer's note signed by a key this node cannot place (its registry no longer holds the key, as
// a restored node's may not): the floor co-signs only a key it knows as a human's, so the
// sign-off ATTESTS the part, and succeeds.
db_test!(
    signing_off_a_part_signed_by_an_unknown_key_attests_it,
    |_base, c| {
        let w = ward(&c).await;
        // The remote door admits only an enrolled signer, so the peer is enrolled for the apply.
        let (peer_sk, peer_kid) = common::enroll_human_with_role(&c, "peer-clinician").await;
        let a = NoteAssertion {
            narrative: EscapedNarrative::of_typed("seen at the clinic"),
            stated: StatedTime::NotStated,
            recording_started: "2026-11-01T01:00:00Z",
            recording_ended: "2026-11-01T01:05:00Z",
            addendum_to: None,
        };
        let hlc = cairn_event::Hlc {
            wall: chrono::Utc::now().timestamp_millis(),
            counter: 0,
            node_origin: "peer".into(),
        };
        let p = Uuid::now_v7();
        let body = with_human_author(note_body(p, w.chart, &peer_kid, hlc, &a), &peer_kid);
        common::apply_remote_raw(&c, &peer_sk, body)
            .await
            .unwrap_or_else(|e| panic!("the remote door admits it: {}", common::db_msg(&e)));
        // Then the registry forgets the key. `actor_event` is append-only (db/004's trigger), so the
        // trigger comes off for this one fixture act, as dr_clinical_guarantee_gap.rs does; a node
        // never does this. ONE batch, so one implicit transaction: `DISABLE TRIGGER` is persistent
        // DDL, and a failure part-way must roll it back rather than leave the shared test database
        // without its append-only floor. The key is lower hex (`generate_key`), checked before it
        // is spliced into the SQL.
        assert!(peer_kid.bytes().all(|b| b.is_ascii_hexdigit()));
        c.batch_execute(&format!(
            "ALTER TABLE actor_event DISABLE TRIGGER actor_event_no_update;
             DELETE FROM actor_event WHERE signing_key_id = '{peer_kid}';
             ALTER TABLE actor_event ENABLE TRIGGER actor_event_no_update;"
        ))
        .await
        .unwrap();
        let before = note(&c, w.chart, p).await;
        let signer = before.parts()[0].signed_by.clone().expect("a signed part");
        assert_eq!(signer.actor, "", "a key this node cannot place");
        assert!(needs_sign_off(before.parts()));

        let ids = sign_off(&mut c, &w.zed, w.chart, p, &[p])
            .await
            .expect("the sign-off stands");
        let row = projected(&c, ids[0]).await.unwrap();
        assert_eq!((row.3.as_str(), row.4.as_str()), ("sign-off", "attested"));
        assert_eq!(
            part_byline(&note(&c, w.chart, p).await.parts()[0]).shown,
            "signed by a key this node does not know · signed off by Dr Zed"
        );
    }
);
