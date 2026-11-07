//! Sign-in slice 3c-3a, Task 5 — "— not co-signed" follows the SUPERVISION RECORD as it stood
//! when the part was signed, against a real database.
//!
//! On paper a probationer's entry waits for their supervisor's countersignature, and lifting the
//! probation later does not make yesterday's entries countersigned. Here the record is db/072's
//! registry rows (set up through the owner console door, `cairn_supervision_console_act`), read
//! with the notes (`note::supervision`), and the rules are `note::cosign`'s. Pinned here:
//! a requirement lifted after a part was signed still leaves that part waiting (Review Focus 1);
//! while the writer has a supervisor, only that supervisor's sign-off clears the mark (Review
//! Focus 2); and a student with no supervisor recorded is cleared by any clinician, as before
//! this slice (Review Focus 3).
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
use cairn_node::note::sign::{sign_note, SignNoteInput};
use cairn_node::note::view::part_byline;
use cairn_node::note::vouch::{needs_sign_off, sign_off_note, SignOffInput};
use common::note_setup;
use note_vouch_kit::{member, timer_part_by, Member, ORIGIN};
use tokio_postgres::Client;
use uuid::Uuid;

/// The node's key and a chart, from the note fixture.
async fn ward(c: &Client) -> (SigningKey, Uuid) {
    let (node_sk, _, _, _, chart) = note_setup(c).await;
    (node_sk, chart)
}

/// One supervision act through the owner's console door: `op` on `subject`, naming `supervisor`
/// for a link op (NULL otherwise).
async fn console(c: &Client, op: &str, subject: &str, supervisor: Option<&str>) {
    c.execute(
        "SELECT cairn_supervision_console_act($1, $2, $3)",
        &[&op, &subject, &supervisor],
    )
    .await
    .unwrap_or_else(|e| panic!("{op} {subject}: {}", common::db_msg(&e)));
}

/// `m` signs their own one-part note on `chart`. Returns its id.
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

/// `m` presses "Sign off this note" on the one-part note `p`.
async fn sign_off(c: &mut Client, m: &Member, chart: Uuid, p: Uuid) {
    let input = SignOffInput {
        patient: chart,
        root: p,
        shown_parts: &[p],
        node_origin: ORIGIN,
    };
    sign_off_note(c, &input, &m.author())
        .await
        .unwrap()
        .expect("the sign-off writes a vouch");
}

/// The compound `root` as the chart reads it.
async fn note(c: &Client, chart: Uuid, root: Uuid) -> CompoundNote {
    let notes = list_patient_notes(c, chart).await.unwrap().notes;
    compounds(notes)
        .into_iter()
        .find(|n| n.root() == root)
        .expect("the note is read")
}

/// The byline of the one-part note `p`.
async fn byline(c: &Client, chart: Uuid, p: Uuid) -> String {
    part_byline(&note(c, chart, p).await.parts()[0]).shown
}

/// Let the clock pass a millisecond boundary: the record and the parts are compared at
/// millisecond granularity, and an act in the same millisecond counts as already in effect.
async fn next_millisecond() {
    tokio::time::sleep(std::time::Duration::from_millis(3)).await;
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

// Review Focus 1: required → signed → lifted. The part was signed under the requirement, so it
// still waits; lifting changes only what is signed from now on.
db_test!(a_lifted_requirement_leaves_the_earlier_part_waiting, |c| {
    let (node_sk, chart) = ward(&c).await;
    let bea = member(&c, "bea", "Dr Bea", "clinician").await;
    console(&c, "require-cosign", "bea", None).await;
    let p = sign_as(&mut c, &node_sk, chart, &bea).await;
    next_millisecond().await;
    console(&c, "lift-cosign", "bea", None).await;

    assert_eq!(
        byline(&c, chart, p).await,
        "signed by Dr Bea — not co-signed (no supervisor recorded)"
    );
    assert!(needs_sign_off(note(&c, chart, p).await.parts()));

    // A part signed after the lift owes nothing.
    next_millisecond().await;
    let later = sign_as(&mut c, &node_sk, chart, &bea).await;
    assert_eq!(byline(&c, chart, later).await, "signed by Dr Bea");
    assert!(!needs_sign_off(note(&c, chart, later).await.parts()));
});

// Review Focus 2: with a supervisor linked, a colleague's co-sign leaves the mark; the
// supervisor's clears it.
db_test!(only_a_supervisor_clears_a_mandatory_co_sign, |c| {
    let (node_sk, chart) = ward(&c).await;
    let bea = member(&c, "bea", "Dr Bea", "clinician").await;
    let sue = member(&c, "sue", "Dr Sue", "registrar").await;
    let zed = member(&c, "zed", "Dr Zed", "registrar").await;
    console(&c, "require-cosign", "bea", None).await;
    console(&c, "link-supervisor", "bea", Some("sue")).await;
    let p = sign_as(&mut c, &node_sk, chart, &bea).await;
    assert_eq!(
        byline(&c, chart, p).await,
        "signed by Dr Bea — not co-signed"
    );

    sign_off(&mut c, &zed, chart, p).await;
    assert_eq!(
        byline(&c, chart, p).await,
        "signed by Dr Bea — not co-signed · co-signed by Dr Zed"
    );
    assert!(needs_sign_off(note(&c, chart, p).await.parts()));

    sign_off(&mut c, &sue, chart, p).await;
    assert_eq!(
        byline(&c, chart, p).await,
        "signed by Dr Bea · co-signed by Dr Zed · co-signed by Dr Sue"
    );
    assert!(!needs_sign_off(note(&c, chart, p).await.parts()));
});

// Review Focus 3: a student with no supervisor recorded (every part signed before this slice)
// is cleared by any clinician's co-sign, as before; until then it says no supervisor is recorded.
db_test!(a_pre_slice_students_part_is_cleared_by_any_clinician, |c| {
    let (node_sk, chart) = ward(&c).await;
    let sam = member(&c, "sam", "Sam", "student").await;
    let zed = member(&c, "zed", "Dr Zed", "registrar").await;
    let p = sign_as(&mut c, &node_sk, chart, &sam).await;
    assert_eq!(
        byline(&c, chart, p).await,
        "signed by Sam (student) — not co-signed (no supervisor recorded)"
    );
    assert!(needs_sign_off(note(&c, chart, p).await.parts()));

    sign_off(&mut c, &zed, chart, p).await;
    assert_eq!(
        byline(&c, chart, p).await,
        "signed by Sam (student) · co-signed by Dr Zed"
    );
    assert!(!needs_sign_off(note(&c, chart, p).await.parts()));
});

// The fallback accepts the strongest countersignature the floor allows on the part. A part the
// quarantine timer committed went in under the NODE's key, so the floor admits only an ATTESTED
// sign-off on it (db/071). A student's such
// part, signed later by the student (their addendum's vouch), is cleared by a clinician's "Sign
// off this note" when no supervisor is recorded; otherwise it would wait forever.
db_test!(
    a_part_gone_in_unsigned_is_cleared_by_a_clinicians_sign_off,
    |c| {
        let (node_sk, chart) = ward(&c).await;
        let sam = member(&c, "sam", "Sam", "student").await;
        let zed = member(&c, "zed", "Dr Zed", "registrar").await;
        let p = timer_part_by(&mut c, &node_sk, chart, "sam", None).await;
        let input = SignNoteInput {
            draft: Uuid::now_v7(),
            patient: chart,
            terminal: "ward-3-pc",
            writer: Some(sam.staff),
            narrative: "reviewed, obs stable",
            stated: None,
            node_origin: ORIGIN,
            addendum_to: Some(p),
            also_vouches: &[p],
        };
        sign_note(&mut c, &node_sk, &input, &sam.author())
            .await
            .unwrap()
            .expect("Sam's addendum signs and vouches the earlier part");
        let before = byline(&c, chart, p).await;
        assert!(
            before.ends_with(
                " · signed later by Sam (student) — not co-signed (no supervisor recorded)"
            ),
            "{before}"
        );
        let shown: Vec<Uuid> = note(&c, chart, p)
            .await
            .parts()
            .iter()
            .map(|q| q.event_id)
            .collect();
        assert_eq!(shown.len(), 2, "the part and Sam's addendum");
        assert!(needs_sign_off(note(&c, chart, p).await.parts()));

        let input = SignOffInput {
            patient: chart,
            root: p,
            shown_parts: &shown,
            node_origin: ORIGIN,
        };
        sign_off_note(&mut c, &input, &zed.author())
            .await
            .unwrap()
            .expect("Zed signs the note off");
        let after = byline(&c, chart, p).await;
        assert!(
            after.ends_with(" · signed later by Sam (student) · signed off by Dr Zed"),
            "{after}"
        );
        let n = note(&c, chart, p).await;
        assert!(
            !n.parts()[1..]
                .iter()
                .any(|q| part_byline(q).shown.contains("not co-signed")),
            "Sam's addendum is co-signed too"
        );
        assert!(!needs_sign_off(n.parts()));
    }
);
