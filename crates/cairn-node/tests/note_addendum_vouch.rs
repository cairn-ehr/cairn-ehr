//! Sign-in slice 3c-2, Task 4 — an addendum's signature covers its writer's earlier unsigned
//! parts (`note::sign::sign_note`'s vouch step), against a real database.
//!
//! On paper a nurse who left a line unsigned comes back, adds a line and signs, and that one
//! signature covers her earlier line too. Here the earlier line went in unsigned (the quarantine
//! timer committed it, `{writer, drafted}`), and Sign's same press files one `addendum` vouch
//! (`attested`) for it — but ONLY for a part the window's strip listed (`also_vouches`) AND that
//! the signer drafted, decided by key. Nurse B signing while the strip still lists Nurse A's
//! parts vouches none of them (Review Focus 1); a part the strip did not list is never vouched
//! unseen; a new note and the timer vouch nothing.
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard` taken BEFORE connecting.
//! Key material is minted at runtime (house rule 6).
mod common;
#[path = "common/note_vouch_kit.rs"]
mod note_vouch_kit;

use cairn_event::SigningKey;
use cairn_node::db;
use cairn_node::note::compound::{compound_state, compounds, CompoundNote, CompoundState};
use cairn_node::note::read::list_patient_notes;
use cairn_node::note::sign::{sign_note, SignNoteInput, SignedNote};
use cairn_node::note::vouch::part_is_signed;
use common::note_setup;
use note_vouch_kit::{address_hex, member, projected, timer_part, timer_part_to, Member, ORIGIN};
use tokio_postgres::Client;
use uuid::Uuid;

/// The suite's fixture: the node, a chart, `notewriter` (Dr Ada Notewriter, whose parts the
/// timer commits) and Dr Zed (a registrar, another person).
struct Ward {
    node_sk: SigningKey,
    chart: Uuid,
    ada: Member,
    zed: Member,
}

async fn ward(c: &Client) -> Ward {
    let (node_sk, _, ada_sk, ada_kid, chart) = note_setup(c).await;
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

/// `m` signs a one-part draft of their own adding to `to` (`None` = a new note), the strip
/// having listed `listed` as the parts the signature also covers. Returns what Sign reports.
async fn sign(
    c: &mut Client,
    w: &Ward,
    m: &Member,
    to: Option<Uuid>,
    listed: &[Uuid],
) -> SignedNote {
    sign_claimed(c, w, m, m.staff, to, listed).await
}

/// [`sign`], but with `m`'s KEY while the who's-here claim names `writer` (the claim and the key
/// can disagree: a colleague presses Sign at a terminal still claimed by someone else).
async fn sign_claimed(
    c: &mut Client,
    w: &Ward,
    m: &Member,
    writer: &str,
    to: Option<Uuid>,
    listed: &[Uuid],
) -> SignedNote {
    let input = SignNoteInput {
        draft: Uuid::now_v7(),
        patient: w.chart,
        terminal: "ward-3-pc",
        writer: Some(writer),
        narrative: "reviewed, obs stable",
        stated: None,
        node_origin: ORIGIN,
        addendum_to: to,
        also_vouches: listed,
    };
    sign_note(c, &w.node_sk, &input, &m.author())
        .await
        .unwrap()
        .expect("the note signs")
        .value
}

/// The compound `root` as the chart reads it.
async fn note(c: &Client, w: &Ward, root: Uuid) -> CompoundNote {
    let notes = list_patient_notes(c, w.chart).await.unwrap().notes;
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

db_test!(
    signing_an_addendum_vouches_the_writers_own_unsigned_part,
    |c| {
        let w = ward(&c).await;
        let p = timer_part(&mut c, &w.node_sk, w.chart).await;
        let address = address_hex(&c, p).await;

        let signed = sign(&mut c, &w, &w.ada, Some(p), &[p]).await;
        assert_eq!(signed.parts.len(), 1, "the addendum is one part");
        assert_eq!(signed.vouched.len(), 1, "her earlier part is vouched");
        let row = projected(&c, signed.vouched[0])
            .await
            .expect("the vouch is projected");
        assert_eq!(
            row,
            (
                w.chart.to_string(),
                p.to_string(),
                address,
                "addendum".into(),
                "attested".into(),
                w.ada.kid.clone()
            )
        );

        let after = note(&c, &w, p).await;
        assert!(part_is_signed(&after.parts()[0]));
        assert_eq!(compound_state(after.parts()), CompoundState::EverySigned);
    }
);

// Review Focus 1: Nurse B signs while the strip (who's here) still lists Nurse A's parts. The
// claim still names Ada, so the strip listed her part; the KEY is Zed's, and the key decides.
db_test!(a_listed_part_that_is_not_the_signers_is_not_vouched, |c| {
    let w = ward(&c).await;
    let p = timer_part(&mut c, &w.node_sk, w.chart).await;

    let signed = sign_claimed(&mut c, &w, &w.zed, w.ada.staff, Some(p), &[p]).await;
    assert_eq!(signed.parts.len(), 1, "Zed's addendum still signs");
    assert!(signed.vouched.is_empty(), "Ada's part is not Zed's by key");
    assert_eq!(vouch_count(&c).await, 0);
    assert_eq!(
        compound_state(note(&c, &w, p).await.parts()),
        CompoundState::IncludesUnsigned
    );
});

db_test!(an_unlisted_own_part_is_not_vouched, |c| {
    let w = ward(&c).await;
    // Two parts of Ada's went in unsigned; the strip listed only the first.
    let p = timer_part(&mut c, &w.node_sk, w.chart).await;
    let q = timer_part_to(&mut c, &w.node_sk, w.chart, Some(p)).await;

    let signed = sign(&mut c, &w, &w.ada, Some(p), &[p]).await;
    assert_eq!(signed.vouched.len(), 1, "only the listed part");
    let row = projected(&c, signed.vouched[0]).await.unwrap();
    assert_eq!(row.1, p.to_string());

    let after = note(&c, &w, p).await;
    let unlisted = after.parts().iter().find(|r| r.event_id == q).unwrap();
    assert!(!part_is_signed(unlisted), "never vouched unseen");
    assert_eq!(
        compound_state(after.parts()),
        CompoundState::IncludesUnsigned
    );
});

db_test!(a_new_note_vouches_nothing, |c| {
    let w = ward(&c).await;
    let p = timer_part(&mut c, &w.node_sk, w.chart).await;

    // A new note adds to nothing, so it covers nothing, whatever the window sent.
    let signed = sign(&mut c, &w, &w.ada, None, &[p]).await;
    assert_eq!(signed.parts.len(), 1);
    assert!(signed.vouched.is_empty());
    assert_eq!(vouch_count(&c).await, 0);
});

db_test!(the_timer_commit_vouches_nothing, |c| {
    let w = ward(&c).await;
    // The timer holds no human key: Ada's unsigned addendum to her unsigned note vouches nothing.
    let p = timer_part(&mut c, &w.node_sk, w.chart).await;
    timer_part_to(&mut c, &w.node_sk, w.chart, Some(p)).await;

    assert_eq!(vouch_count(&c).await, 0);
    let after = note(&c, &w, p).await;
    assert!(after.parts().iter().all(|r| !part_is_signed(r)));
    assert_eq!(
        compound_state(after.parts()),
        CompoundState::IncludesUnsigned
    );
});

// Final-check review (2026-11-06): the header may target ANY part of a note, not only its root
// (db/070's door admits any note of the chart; a link judgement or a healed body can turn an
// orphan root into a middle part while the draft waits). The window shows the covers line for
// the note CONTAINING the target, so Sign must vouch within that same note.
db_test!(
    an_addendum_to_a_middle_part_vouches_the_writers_listed_parts,
    |c| {
        let w = ward(&c).await;
        let p = timer_part(&mut c, &w.node_sk, w.chart).await;
        let q = timer_part_to(&mut c, &w.node_sk, w.chart, Some(p)).await;

        let signed = sign(&mut c, &w, &w.ada, Some(q), &[p, q]).await;
        assert_eq!(
            signed.vouched.len(),
            2,
            "both listed parts of the note are vouched"
        );
        assert_eq!(
            compound_state(note(&c, &w, p).await.parts()),
            CompoundState::EverySigned
        );
    }
);
