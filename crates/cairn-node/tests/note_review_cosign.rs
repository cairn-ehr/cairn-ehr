//! Sign-in slice 3c-3a, Task 6 — "Co-sign as reviewed" (`note::review::review_note`), against a
//! real database.
//!
//! On paper a supervisor reads their registrar's entry and initials it as reviewed, though no
//! countersignature was owed. Here that is a `sign-off`/`co-signed` vouch per part the presser's
//! supervisee signed, filed in one transaction through Sign off's own (`vouch_as_shown`), so the
//! note must still hold exactly the parts shown. Pinned here (Review Focus 5): it never vouches
//! another writer's part, and the presser is whoever's KEY signs. The who's-here claim is not an
//! input to the gesture at all: a clinician named in who's-here signing with a colleague's key
//! reviews as that colleague.
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard` taken BEFORE connecting.
//! Key material is minted at runtime (house rule 6).
mod common;
#[path = "common/note_vouch_kit.rs"]
mod note_vouch_kit;

use cairn_event::SigningKey;
use cairn_node::db;
use cairn_node::note::compound::{compounds, CompoundNote};
use cairn_node::note::read::list_patient_notes;
use cairn_node::note::review::{can_review, review_note};
use cairn_node::note::sign::{sign_note, SignNoteInput};
use cairn_node::note::view::part_byline;
use cairn_node::note::vouch::{sign_off_note, SignOffInput, SignOffRefusal};
use common::note_setup;
use note_vouch_kit::{member, projected, timer_part_by, Member, ORIGIN};
use tokio_postgres::Client;
use uuid::Uuid;

/// One supervision act through the owner's console door: `op` on `subject`, naming `supervisor`
/// for a link op.
async fn console(c: &Client, op: &str, subject: &str, supervisor: Option<&str>) {
    c.execute(
        "SELECT cairn_supervision_console_act($1, $2, $3)",
        &[&op, &subject, &supervisor],
    )
    .await
    .unwrap_or_else(|e| panic!("{op} {subject}: {}", common::db_msg(&e)));
}

/// `m` signs a one-part note on `chart` (adding to `to`, if given). Returns its id.
async fn sign_as(
    c: &mut Client,
    node_sk: &SigningKey,
    chart: Uuid,
    m: &Member,
    to: Option<Uuid>,
) -> Uuid {
    let input = SignNoteInput {
        draft: Uuid::now_v7(),
        patient: chart,
        terminal: "ward-3-pc",
        writer: Some(m.staff),
        narrative: "obs stable",
        stated: None,
        node_origin: ORIGIN,
        addendum_to: to,
        also_vouches: &[],
    };
    sign_note(c, node_sk, &input, &m.author())
        .await
        .unwrap()
        .expect("the note signs")
        .event_id
}

/// `m` presses "Co-sign as reviewed" on the compound `root`, having been shown `shown`.
async fn review(
    c: &mut Client,
    m: &Member,
    chart: Uuid,
    root: Uuid,
    shown: &[Uuid],
) -> Result<Vec<Uuid>, SignOffRefusal> {
    let input = SignOffInput {
        patient: chart,
        root,
        shown_parts: shown,
        node_origin: ORIGIN,
    };
    review_note(c, &input, &m.author()).await.unwrap()
}

/// The compound `root` as the chart reads it.
async fn note(c: &Client, chart: Uuid, root: Uuid) -> CompoundNote {
    let notes = list_patient_notes(c, chart).await.unwrap().notes;
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

/// `m`'s actor id, read from their KEY as the gesture reads the presser's.
async fn actor_of(c: &Client, m: &Member) -> String {
    c.query_one("SELECT cairn_note_key_actor($1)", &[&m.kid])
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
            // A part's time is its HLC wall, a registry row's the DB clock: a clock a previous
            // suite left ahead (hlc_drift parks it near the drift ceiling) would time this
            // suite's parts after acts recorded later. Every write below ticks it back to now.
            $c.batch_execute("UPDATE hlc_state SET hlc_wall = 0, hlc_counter = 0")
                .await
                .unwrap();
            $body
        }
    };
}

// The gesture end to end: Sue supervises Bea, who owes no co-sign; Sue co-signs Bea's note as
// reviewed, it reads as a co-sign, and is not offered again.
db_test!(a_supervisor_co_signs_a_reviewed_note, |c| {
    let (node_sk, _, _, _, chart) = note_setup(&c).await;
    let bea = member(&c, "bea", "Dr Bea", "clinician").await;
    let sue = member(&c, "sue", "Dr Sue", "registrar").await;
    console(&c, "link-supervisor", "bea", Some("sue")).await;
    let p = sign_as(&mut c, &node_sk, chart, &bea, None).await;
    let sue_actor = actor_of(&c, &sue).await;
    assert!(can_review(note(&c, chart, p).await.parts(), &sue_actor));

    let ids = review(&mut c, &sue, chart, p, &[p])
        .await
        .expect("the review writes a vouch");
    assert_eq!(ids.len(), 1);
    let row = projected(&c, ids[0]).await.expect("the vouch is projected");
    assert_eq!(row.1, p.to_string());
    assert_eq!((row.3.as_str(), row.4.as_str()), ("sign-off", "co-signed"));
    let n = note(&c, chart, p).await;
    assert_eq!(
        part_byline(&n.parts()[0]).shown,
        "signed by Dr Bea · co-signed by Dr Sue"
    );
    assert!(
        !can_review(n.parts(), &sue_actor),
        "reviewed once is enough"
    );
    assert_eq!(
        review(&mut c, &sue, chart, p, &[p]).await,
        Err(SignOffRefusal::NothingToSign)
    );
    assert_eq!(vouch_count(&c).await, 1);
});

// Review Focus 5: Bea's part and Cal's addendum on one note. Sue supervises Bea only, so only
// Bea's part is vouched; Cal's is not hers to review.
db_test!(review_never_vouches_another_writers_part, |c| {
    let (node_sk, _, _, _, chart) = note_setup(&c).await;
    let bea = member(&c, "bea", "Dr Bea", "clinician").await;
    let cal = member(&c, "cal", "Dr Cal", "clinician").await;
    let sue = member(&c, "sue", "Dr Sue", "registrar").await;
    console(&c, "link-supervisor", "bea", Some("sue")).await;
    let p = sign_as(&mut c, &node_sk, chart, &bea, None).await;
    let a = sign_as(&mut c, &node_sk, chart, &cal, Some(p)).await;

    let ids = review(&mut c, &sue, chart, p, &[p, a])
        .await
        .expect("the review writes a vouch");
    assert_eq!(ids.len(), 1, "one vouch, for Bea's part only");
    assert_eq!(projected(&c, ids[0]).await.unwrap().1, p.to_string());
    assert_eq!(vouch_count(&c).await, 1);
    let n = note(&c, chart, p).await;
    assert!(!part_byline(&n.parts()[1]).shown.contains("Dr Sue"));
});

// Review Focus 5: the KEY decides, not the who's-here claim. Sue is Bea's supervisor; Zed is
// not. A press that signs with Zed's key reviews as Zed — whoever the terminal says is here
// (the claim never reaches the gesture) — and Zed supervises no one, so nothing is written.
db_test!(the_key_not_the_claim_decides, |c| {
    let (node_sk, _, _, _, chart) = note_setup(&c).await;
    let bea = member(&c, "bea", "Dr Bea", "clinician").await;
    let sue = member(&c, "sue", "Dr Sue", "registrar").await;
    let zed = member(&c, "zed", "Dr Zed", "registrar").await;
    console(&c, "link-supervisor", "bea", Some("sue")).await;
    let p = sign_as(&mut c, &node_sk, chart, &bea, None).await;

    assert_eq!(
        review(&mut c, &zed, chart, p, &[p]).await,
        Err(SignOffRefusal::NothingToSign)
    );
    assert_eq!(vouch_count(&c).await, 0, "nothing was signed");
    // The control: Sue's own key reviews it.
    assert!(review(&mut c, &sue, chart, p, &[p]).await.is_ok());
});

// As Sign off: a part that landed after the note was shown refuses, and nothing is signed unseen.
db_test!(a_part_added_after_it_was_shown_refuses_changed, |c| {
    let (node_sk, _, _, _, chart) = note_setup(&c).await;
    let bea = member(&c, "bea", "Dr Bea", "clinician").await;
    let sue = member(&c, "sue", "Dr Sue", "registrar").await;
    console(&c, "link-supervisor", "bea", Some("sue")).await;
    let p = sign_as(&mut c, &node_sk, chart, &bea, None).await;
    sign_as(&mut c, &node_sk, chart, &bea, Some(p)).await;

    assert_eq!(
        review(&mut c, &sue, chart, p, &[p]).await,
        Err(SignOffRefusal::Changed)
    );
    assert_eq!(vouch_count(&c).await, 0, "nothing was signed");
});

// Design §3.3, "never ... an unsigned part": Bea's part went in unsigned on the quarantine timer
// and colleague Cal signed it off (an attested vouch, the only one the floor admits on a
// node-keyed part). Bea never signed it herself, so there is no supervisee's signature for Sue to
// review: nothing is offered, and nothing is written.
db_test!(
    a_colleagues_sign_off_of_a_timer_part_is_not_reviewable,
    |c| {
        let (node_sk, _, _, _, chart) = note_setup(&c).await;
        let _bea = member(&c, "bea", "Dr Bea", "clinician").await;
        let cal = member(&c, "cal", "Dr Cal", "clinician").await;
        let sue = member(&c, "sue", "Dr Sue", "registrar").await;
        console(&c, "link-supervisor", "bea", Some("sue")).await;
        let p = timer_part_by(&mut c, &node_sk, chart, "bea", None).await;
        let input = SignOffInput {
            patient: chart,
            root: p,
            shown_parts: &[p],
            node_origin: ORIGIN,
        };
        sign_off_note(&mut c, &input, &cal.author())
            .await
            .unwrap()
            .expect("Cal signs the part off");
        assert_eq!(vouch_count(&c).await, 1);

        let sue_actor = actor_of(&c, &sue).await;
        assert!(!can_review(note(&c, chart, p).await.parts(), &sue_actor));
        assert_eq!(
            review(&mut c, &sue, chart, p, &[p]).await,
            Err(SignOffRefusal::NothingToSign)
        );
        assert_eq!(vouch_count(&c).await, 1, "nothing was signed");
    }
);
