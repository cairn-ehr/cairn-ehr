//! Sign-in slice 3c-1, Task 4 — Sign, the quarantine timer and the drafts read carry an
//! addendum's target (`cairn_node::note::{sign, commit, draft}`).
//!
//! WHY: "Add to this note" starts a draft whose HEADER remembers the note it adds to (db/070's
//! `note_draft.addendum_to`). Whoever turns that draft into events — the clinician's Sign, or the
//! timer when the writer walked away — must put that target on the FIRST part's `addendum-to`,
//! read from the header (never from the window), while later parts keep chaining to the part
//! before. These tests pin: the signed first part names the root; a Sign pressed before any save
//! (inside the window's debounce) still makes an addendum; a two-writer addendum chains part 2 to
//! part 1; the timer commits an addendum as one; two addenda signed at once both join the one
//! compound; a target on another chart refuses the Sign with nothing written; and the drafts list
//! carries the target.
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard` taken BEFORE connecting.
//! Key material is minted at runtime (house rule 6).
mod common;

use cairn_event::generate_key;
use cairn_event::SigningKey;
use cairn_node::db;
use cairn_node::medication::AuthorParams;
use cairn_node::note::commit::{commit_overdue, CommitOutcome};
use cairn_node::note::compound::compounds;
use cairn_node::note::draft::{open_drafts, save_part, SaveOutcome};
use cairn_node::note::read::list_patient_notes;
use cairn_node::note::sign::{sign_note, SignNoteInput, SignRefusal};
use tokio_postgres::Client;
use uuid::Uuid;

const ORIGIN: &str = "note-addendum-sign-test-node";
const TERMINAL: &str = "ward-3-pc";

/// The input the window sends with Sign, by the signer (`notewriter`), adding to `target`
/// (`None` = a new note).
fn input<'a>(
    draft: Uuid,
    patient: Uuid,
    narrative: &'a str,
    target: Option<Uuid>,
) -> SignNoteInput<'a> {
    SignNoteInput {
        draft,
        patient,
        terminal: TERMINAL,
        writer: Some("notewriter"),
        narrative,
        stated: None,
        node_origin: ORIGIN,
        addendum_to: target,
    }
}

/// Save `text` as `writer`'s part of `draft` on `chart`, adding to `target`, as the window's
/// autosave would; asserts it saved.
async fn save_as(
    c: &Client,
    draft: Uuid,
    chart: Uuid,
    writer: Option<&str>,
    text: &str,
    target: Option<Uuid>,
) {
    let saved = save_part(c, draft, chart, TERMINAL, writer, text, None, target)
        .await
        .unwrap();
    assert_eq!(saved, SaveOutcome::Saved, "{writer:?} saves {text:?}");
}

/// Sign a plain one-part note on `chart` by the signer: the root every addendum below adds to.
async fn root_note(
    c: &mut Client,
    node_sk: &SigningKey,
    author: &AuthorParams<'_>,
    chart: Uuid,
) -> Uuid {
    let draft = Uuid::now_v7();
    sign_note(
        c,
        node_sk,
        &input(draft, chart, "chest clear", None),
        author,
    )
    .await
    .unwrap()
    .expect("the root note signs")
    .event_id
}

/// A note's `refs`, as projected.
async fn refs(c: &Client, event: Uuid) -> serde_json::Value {
    let r = c
        .query_one(
            "SELECT refs::text FROM clinical_note WHERE event_id = $1::text::uuid",
            &[&event.to_string()],
        )
        .await
        .unwrap();
    serde_json::from_str(&r.get::<_, String>(0)).unwrap()
}

fn addendum_to(target: Uuid) -> serde_json::Value {
    serde_json::json!([{"rel": "addendum-to", "event": target.to_string()}])
}

async fn note_count(c: &Client, patient: Uuid) -> i64 {
    c.query_one(
        "SELECT count(*) FROM clinical_note WHERE patient_id = $1::text::uuid",
        &[&patient.to_string()],
    )
    .await
    .unwrap()
    .get(0)
}

/// Enrol `Nurse A` (`nurse-a`) as a vault member with a fresh key, as `note_sign_parts.rs`
/// does.
async fn enrol_nurse_a(c: &Client) {
    let (_sk, kid): (SigningKey, String) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('human', '{\"role\":\"clinician\",\"handle\":\"nurse-a\"}', $1)",
        &[&kid],
    )
    .await
    .unwrap();
    c.execute(
        "INSERT INTO vault_member (staff_id, display_name, role, actor_id, signing_key_id, \
                                   sealed, enrolled_by, contact_email) \
         SELECT 'nurse-a', 'Nurse A', 'nurse', actor_id, $1, '\\x00'::bytea, 'console', \
                'a@example.org' \
           FROM actor_event WHERE signing_key_id = $1 AND op = 'enroll' LIMIT 1",
        &[&kid],
    )
    .await
    .unwrap();
}

/// Make `draft` as if typed `minutes` earlier (header and parts together), as `note_commit.rs`
/// does: "overdue" is made by moving time back, never by sleeping.
async fn age(c: &Client, draft: Uuid, minutes: i32) {
    c.execute(
        "WITH h AS (UPDATE note_draft SET last_keystroke = last_keystroke - make_interval(mins => $2), \
                           recording_started = recording_started - make_interval(mins => $2) \
                     WHERE draft_id = $1::text::uuid) \
         UPDATE note_draft_part SET last_keystroke = last_keystroke - make_interval(mins => $2), \
                recording_started = recording_started - make_interval(mins => $2) \
          WHERE draft_id = $1::text::uuid",
        &[&draft.to_string(), &minutes],
    )
    .await
    .unwrap();
}

/// The shared start of every test, as a macro so its bindings (the serial guard, the client, the
/// keys the borrowed `AuthorParams` points into) live in the test's own scope: connect (or skip
/// when no DB is configured), set up a chart and its signer, and sign one root note on it. Binds
/// the given names to the connection string, client, node key, node key id, author, chart and
/// root; pass `_name` for one a test does not use.
macro_rules! setup {
    ($base:ident, $c:ident, $node_sk:ident, $node_kid:ident, $author:ident, $chart:ident, $root:ident) => {
        let Some($base) = common::cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let _guard = db::test_serial_guard(&$base).await.unwrap();
        let mut $c = db::connect_and_load_schema(&$base).await.unwrap();
        let ($node_sk, $node_kid, human_sk, human_kid, $chart) = common::note_setup(&$c).await;
        let $author = AuthorParams {
            human_sk: &human_sk,
            human_kid: &human_kid,
        };
        let $root = root_note(&mut $c, &$node_sk, &$author, $chart).await;
    };
}

#[tokio::test]
async fn signing_an_addendum_names_the_root() {
    setup!(_base, c, node_sk, _node_kid, author, chart, root);
    let draft = Uuid::now_v7();
    save_as(
        &c,
        draft,
        chart,
        Some("notewriter"),
        "BP now 120/80",
        Some(root),
    )
    .await;

    let ids = sign_note(
        &mut c,
        &node_sk,
        &input(draft, chart, "BP now 120/80", Some(root)),
        &author,
    )
    .await
    .unwrap()
    .expect("the addendum signs")
    .value;

    assert_eq!(ids.len(), 1);
    assert_eq!(refs(&c, ids[0]).await, addendum_to(root));
    assert_eq!(
        refs(&c, root).await,
        serde_json::json!([]),
        "the root is untouched"
    );
}

/// Sign takes the target from the draft's HEADER, never from the window: a draft made as an
/// addendum, signed by a window that sends no target (db/066's door, which keeps the header),
/// still signs as an addendum to the root.
#[tokio::test]
async fn sign_takes_the_target_from_the_header_not_the_window() {
    setup!(_base, c, node_sk, _node_kid, author, chart, root);
    let draft = Uuid::now_v7();
    save_as(&c, draft, chart, Some("notewriter"), "adding", Some(root)).await;

    let ids = sign_note(
        &mut c,
        &node_sk,
        &input(draft, chart, "adding", None),
        &author,
    )
    .await
    .unwrap()
    .expect("the addendum signs")
    .value;

    assert_eq!(refs(&c, ids[0]).await, addendum_to(root));
}

/// Review Focus 2: Sign pressed inside the window's debounce, before any save made the header.
/// Sign's own save must create the ADDENDUM header, so the event is an addendum, not a new note.
#[tokio::test]
async fn sign_inside_the_debounce_still_makes_an_addendum() {
    setup!(_base, c, node_sk, _node_kid, author, chart, root);
    let draft = Uuid::now_v7();

    let ids = sign_note(
        &mut c,
        &node_sk,
        &input(draft, chart, "pulses present", Some(root)),
        &author,
    )
    .await
    .unwrap()
    .expect("the addendum signs")
    .value;

    assert_eq!(refs(&c, ids[0]).await, addendum_to(root));
}

/// Only the FIRST part names the root; part 2 chains to part 1, as every multi-writer draft does.
#[tokio::test]
async fn a_two_writer_addendum_chains_part_two_to_part_one() {
    setup!(_base, c, node_sk, _node_kid, author, chart, root);
    enrol_nurse_a(&c).await;
    let draft = Uuid::now_v7();
    save_as(&c, draft, chart, Some("nurse-a"), "obs stable", Some(root)).await;

    let ids = sign_note(
        &mut c,
        &node_sk,
        &input(draft, chart, "plan: review", Some(root)),
        &author,
    )
    .await
    .unwrap()
    .expect("the addendum signs")
    .value;

    let [first, second] = ids[..] else {
        panic!("two parts, two events: {ids:?}");
    };
    assert_eq!(refs(&c, first).await, addendum_to(root));
    assert_eq!(refs(&c, second).await, addendum_to(first));
}

/// Review Focus 3: the writer walked away; the timer's first part names the root as a Sign's
/// would — read from the header, which the commit then deletes.
#[tokio::test]
async fn the_timer_commits_an_addendum_as_an_addendum() {
    setup!(_base, c, node_sk, _node_kid, _author, chart, root);
    let draft = Uuid::now_v7();
    save_as(
        &c,
        draft,
        chart,
        Some("notewriter"),
        "left without signing",
        Some(root),
    )
    .await;
    age(&c, draft, 181).await;

    let ids = match commit_overdue(&mut c, &node_sk, draft, ORIGIN)
        .await
        .unwrap()
    {
        CommitOutcome::Committed(done) => done.value,
        other => panic!("expected Committed, got {other:?}"),
    };

    assert_eq!(refs(&c, ids[0]).await, addendum_to(root));
}

/// The timer's chain is Sign's: only the FIRST part of a two-writer addendum names the root, and
/// part 2 chains to part 1 (ADR-0088 decision 14). `commit.rs` repeats Sign's
/// `ids.last().or(root)` and nothing else pins it; a swap would make every part name the root,
/// which the compound would still group, so only the wire shows it (PR #992 review).
#[tokio::test]
async fn a_two_writer_addendum_committed_by_the_timer_chains_part_two_to_part_one() {
    setup!(_base, c, node_sk, _node_kid, _author, chart, root);
    enrol_nurse_a(&c).await;
    let draft = Uuid::now_v7();
    save_as(&c, draft, chart, Some("nurse-a"), "obs stable", Some(root)).await;
    save_as(
        &c,
        draft,
        chart,
        Some("notewriter"),
        "plan: review",
        Some(root),
    )
    .await;
    age(&c, draft, 181).await;

    let ids = match commit_overdue(&mut c, &node_sk, draft, ORIGIN)
        .await
        .unwrap()
    {
        CommitOutcome::Committed(done) => done.value,
        other => panic!("expected Committed, got {other:?}"),
    };

    let [first, second] = ids[..] else {
        panic!("two parts, two events: {ids:?}");
    };
    assert_eq!(refs(&c, first).await, addendum_to(root));
    assert_eq!(refs(&c, second).await, addendum_to(first));
}

/// Review Focus 1: two clinicians add to the same note at once, on two terminals (two drafts, two
/// connections). Both name the root and both land inside the ONE compound, three parts.
#[tokio::test]
async fn two_addenda_signed_at_once_both_join_the_note() {
    setup!(base, c, node_sk, _node_kid, author, chart, root);
    let mut other = db::connect(&base).await.unwrap();
    let (one, two) = (Uuid::now_v7(), Uuid::now_v7());
    let in_one = input(one, chart, "first addendum", Some(root));
    let in_two = input(two, chart, "second addendum", Some(root));

    let (x, y) = tokio::join!(
        sign_note(&mut c, &node_sk, &in_one, &author),
        sign_note(&mut other, &node_sk, &in_two, &author)
    );

    for out in [x, y] {
        let ids = out.unwrap().expect("each addendum signs").value;
        assert_eq!(refs(&c, ids[0]).await, addendum_to(root));
    }
    let read = list_patient_notes(&c, chart).await.unwrap();
    let grouped = compounds(read.notes);
    assert_eq!(grouped.len(), 1, "{grouped:?}");
    assert_eq!(grouped[0].root(), root);
    assert_eq!(grouped[0].parts().len(), 3);
}

/// A target that is not a note on this chart (here: another chart's note) refuses the Sign
/// `NoSuchNote`, and nothing is written — no note, no draft.
#[tokio::test]
async fn an_addendum_target_on_another_chart_refuses_the_sign() {
    setup!(_base, c, node_sk, node_kid, author, chart, _root);
    let x = Uuid::now_v7();
    common::submit_registration(&c, &node_sk, &node_kid, x, 5).await;
    let elsewhere = root_note(&mut c, &node_sk, &author, x).await;
    let before = note_count(&c, chart).await;
    let draft = Uuid::now_v7();

    let out = sign_note(
        &mut c,
        &node_sk,
        &input(draft, chart, "wrong chart", Some(elsewhere)),
        &author,
    )
    .await
    .unwrap();

    assert_eq!(out.err(), Some(SignRefusal::NoSuchNote));
    assert_eq!(note_count(&c, chart).await, before);
    assert!(open_drafts(&c, chart).await.unwrap().is_empty());
}

/// The drafts list says what each draft adds to: the root for an addendum, `None` for a new note.
#[tokio::test]
async fn the_drafts_list_carries_the_target() {
    setup!(_base, c, _node_sk, _node_kid, _author, chart, root);
    let (addendum, fresh) = (Uuid::now_v7(), Uuid::now_v7());
    save_as(
        &c,
        addendum,
        chart,
        Some("notewriter"),
        "adding",
        Some(root),
    )
    .await;
    save_as(&c, fresh, chart, Some("notewriter"), "a new note", None).await;

    let drafts = open_drafts(&c, chart).await.unwrap();

    let target_of = |id: Uuid| {
        drafts
            .iter()
            .find(|d| d.draft_id == id)
            .unwrap()
            .addendum_to
    };
    assert_eq!(target_of(addendum), Some(root));
    assert_eq!(target_of(fresh), None);
}
