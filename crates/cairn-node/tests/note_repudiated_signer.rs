//! Sign-in slice 3c-3b — a note part signed by a since-REPUDIATED key counts as unsigned, against
//! a real database (ADR-0085/0086 for repudiation; ADR-0093 for what the note does about it).
//!
//! The impostor case: someone signs with a stolen key, the key is repudiated, and the part still
//! carries "signed by Dr Ada". On paper a forged signature is struck out and the real person
//! initials again. Here the part reads "— counts as unsigned"; a colleague's sign-off is only an
//! attestation and clears nothing; only the real person (same ACTOR, new key) re-signing does.
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard` taken BEFORE connecting.
//! Key material is minted at runtime (house rule 6).
mod common;
#[path = "common/note_vouch_kit.rs"]
mod note_vouch_kit;

use cairn_node::db;
use cairn_node::note::compound::{compounds, CompoundNote};
use cairn_node::note::read::list_patient_notes;
use cairn_node::note::sign::{sign_note, SignNoteInput};
use cairn_node::note::view::part_byline;
use cairn_node::note::vouch::{needs_sign_off, sign_off_note, SignOffInput, SignOffRefusal};
use common::note_setup;
use note_vouch_kit::{member, projected, Member, ORIGIN};
use tokio_postgres::Client;
use uuid::Uuid;

/// The fixture: the node's key, a chart, Dr Ada Notewriter (the note fixture's clinician) and a
/// registrar, Dr Zed.
struct Ward {
    node_sk: cairn_event::SigningKey,
    chart: Uuid,
    ada: Member,
    zed: Member,
}

async fn ward(c: &Client) -> Ward {
    let (node_sk, _node_kid, ada_sk, ada_kid, chart) = note_setup(c).await;
    Ward {
        node_sk,
        chart,
        ada: Member {
            sk: ada_sk,
            kid: ada_kid,
            staff: "notewriter",
        },
        zed: member(c, "zed", "Dr Zed", "registrar").await,
    }
}

/// `m` signs a one-part note of their own on the ward's chart. Returns its id.
async fn sign_as(c: &mut Client, w: &Ward, m: &Member) -> Uuid {
    let input = SignNoteInput {
        draft: Uuid::now_v7(),
        patient: w.chart,
        terminal: "ward-3-pc",
        writer: Some(m.staff),
        narrative: "obs stable",
        stated: None,
        node_origin: ORIGIN,
        addendum_to: None,
        also_vouches: &[],
    };
    sign_note(c, &w.node_sk, &input, &m.author())
        .await
        .unwrap()
        .expect("the note signs")
        .event_id
}

/// `m` presses "Sign off this note" on the note `root`, having been shown only `root`.
async fn sign_off(
    c: &mut Client,
    m: &Member,
    w: &Ward,
    root: Uuid,
) -> Result<Vec<Uuid>, SignOffRefusal> {
    let input = SignOffInput {
        patient: w.chart,
        root,
        shown_parts: &[root],
        node_origin: ORIGIN,
    };
    sign_off_note(c, &input, &m.author()).await.unwrap()
}

/// The note `root` as the chart reads it.
async fn note(c: &Client, w: &Ward, root: Uuid) -> CompoundNote {
    let notes = list_patient_notes(c, w.chart).await.unwrap().notes;
    compounds(notes)
        .into_iter()
        .find(|n| n.root() == root)
        .expect("the note is read")
}

/// Repudiate `m`'s key and give the same person (same actor) a fresh one, as ADR-0085/0086 do
/// after a stolen key: rotate first, then repudiate the OLD key. Returns the member on the new key.
async fn repudiate_and_rekey(c: &Client, m: &Member) -> Member {
    let aid: Vec<u8> = c
        .query_one(
            "SELECT actor_id FROM actor_event WHERE signing_key_id = $1 AND op = 'enroll'",
            &[&m.kid],
        )
        .await
        .unwrap()
        .get(0);
    let (sk, kid) = cairn_event::generate_key().unwrap();
    c.execute(
        "SELECT cairn_actor_append($1, 'rotate-key', $2, NULL, NULL)",
        &[&aid, &kid],
    )
    .await
    .unwrap();
    c.execute(
        "SELECT cairn_actor_append($1, 'repudiate', $2, 'console', NULL)",
        &[&aid, &m.kid],
    )
    .await
    .unwrap();
    let state: String = c
        .query_one("SELECT cairn_key_confirmation($1)", &[&m.kid])
        .await
        .unwrap()
        .get(0);
    assert_eq!(state, "repudiated", "the fixture's premise");
    Member {
        sk,
        kid,
        staff: m.staff,
    }
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

const FORGED: &str = "signed by Dr Ada Notewriter (key had been repudiated) — counts as unsigned";

// Review Focus 1 + 2: a colleague clears nothing; the re-keyed real person is not "NothingToSign".
db_test!(
    a_repudiated_signers_part_reads_unsigned_until_they_re_sign,
    |c| {
        let w = ward(&c).await;
        let p = sign_as(&mut c, &w, &w.ada).await;
        let ada2 = repudiate_and_rekey(&c, &w.ada).await;

        let n = note(&c, &w, p).await;
        assert_eq!(part_byline(&n.parts()[0]).shown, FORGED);
        assert!(needs_sign_off(n.parts()));

        sign_off(&mut c, &w.zed, &w, p)
            .await
            .expect("Zed signs off");
        let n = note(&c, &w, p).await;
        assert_eq!(
            part_byline(&n.parts()[0]).shown,
            format!("{FORGED} · signed off by Dr Zed")
        );
        assert!(needs_sign_off(n.parts()), "a colleague clears nothing");

        let ids = sign_off(&mut c, &ada2, &w, p)
            .await
            .expect("the real person is not told NothingToSign");
        assert_eq!(ids.len(), 1);
        let n = note(&c, &w, p).await;
        assert_eq!(
            part_byline(&n.parts()[0]).shown,
            "signed by Dr Ada Notewriter (key had been repudiated) · signed off by Dr Zed · \
             re-signed by Dr Ada Notewriter"
        );
        assert!(!needs_sign_off(n.parts()));
    }
);

// Review Focus 3: the planner attests, never co-signs, a repudiated part (through the floor).
db_test!(a_colleagues_vouch_on_a_repudiated_part_is_attested, |c| {
    let w = ward(&c).await;
    let p = sign_as(&mut c, &w, &w.ada).await;
    repudiate_and_rekey(&c, &w.ada).await;

    let ids = sign_off(&mut c, &w.zed, &w, p)
        .await
        .expect("Zed signs off");
    let row = projected(&c, ids[0]).await.expect("the vouch is projected");
    assert_eq!(row.4, "attested", "never co-signed over a forged signature");
});

// #1009: once the real person re-signs, a supervisor's sign-off co-signs the part again, the
// floor admits it, and a student's "— not co-signed" clears.
db_test!(
    a_re_signed_students_part_is_co_signed_by_a_supervisor,
    |c| {
        let w = ward(&c).await;
        let sam = member(&c, "sam", "Sam", "student").await;
        let p = sign_as(&mut c, &w, &sam).await;
        let sam2 = repudiate_and_rekey(&c, &sam).await;

        sign_off(&mut c, &sam2, &w, p).await.expect("Sam re-signs");
        let n = note(&c, &w, p).await;
        assert_eq!(
            part_byline(&n.parts()[0]).shown,
            "signed by Sam (student) (key had been repudiated) — not co-signed · \
         re-signed by Sam (student)"
        );

        let ids = sign_off(&mut c, &w.zed, &w, p).await.expect("Zed co-signs");
        let row = projected(&c, ids[0]).await.expect("the vouch is projected");
        assert_eq!(row.4, "co-signed", "the re-signed signature stands again");
        let n = note(&c, &w, p).await;
        assert_eq!(
            part_byline(&n.parts()[0]).shown,
            "signed by Sam (student) (key had been repudiated) · re-signed by Sam (student) · \
         co-signed by Dr Zed"
        );
    }
);
