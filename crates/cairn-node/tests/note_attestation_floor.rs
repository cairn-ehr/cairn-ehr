//! Sign-in slice 3c-2, Task 2 — the vouch stream `clinical.note-attestation.asserted` at the
//! database: the floor (`db/071`'s `cairn_check_note_attestation`), its projection
//! `clinical_note_attestation`, the shred hook (db/037), and the three reader doors.
//!
//! A vouch is a human saying "I stand behind that note part", later and separately: the part's
//! own writer signing it (`addendum`), or a clinician signing a whole note off (`sign-off`). The
//! signature is real, so the STRICT door refuses one a raw-SQL client could forge: an addendum
//! for a part someone else drafted, a co-signature of one's own signature or of a part no human
//! signed, a vouch filed on another chart, or one naming a stale version of the part. The REMOTE
//! door checks structure only (ADR-0091 decision 3), so a vouch may land before its note.
//!
//! `db/tests/071_clinical_note_attestation_test.sql` pins the structural rules one by one; this
//! file drives the real seal -> sign -> attest -> submit path.
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard` taken BEFORE connecting.
//! Key material is minted at runtime (house rule 6).
mod common;
#[path = "common/note_vouch_kit.rs"]
mod note_vouch_kit;

use cairn_event::note_attestation::{VouchAct, VouchRole};
use cairn_event::{event_address, generate_key, sign, sign_attestation, SigningKey};
use cairn_node::db;
use cairn_node::medication::AuthorParams;
use cairn_node::shred::shred_event;
use common::{enroll_human_with_role, note_setup, submit_registration};
use note_vouch_kit::{
    address_hex, projected, signed_part, submit_vouch, submit_vouch_as, timer_part, vouch_body,
    Vouch, ORIGIN,
};
use tokio_postgres::Client;
use uuid::Uuid;

/// The suite's fixture: the node, `notewriter` (the note fixture's human), and a chart.
struct Ward {
    node_sk: SigningKey,
    node_kid: String,
    writer_sk: SigningKey,
    writer_kid: String,
    chart: Uuid,
}

async fn ward(c: &Client) -> Ward {
    let (node_sk, node_kid, writer_sk, writer_kid, chart) = note_setup(c).await;
    Ward {
        node_sk,
        node_kid,
        writer_sk,
        writer_kid,
        chart,
    }
}

/// A part `notewriter` signed with their own key.
async fn writers_signed_part(c: &mut Client, w: &Ward) -> Uuid {
    let author = AuthorParams {
        human_sk: &w.writer_sk,
        human_kid: &w.writer_kid,
    };
    signed_part(c, &w.node_sk, &author, w.chart).await
}

/// A second clinician, a distinct actor from `notewriter`.
async fn colleague(c: &Client) -> (SigningKey, String) {
    enroll_human_with_role(c, "registrar").await
}

/// The vouch for `target` on `w`'s chart, by `sk`/`kid`, naming the target's real address.
async fn vouch_for<'a>(
    c: &Client,
    w: &Ward,
    target: Uuid,
    act: VouchAct,
    role: VouchRole,
    sk: &'a SigningKey,
    kid: &'a str,
) -> Vouch<'a> {
    Vouch {
        patient: w.chart,
        target,
        address: address_hex(c, target).await,
        act,
        role,
        sk,
        kid,
    }
}

/// `r` was refused, with a message carrying `frag`.
fn refused(r: Result<Uuid, String>, frag: &str) {
    match r {
        Ok(id) => panic!("admitted {id}, but it must be refused ({frag})"),
        Err(msg) => assert!(msg.contains(frag), "refused for the wrong reason: {msg}"),
    }
}

/// The hex actor id an enrolled key belongs to (its enroll row).
async fn actor_hex(c: &Client, kid: &str) -> String {
    c.query_one(
        "SELECT encode(actor_id, 'hex') FROM actor_event WHERE signing_key_id = $1 AND op = 'enroll'",
        &[&kid],
    )
    .await
    .unwrap()
    .get(0)
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

db_test!(a_sign_off_vouch_is_admitted_and_projected, |c| {
    let w = ward(&c).await;
    let part = writers_signed_part(&mut c, &w).await;
    let (b_sk, b_kid) = colleague(&c).await;
    let v = vouch_for(
        &c,
        &w,
        part,
        VouchAct::SignOff,
        VouchRole::CoSigned,
        &b_sk,
        &b_kid,
    )
    .await;

    let id = submit_vouch(&c, &v).await.expect("a colleague co-signs");

    assert_eq!(
        projected(&c, id).await,
        Some((
            w.chart.to_string(),
            part.to_string(),
            v.address.clone(),
            "sign-off".into(),
            "co-signed".into(),
            b_kid.clone(),
        ))
    );
    let same_address: bool = c
        .query_one(
            "SELECT a.content_address = e.content_address AND a.t_effective IS NULL \
               FROM clinical_note_attestation a JOIN event_log e USING (event_id) \
              WHERE a.event_id = $1::text::uuid",
            &[&id.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert!(same_address, "the row is keyed to the vouch's own address");
});

db_test!(an_addendum_vouch_by_the_parts_own_writer_is_admitted, |c| {
    let w = ward(&c).await;
    let part = timer_part(&mut c, &w.node_sk, w.chart).await;
    let v = vouch_for(
        &c,
        &w,
        part,
        VouchAct::Addendum,
        VouchRole::Attested,
        &w.writer_sk,
        &w.writer_kid,
    )
    .await;

    let id = submit_vouch(&c, &v)
        .await
        .expect("the writer signs it later");
    assert_eq!(projected(&c, id).await.unwrap().3, "addendum");
});

db_test!(an_addendum_vouch_for_someone_elses_part_is_refused, |c| {
    let w = ward(&c).await;
    let part = timer_part(&mut c, &w.node_sk, w.chart).await;
    let (b_sk, b_kid) = colleague(&c).await;
    let v = vouch_for(
        &c,
        &w,
        part,
        VouchAct::Addendum,
        VouchRole::Attested,
        &b_sk,
        &b_kid,
    )
    .await;
    refused(submit_vouch(&c, &v).await, "drafted by someone else");
});

db_test!(a_co_sign_of_ones_own_signature_is_refused, |c| {
    let w = ward(&c).await;
    let part = writers_signed_part(&mut c, &w).await;
    let v = vouch_for(
        &c,
        &w,
        part,
        VouchAct::SignOff,
        VouchRole::CoSigned,
        &w.writer_sk,
        &w.writer_kid,
    )
    .await;
    refused(submit_vouch(&c, &v).await, "the signer's own signature");
});

db_test!(a_co_sign_of_a_part_no_human_signed_is_refused, |c| {
    let w = ward(&c).await;
    let part = timer_part(&mut c, &w.node_sk, w.chart).await;
    let (b_sk, b_kid) = colleague(&c).await;
    let v = vouch_for(
        &c,
        &w,
        part,
        VouchAct::SignOff,
        VouchRole::CoSigned,
        &b_sk,
        &b_kid,
    )
    .await;
    refused(submit_vouch(&c, &v).await, "not signed by a human");
});

db_test!(a_target_on_another_chart_is_refused, |c| {
    let w = ward(&c).await;
    let part = writers_signed_part(&mut c, &w).await;
    let other = Uuid::now_v7();
    submit_registration(&c, &w.node_sk, &w.node_kid, other, 0).await;
    let (b_sk, b_kid) = colleague(&c).await;
    let mut v = vouch_for(
        &c,
        &w,
        part,
        VouchAct::SignOff,
        VouchRole::CoSigned,
        &b_sk,
        &b_kid,
    )
    .await;
    v.patient = other;
    refused(submit_vouch(&c, &v).await, "lives on chart");
});

db_test!(a_stale_target_address_is_refused, |c| {
    let w = ward(&c).await;
    let part = writers_signed_part(&mut c, &w).await;
    let (b_sk, b_kid) = colleague(&c).await;
    let mut v = vouch_for(
        &c,
        &w,
        part,
        VouchAct::SignOff,
        VouchRole::CoSigned,
        &b_sk,
        &b_kid,
    )
    .await;
    // Another version's address: well-formed, just not this part's.
    v.address = hex::encode(event_address(b"another version of the part"));
    refused(submit_vouch(&c, &v).await, "is not the content address");
});

db_test!(a_target_that_is_not_a_note_is_refused, |c| {
    let w = ward(&c).await;
    // The chart's registration: a real event on the chart, but not a note.
    let registration: Uuid = c
        .query_one(
            "SELECT event_id::text FROM event_log WHERE patient_id = $1::text::uuid \
               AND event_type = 'identity.registration.asserted'",
            &[&w.chart.to_string()],
        )
        .await
        .unwrap()
        .get::<_, String>(0)
        .parse()
        .unwrap();
    let (b_sk, b_kid) = colleague(&c).await;
    let v = vouch_for(
        &c,
        &w,
        registration,
        VouchAct::SignOff,
        VouchRole::Attested,
        &b_sk,
        &b_kid,
    )
    .await;
    refused(submit_vouch(&c, &v).await, "is not a note");
});

db_test!(a_vouch_without_a_token_is_refused, |c| {
    let w = ward(&c).await;
    let part = writers_signed_part(&mut c, &w).await;
    let (b_sk, b_kid) = colleague(&c).await;
    let v = vouch_for(
        &c,
        &w,
        part,
        VouchAct::SignOff,
        VouchRole::CoSigned,
        &b_sk,
        &b_kid,
    )
    .await;
    refused(
        submit_vouch_as(&c, &v, false).await,
        "requires attestation (no token presented)",
    );
});

// Review Focus 4: a peer's vouch reaches this node before its note. Admitted (no error, so the
// puller pens nothing) and projected; that the read counts it once the note lands is pinned end
// to end by `note_vouch_remote_read.rs::a_vouch_ahead_of_its_note_counts_once_the_note_lands`.
// UNSEALED through the remote door, as note_addendum_ahead_of_target.rs explains: the lenient
// arm admits it and runs the full floor on a clear body.
db_test!(the_remote_door_admits_a_vouch_whose_note_is_not_held, |c| {
    let w = ward(&c).await;
    let (peer_sk, peer_kid) = enroll_human_with_role(&c, "peer-clinician").await;
    let absent = Uuid::now_v7();
    let v = Vouch {
        patient: w.chart,
        target: absent,
        address: hex::encode(event_address(b"a note this node never saw")),
        act: VouchAct::SignOff,
        role: VouchRole::Attested,
        sk: &peer_sk,
        kid: &peer_kid,
    };
    let id = Uuid::now_v7();
    let hlc = cairn_event::Hlc {
        wall: chrono::Utc::now().timestamp_millis(),
        counter: 0,
        node_origin: "peer".into(),
    };
    common::apply_remote_attested(&c, &peer_sk, vouch_body(id, hlc, &v), &peer_sk, &peer_kid)
        .await
        .unwrap_or_else(|e| panic!("the remote door admits it: {}", common::db_msg(&e)));
    assert_eq!(
        projected(&c, id).await.map(|r| r.1),
        Some(absent.to_string())
    );
});

db_test!(
    a_re_keyed_writer_vouches_for_a_part_drafted_under_the_old_key,
    |c| {
        let w = ward(&c).await;
        let part = timer_part(&mut c, &w.node_sk, w.chart).await;
        let actor: Vec<u8> = c
            .query_one(
                "SELECT actor_id FROM actor_event WHERE signing_key_id = $1 AND op = 'enroll'",
                &[&w.writer_kid],
            )
            .await
            .unwrap()
            .get(0);
        let (new_sk, new_kid) = generate_key().unwrap();
        c.execute(
            "SELECT cairn_actor_append($1, 'rotate-key', $2, NULL, NULL)",
            &[&actor, &new_kid],
        )
        .await
        .unwrap();
        let v = vouch_for(
            &c,
            &w,
            part,
            VouchAct::Addendum,
            VouchRole::Attested,
            &new_sk,
            &new_kid,
        )
        .await;
        submit_vouch(&c, &v)
            .await
            .expect("the same person, under a new key, is still the part's writer");
    }
);

db_test!(a_shred_scrubs_the_projection, |c| {
    let w = ward(&c).await;
    let part = writers_signed_part(&mut c, &w).await;
    let (b_sk, b_kid) = colleague(&c).await;
    let v = vouch_for(
        &c,
        &w,
        part,
        VouchAct::SignOff,
        VouchRole::CoSigned,
        &b_sk,
        &b_kid,
    )
    .await;
    let id = submit_vouch(&c, &v).await.unwrap();
    assert!(projected(&c, id).await.is_some());

    shred_event(
        &mut c,
        &w.node_sk,
        &w.node_kid,
        ORIGIN,
        id,
        "retention ceiling",
        None,
    )
    .await
    .expect("the shred is admitted");
    assert_eq!(projected(&c, id).await, None, "the shred scrubs the row");
});

db_test!(the_doors_answer_actor_role_and_staff_actor, |c| {
    let w = ward(&c).await;
    let (_, student_kid) = enroll_human_with_role(&c, "student").await;
    let (_, unknown_kid) = generate_key().unwrap();
    let one = |q: &'static str, arg: String| {
        let c = &c;
        async move {
            c.query_one(q, &[&arg])
                .await
                .unwrap()
                .get::<_, Option<String>>(0)
        }
    };
    let writer = actor_hex(&c, &w.writer_kid).await;

    assert_eq!(
        one("SELECT cairn_note_key_actor($1)", w.writer_kid.clone()).await,
        Some(writer.clone())
    );
    assert_eq!(
        one("SELECT cairn_note_key_role($1)", student_kid.clone()).await,
        Some("student".into())
    );
    assert_eq!(
        one("SELECT cairn_note_key_role($1)", w.writer_kid.clone()).await,
        Some("clinician".into())
    );
    assert_eq!(
        one("SELECT cairn_note_key_actor($1)", unknown_kid.clone()).await,
        None
    );
    assert_eq!(
        one("SELECT cairn_note_key_role($1)", unknown_kid).await,
        None
    );
    // Normalised as the vault's doors normalise a staff id.
    assert_eq!(
        one("SELECT cairn_note_staff_actor($1)", " NoteWriter ".into()).await,
        Some(writer)
    );
    assert_eq!(
        one("SELECT cairn_note_staff_actor($1)", "nobody".into()).await,
        None
    );
});

/// `v` through the REMOTE door as a peer would deliver it, its token carrying `v`'s own role
/// (`common::apply_remote_attested` fixes the role to `attested`).
async fn apply_remote_vouch(c: &Client, v: &Vouch<'_>) -> Result<u64, String> {
    let hlc = cairn_event::Hlc {
        wall: chrono::Utc::now().timestamp_millis(),
        counter: 0,
        node_origin: "peer".into(),
    };
    let signed = sign(&vouch_body(Uuid::now_v7(), hlc, v), v.sk).unwrap();
    let ca = event_address(&signed.signed_bytes);
    let token = sign_attestation(&ca, v.kid, v.role.as_wire(), v.sk).unwrap();
    let vk = v.sk.verifying_key().to_bytes().to_vec();
    c.execute(
        "SELECT apply_remote_event($1,$2,$3)",
        &[&signed.signed_bytes, &token, &vk],
    )
    .await
    .map_err(|e| common::db_msg(&e))
}

// Final-check review (2026-11-06), plan Review Focus 3: the remote door admits the two forgeries
// the strict door refuses EVEN WHILE the note is held, because the part rules read node-local
// state (a writer's `rotate-key` row may not have synced yet): judging them there would fork the
// event set (ADR-0091 decision 3). The read, not the door, keeps them from counting.
db_test!(
    the_remote_door_admits_a_part_rule_forgery_for_a_held_note,
    |c| {
        let w = ward(&c).await;
        let (peer_sk, peer_kid) = colleague(&c).await;
        let unsigned = timer_part(&mut c, &w.node_sk, w.chart).await;
        let own = writers_signed_part(&mut c, &w).await;

        // An addendum vouch for a part someone else drafted.
        let v = vouch_for(
            &c,
            &w,
            unsigned,
            VouchAct::Addendum,
            VouchRole::Attested,
            &peer_sk,
            &peer_kid,
        )
        .await;
        apply_remote_vouch(&c, &v)
            .await
            .unwrap_or_else(|e| panic!("an addendum for another's part is admitted: {e}"));
        // A co-sign of one's own signature.
        let v = vouch_for(
            &c,
            &w,
            own,
            VouchAct::SignOff,
            VouchRole::CoSigned,
            &w.writer_sk,
            &w.writer_kid,
        )
        .await;
        apply_remote_vouch(&c, &v)
            .await
            .unwrap_or_else(|e| panic!("a co-sign of one's own signature is admitted: {e}"));
    }
);
