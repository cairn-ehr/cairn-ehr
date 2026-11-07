//! Sign-in slice 3c-3c, Task 2 — `parts_by_keys` reads note parts BY WRITER KEY through the very
//! query and row parser the chart list uses, so the cross-chart "Waiting for you" menu can never
//! disagree with the chart about a part. Against a real database.
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard` taken BEFORE connecting.
//! Key material is minted at runtime (house rule 6).
mod common;
#[path = "common/note_vouch_kit.rs"]
mod note_vouch_kit;

use cairn_event::SigningKey;
use cairn_node::db;
use cairn_node::note::read::{list_patient_notes, parts_by_keys};
use cairn_node::note::sign::{sign_note, SignNoteInput};
use cairn_node::note::supervision;
use common::note_setup;
use note_vouch_kit::{member, timer_part, Member, ORIGIN};
use tokio_postgres::Client;
use uuid::Uuid;

/// `m` signs a one-part note of their own on `chart`. Returns its id.
async fn sign_as(c: &mut Client, node_sk: &SigningKey, chart: Uuid, m: &Member) -> Uuid {
    let input = SignNoteInput {
        draft: Uuid::now_v7(),
        patient: chart,
        terminal: "ward-3-pc",
        writer: Some(m.staff),
        narrative: "obs stable",
        stated: None,
        node_origin: ORIGIN,
        addendum_to: None,
        also_vouches: &[],
    };
    sign_note(c, node_sk, &input, &m.author())
        .await
        .unwrap()
        .expect("the note signs")
        .event_id
}

/// The parts `keys` wrote, read with the supervision record as the menu will read them.
async fn by_keys(c: &Client, keys: &[&str]) -> Vec<cairn_node::note::read::NoteRow> {
    let keys: Vec<String> = keys.iter().map(|k| k.to_string()).collect();
    let sup = supervision::load(c).await.unwrap();
    parts_by_keys(c, &keys, &sup).await.unwrap()
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
            // A part's time is its HLC wall; start the clock from zero (see note_supervision.rs).
            $c.batch_execute("UPDATE hlc_state SET hlc_wall = 0, hlc_counter = 0")
                .await
                .unwrap();
            $body
        }
    };
}

db_test!(a_signed_part_is_found_by_its_signers_key, |c| {
    let (node_sk, _, _, _, chart) = note_setup(&c).await;
    let bea = member(&c, "bea", "Dr Bea", "clinician").await;
    let p = sign_as(&mut c, &node_sk, chart, &bea).await;
    let got = by_keys(&c, &[&bea.kid]).await;
    assert_eq!(got.iter().map(|r| r.event_id).collect::<Vec<_>>(), [p]);
});

db_test!(
    a_part_that_went_in_unsigned_is_found_by_its_drafted_writers_key,
    |c| {
        let (node_sk, _, _, ada_kid, chart) = {
            let (a, b, _sk, kid, d) = note_setup(&c).await;
            (a, b, (), kid, d)
        };
        // The timer commits Ada's draft NODE-signed: only the `drafted` contributor names her.
        let p = timer_part(&mut c, &node_sk, chart).await;
        let got = by_keys(&c, &[&ada_kid]).await;
        assert_eq!(got.iter().map(|r| r.event_id).collect::<Vec<_>>(), [p]);
    }
);

db_test!(
    a_part_signed_with_a_since_rotated_key_is_found_by_that_key,
    |c| {
        let (node_sk, _, _, _, chart) = note_setup(&c).await;
        let bea = member(&c, "bea", "Dr Bea", "clinician").await;
        let p = sign_as(&mut c, &node_sk, chart, &bea).await;
        let actor: Vec<u8> = c
            .query_one(
                "SELECT actor_id FROM actor_event WHERE signing_key_id = $1 AND op = 'enroll'",
                &[&bea.kid],
            )
            .await
            .unwrap()
            .get(0);
        let (_new_sk, new_kid) = cairn_event::generate_key().unwrap();
        c.execute(
            "SELECT cairn_actor_append($1, 'rotate-key', $2, NULL, NULL)",
            &[&actor, &new_kid],
        )
        .await
        .unwrap();
        // The caller passes every key the person ever held; the old one still finds the part.
        let got = by_keys(&c, &[&bea.kid, &new_kid]).await;
        assert_eq!(got.iter().map(|r| r.event_id).collect::<Vec<_>>(), [p]);
    }
);

db_test!(
    another_writers_part_on_the_same_chart_is_not_returned,
    |c| {
        let (node_sk, _, _, _, chart) = note_setup(&c).await;
        let bea = member(&c, "bea", "Dr Bea", "clinician").await;
        let cy = member(&c, "cy", "Dr Cy", "clinician").await;
        let mine = sign_as(&mut c, &node_sk, chart, &bea).await;
        let _theirs = sign_as(&mut c, &node_sk, chart, &cy).await;
        let got = by_keys(&c, &[&bea.kid]).await;
        assert_eq!(got.iter().map(|r| r.event_id).collect::<Vec<_>>(), [mine]);
        assert!(by_keys(&c, &[]).await.is_empty());
    }
);

db_test!(
    a_part_read_by_keys_equals_the_same_part_read_by_chart,
    |c| {
        let (node_sk, _, _, _, chart) = note_setup(&c).await;
        let bea = member(&c, "bea", "Dr Bea", "clinician").await;
        let sue = member(&c, "sue", "Dr Sue", "clinician").await;
        // Give the row non-default supervision fields: Bea needs co-signing and has Sue as supervisor.
        for (op, subject, sup) in [
            ("link-supervisor", "bea", Some("sue")),
            ("require-cosign", "bea", None),
        ] {
            c.execute(
                "SELECT cairn_supervision_console_act($1, $2, $3)",
                &[&op, &subject, &sup],
            )
            .await
            .unwrap();
        }
        tokio::time::sleep(std::time::Duration::from_millis(3)).await;
        let p = sign_as(&mut c, &node_sk, chart, &bea).await;
        let by_chart = list_patient_notes(&c, chart)
            .await
            .unwrap()
            .notes
            .into_iter()
            .find(|n| n.event_id == p)
            .unwrap();
        let by_key = by_keys(&c, &[&bea.kid]).await.pop().unwrap();
        assert!(by_chart.needs_cosigning, "the premise: a non-default row");
        assert!(!by_chart.writer_supervisors.is_empty());
        assert_eq!(by_key, by_chart);
        let _ = sue;
    }
);
