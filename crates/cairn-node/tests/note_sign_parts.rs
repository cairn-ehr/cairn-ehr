//! Sign-in slice 3b-1, Task 4 — a draft signs as ONE EVENT PER WRITER'S PART
//! (`cairn_node::note::sign::sign_note`).
//!
//! Several people may write into one draft; signing it makes one note event per non-empty part,
//! in one transaction, each saying who wrote it: the signer's own part `{signer, authored}`,
//! anyone else's `{writer, drafted} + {signer, attested}`, a part by a writer not known
//! `{signer, attested}` alone. Part k > 1 is an addendum to the part signed before it. These
//! tests pin those sets, the chain, that a refused later part takes the earlier ones with it, and
//! the refusals (`Empty`, `Moved`, `NoSuchWriter`) that write nothing. 3a's single-part cases
//! stay in `note_sign.rs`.
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard` taken BEFORE connecting.
//! Key material is minted at runtime (house rule 6).
mod common;

use cairn_event::generate_key;
use cairn_event::SigningKey;
use cairn_node::db;
use cairn_node::medication::AuthorParams;
use cairn_node::note::draft::{open_drafts, save_part, take_draft, SaveOutcome, TakeOutcome};
use cairn_node::note::read::list_patient_notes;
use cairn_node::note::sign::{sign_note, SignNoteInput, SignRefusal};
use cairn_node::note::view::{NoteGrade, PartWriter};
use tokio_postgres::Client;
use uuid::Uuid;

const ORIGIN: &str = "note-sign-parts-test-node";
const TERMINAL: &str = "ward-3-pc";

/// The input the window sends with Sign, for a writer not known; a test names one by setting
/// `writer`.
fn input(draft: Uuid, patient: Uuid, narrative: &str) -> SignNoteInput<'_> {
    SignNoteInput {
        draft,
        patient,
        terminal: TERMINAL,
        writer: None,
        narrative,
        stated: None,
        node_origin: ORIGIN,
    }
}

/// Notes projected for one chart.
async fn note_count(c: &Client, patient: Uuid) -> i64 {
    c.query_one(
        "SELECT count(*) FROM clinical_note WHERE patient_id = $1::text::uuid",
        &[&patient.to_string()],
    )
    .await
    .unwrap()
    .get(0)
}

/// A second clinical member, `Nurse A` (`nurse-a`), enrolled as `note_setup` enrols its author: an
/// actor for a fresh key (so `cairn_note_signer_name` can name it) and its vault row. Returns the
/// member's key id.
async fn enrol_nurse_a(c: &Client) -> String {
    let (_sk, kid): (SigningKey, String) = generate_key().unwrap();
    c.execute(
        // A handle of her own: the actor's id derives from its pinned determinants.
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
    kid
}

/// Save `text` as `writer`'s part of `draft` on `chart`, as the window's autosave would.
async fn save_as(c: &Client, draft: Uuid, chart: Uuid, writer: Option<&str>, text: &str) {
    let saved = save_part(c, draft, chart, TERMINAL, writer, text, None)
        .await
        .unwrap();
    assert_eq!(saved, SaveOutcome::Saved, "{writer:?} saves {text:?}");
}

/// One signed event's contributor set and its note's `refs`, as stored.
async fn stored(c: &Client, event: Uuid) -> (serde_json::Value, serde_json::Value) {
    let r = c
        .query_one(
            "SELECT e.contributors::text, n.refs::text FROM event_log e \
               JOIN clinical_note n ON n.event_id = e.event_id WHERE e.event_id = $1::text::uuid",
            &[&event.to_string()],
        )
        .await
        .unwrap();
    (
        serde_json::from_str(&r.get::<_, String>(0)).unwrap(),
        serde_json::from_str(&r.get::<_, String>(1)).unwrap(),
    )
}

fn contributor(kid: &str, role: &str) -> serde_json::Value {
    serde_json::json!({"actor_id": kid, "role": role})
}

/// The signer's OWN part signs as 3a's did, `{signer, authored}`, never as a part they merely
/// attest (`{signer, drafted} + {signer, attested}`): "own" is decided by the writer's key.
#[tokio::test]
async fn a_one_writer_draft_signs_as_the_signers_own_authored_note() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (node_sk, node_kid, human_sk, human_kid, chart) = common::note_setup(&c).await;
    let author = AuthorParams {
        human_sk: &human_sk,
        human_kid: &human_kid,
    };
    let draft = Uuid::now_v7();
    save_as(&c, draft, chart, Some("notewriter"), "chest clear").await;
    let mut sent = input(draft, chart, "chest clear");
    // The door normalises case and spaces: still the signer's own part.
    sent.writer = Some(" NoteWriter ");

    let signed = sign_note(&mut c, &node_sk, &sent, &author)
        .await
        .unwrap()
        .expect("signs");

    assert_eq!(signed.value, vec![signed.event_id], "one part, one event");
    let (contributors, refs) = stored(&c, signed.event_id).await;
    assert_eq!(
        contributors,
        serde_json::json!([
            contributor(&human_kid, "authored"),
            contributor(&node_kid, "recorded")
        ])
    );
    assert_eq!(refs, serde_json::json!([]));
    let list = list_patient_notes(&c, chart).await.unwrap();
    assert_eq!(list.notes.len(), 1);
    assert_eq!(
        list.notes[0].grade,
        NoteGrade::Signed {
            by: common::NOTE_AUTHOR.into()
        }
    );
    assert_eq!(list.notes[0].writer, PartWriter::TheSigner);
    assert_eq!(list.notes[0].addendum_to, None);
}

#[tokio::test]
async fn a_two_writer_draft_signs_as_two_events_chained_by_addendum_to() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (node_sk, node_kid, human_sk, human_kid, chart) = common::note_setup(&c).await;
    let nurse_kid = enrol_nurse_a(&c).await;
    let author = AuthorParams {
        human_sk: &human_sk,
        human_kid: &human_kid,
    };
    let draft = Uuid::now_v7();
    save_as(&c, draft, chart, Some("nurse-a"), "obs stable").await;
    let mut sent = input(draft, chart, "plan: review in the morning");
    sent.writer = Some("notewriter");

    let signed = sign_note(&mut c, &node_sk, &sent, &author)
        .await
        .unwrap()
        .expect("signs");

    let [first, second] = signed.value[..] else {
        panic!("two parts, two events: {:?}", signed.value);
    };
    assert_eq!(signed.event_id, first, "the spend names part 1's event");
    let (c1, r1) = stored(&c, first).await;
    assert_eq!(
        c1,
        serde_json::json!([
            contributor(&nurse_kid, "drafted"),
            contributor(&human_kid, "attested"),
            contributor(&node_kid, "recorded")
        ]),
        "Nurse A's part: she drafted it, Dr Ada attests it"
    );
    assert_eq!(r1, serde_json::json!([]));
    let (c2, r2) = stored(&c, second).await;
    assert_eq!(
        c2,
        serde_json::json!([
            contributor(&human_kid, "authored"),
            contributor(&node_kid, "recorded")
        ])
    );
    assert_eq!(
        r2,
        serde_json::json!([{"rel": "addendum-to", "event": first.to_string()}])
    );

    let list = list_patient_notes(&c, chart).await.unwrap();
    assert_eq!(list.notes.len(), 2);
    let by_id = |id: Uuid| list.notes.iter().find(|n| n.event_id == id).unwrap();
    let signed_by = NoteGrade::Signed {
        by: common::NOTE_AUTHOR.into(),
    };
    let (n1, n2) = (by_id(first), by_id(second));
    assert_eq!((n1.text.as_str(), &n1.grade), ("obs stable", &signed_by));
    assert_eq!(n1.writer, PartWriter::Drafted("Nurse A".into()));
    assert_eq!(n1.addendum_to, None);
    assert_eq!(
        (n2.text.as_str(), &n2.grade),
        ("plan: review in the morning", &signed_by)
    );
    assert_eq!(n2.writer, PartWriter::TheSigner);
    assert_eq!(n2.addendum_to, Some(first));
    assert!(open_drafts(&c, chart).await.unwrap().is_empty());
}

#[tokio::test]
async fn an_unknown_writers_part_is_attested_by_the_signer() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (node_sk, node_kid, human_sk, human_kid, chart) = common::note_setup(&c).await;
    let author = AuthorParams {
        human_sk: &human_sk,
        human_kid: &human_kid,
    };
    let draft = Uuid::now_v7();
    save_as(&c, draft, chart, None, "found on the floor").await;

    let signed = sign_note(
        &mut c,
        &node_sk,
        &input(draft, chart, "found on the floor"),
        &author,
    )
    .await
    .unwrap()
    .expect("signs");

    let (contributors, _) = stored(&c, signed.event_id).await;
    assert_eq!(
        contributors,
        serde_json::json!([
            contributor(&human_kid, "attested"),
            contributor(&node_kid, "recorded")
        ]),
        "no writer is invented (principle 4)"
    );
    let note = &list_patient_notes(&c, chart).await.unwrap().notes[0];
    assert_eq!(
        note.grade,
        NoteGrade::Signed {
            by: common::NOTE_AUTHOR.into()
        }
    );
    assert_eq!(
        note.writer,
        PartWriter::NotKnown,
        "never read as the signer's own"
    );
}

/// Review Focus 5: the parts sign in ONE transaction. Part 2's stored times are made dishonest
/// (it began after it ended), so the floor refuses part 2 AFTER part 1 was submitted: nothing
/// may survive — no note, and the draft with both parts as they were.
#[tokio::test]
async fn a_refused_second_part_rolls_back_the_first() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (node_sk, _, human_sk, human_kid, chart) = common::note_setup(&c).await;
    enrol_nurse_a(&c).await;
    let author = AuthorParams {
        human_sk: &human_sk,
        human_kid: &human_kid,
    };
    let draft = Uuid::now_v7();
    save_as(&c, draft, chart, Some("nurse-a"), "obs stable").await;
    save_as(&c, draft, chart, Some("notewriter"), "plan").await;
    c.execute(
        "UPDATE note_draft_part SET recording_started = last_keystroke + interval '1 hour' \
          WHERE draft_id = $1::text::uuid AND seq = 2",
        &[&draft.to_string()],
    )
    .await
    .unwrap();
    // The same text, so Sign's own save is an idle re-save and leaves part 2's times alone.
    let mut sent = input(draft, chart, "plan");
    sent.writer = Some("notewriter");

    let e = sign_note(&mut c, &node_sk, &sent, &author)
        .await
        .expect_err("the floor refuses part 2");

    assert!(
        format!("{e:#}").contains("recording_started must not be after recording_ended"),
        "the refusal is part 2's: {e:#}"
    );
    assert_eq!(note_count(&c, chart).await, 0, "part 1 rolled back too");
    let drafts = open_drafts(&c, chart).await.unwrap();
    assert_eq!(drafts.len(), 1, "the draft is still open");
    let texts: Vec<_> = drafts[0]
        .parts
        .iter()
        .map(|p| p.narrative.as_str())
        .collect();
    assert_eq!(texts, ["obs stable", "plan"]);
}

/// A blank part signs nothing and the chain skips it: part 3 continues part 1. A draft with no
/// typed part at all is refused `Empty` (never `Spent`), and stays open.
#[tokio::test]
async fn empty_parts_are_skipped_and_an_all_empty_draft_is_refused_empty() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (node_sk, _, human_sk, human_kid, chart) = common::note_setup(&c).await;
    enrol_nurse_a(&c).await;
    let author = AuthorParams {
        human_sk: &human_sk,
        human_kid: &human_kid,
    };
    // Part 1 Nurse A's, part 2 Dr Ada's (typed, then cleared), part 3 Nurse A's again.
    let draft = Uuid::now_v7();
    save_as(&c, draft, chart, Some("nurse-a"), "obs stable").await;
    save_as(&c, draft, chart, Some("notewriter"), "plan").await;
    save_as(&c, draft, chart, Some("notewriter"), " \n ").await;
    save_as(&c, draft, chart, Some("nurse-a"), "obs at 04:00 stable").await;
    // Dr Ada signs with nothing typed below: a blank save by a new writer makes no part.
    let mut sent = input(draft, chart, "");
    sent.writer = Some("notewriter");

    let signed = sign_note(&mut c, &node_sk, &sent, &author)
        .await
        .unwrap()
        .expect("the typed parts sign");

    let [first, third] = signed.value[..] else {
        panic!("two typed parts, two events: {:?}", signed.value);
    };
    let (_, refs) = stored(&c, third).await;
    assert_eq!(
        refs,
        serde_json::json!([{"rel": "addendum-to", "event": first.to_string()}]),
        "the blank part 2 is skipped in the chain"
    );

    // All blank: refused Empty, and the draft stays open.
    let blank = Uuid::now_v7();
    save_as(&c, blank, chart, Some("nurse-a"), "  ").await;
    let mut sent = input(blank, chart, "\t");
    sent.writer = Some("nurse-a");
    let out = sign_note(&mut c, &node_sk, &sent, &author).await.unwrap();
    assert_eq!(out.err(), Some(SignRefusal::Empty));
    assert_eq!(
        note_count(&c, chart).await,
        2,
        "no note for the blank draft"
    );
    let open: Vec<_> = open_drafts(&c, chart)
        .await
        .unwrap()
        .into_iter()
        .map(|d| d.draft_id)
        .collect();
    assert_eq!(open, [blank], "the blank draft is not spent");
}

#[tokio::test]
async fn a_moved_draft_is_refused_moved_and_writes_nothing() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (node_sk, _, human_sk, human_kid, chart) = common::note_setup(&c).await;
    let author = AuthorParams {
        human_sk: &human_sk,
        human_kid: &human_kid,
    };
    let draft = Uuid::now_v7();
    save_as(&c, draft, chart, None, "begun at the ward PC").await;
    let taken = take_draft(&c, draft, chart, "theatre-pc").await.unwrap();
    assert_eq!(taken, TakeOutcome::Taken);

    let out = sign_note(
        &mut c,
        &node_sk,
        &input(draft, chart, "signed at the ward PC"),
        &author,
    )
    .await
    .unwrap();

    assert_eq!(out.err(), Some(SignRefusal::Moved));
    assert_eq!(note_count(&c, chart).await, 0);
    let drafts = open_drafts(&c, chart).await.unwrap();
    assert_eq!(drafts[0].terminal_id, "theatre-pc");
    assert_eq!(drafts[0].parts[0].narrative, "begun at the ward PC");
}

/// A named writer whose member row has gone (left the service) is refused `NoSuchWriter`:
/// their part is never signed as if its writer were unknown.
#[tokio::test]
async fn a_part_whose_writer_left_is_refused_no_such_writer() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (node_sk, _, human_sk, human_kid, chart) = common::note_setup(&c).await;
    enrol_nurse_a(&c).await;
    let author = AuthorParams {
        human_sk: &human_sk,
        human_kid: &human_kid,
    };
    let draft = Uuid::now_v7();
    save_as(&c, draft, chart, Some("nurse-a"), "obs stable").await;
    c.batch_execute(
        "DELETE FROM vault_attempt WHERE staff_id = 'nurse-a'; \
         DELETE FROM vault_member WHERE staff_id = 'nurse-a';",
    )
    .await
    .unwrap();
    let mut sent = input(draft, chart, "plan");
    sent.writer = Some("notewriter");

    let out = sign_note(&mut c, &node_sk, &sent, &author).await.unwrap();

    assert_eq!(out.err(), Some(SignRefusal::NoSuchWriter));
    assert_eq!(note_count(&c, chart).await, 0);
    let drafts = open_drafts(&c, chart).await.unwrap();
    let texts: Vec<_> = drafts[0]
        .parts
        .iter()
        .map(|p| p.narrative.as_str())
        .collect();
    assert_eq!(texts, ["obs stable"], "Sign's own save rolled back too");
}
