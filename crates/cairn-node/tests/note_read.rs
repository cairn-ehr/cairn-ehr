//! Sign-in slice 3a, Task 4 — reading a chart's notes (`cairn_node::note::read::list_patient_notes`).
//!
//! Linked charts read as one, as the medication list does (ADR-0076), but a note on ANOTHER chart
//! of a set holding a doubted link may be a stranger's: it is counted and withheld, never shown.
//! And a note's authorship is graded, never assumed: a note signed by the node alone says the
//! writer is not known.
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard` taken BEFORE connecting.
mod common;

use cairn_event::note::{EscapedNarrative, NoteAssertion, StatedTime};
use cairn_event::SigningKey;
use cairn_node::db;
use cairn_node::medication::sealed_submit::seal_sign_submit;
use cairn_node::medication::AuthorParams;
use cairn_node::note::draft::save_part;
use cairn_node::note::read::list_patient_notes;
use cairn_node::note::sign::{note_body, sign_note, SignNoteInput};
use cairn_node::note::view::{byline_words, grade_words, NoteGrade, PartWriter};
use tokio_postgres::Client;
use uuid::Uuid;

const ORIGIN: &str = "note-read-test-node";

/// Save and sign one note on `chart` as the human author.
async fn sign_on(
    c: &mut Client,
    node_sk: &SigningKey,
    author: &AuthorParams<'_>,
    chart: Uuid,
    text: &str,
) {
    let draft = Uuid::now_v7();
    save_part(c, draft, chart, "pc-1", None, text, None)
        .await
        .unwrap();
    let input = SignNoteInput {
        draft,
        patient: chart,
        terminal: "pc-1",
        writer: None,
        narrative: text,
        stated: None,
        node_origin: ORIGIN,
    };
    sign_note(c, node_sk, &input, author)
        .await
        .unwrap()
        .expect("signs");
}

/// Flag the standing a-b link as db/018 would for an un-attested link that trips the veto
/// (copied from `doubted_link_withholds.rs`, whose lifecycle `link_veto_floor.rs` pins).
async fn flag_link(c: &Client, a: Uuid, b: Uuid) {
    let (lo, hi) = (a.min(b), a.max(b));
    c.execute(
        "INSERT INTO link_veto_flag (low, high, content_address) \
         SELECT low, high, content_address FROM patient_link \
         WHERE low = $1::text::uuid AND high = $2::text::uuid",
        &[&lo.to_string(), &hi.to_string()],
    )
    .await
    .unwrap();
}

fn texts(list: &cairn_node::note::read::PatientNoteList) -> Vec<&str> {
    list.notes.iter().map(|n| n.text.as_str()).collect()
}

#[tokio::test]
async fn a_doubted_set_withholds_notes_on_the_other_chart() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (node_sk, node_kid, human_sk, human_kid, a) = common::note_setup(&c).await;
    let author = AuthorParams {
        human_sk: &human_sk,
        human_kid: &human_kid,
    };
    let x = Uuid::now_v7();
    common::submit_registration(&c, &node_sk, &node_kid, x, 0).await;
    sign_on(&mut c, &node_sk, &author, a, "note on A").await;
    sign_on(&mut c, &node_sk, &author, x, "note on X").await;
    common::submit_link_event(&c, &node_sk, &node_kid, a, x, 10, true).await;

    // Positive control: an undoubted link reads as one record.
    let before = list_patient_notes(&c, a).await.unwrap();
    let mut both = texts(&before);
    both.sort();
    assert_eq!(both, vec!["note on A", "note on X"]);
    assert_eq!(before.withheld_on_linked, 0);

    flag_link(&c, a, x).await;
    let list = list_patient_notes(&c, a).await.unwrap();
    assert_eq!(texts(&list), vec!["note on A"], "X's note is withheld");
    assert_eq!(list.withheld_on_linked, 1, "and counted");

    let mirrored = list_patient_notes(&c, x).await.unwrap();
    assert_eq!(texts(&mirrored), vec!["note on X"], "the rule mirrors");
    assert_eq!(mirrored.withheld_on_linked, 1);
}

/// A note whose only contributor is the node (`recorded`, device-additive — the shape a peer's
/// node-only note has) is graded "writer not known", never "signed by" anyone.
#[tokio::test]
async fn a_peer_note_signed_by_the_node_alone_reads_writer_not_known() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (node_sk, node_kid, _, _, chart) = common::note_setup(&c).await;
    let a = NoteAssertion {
        narrative: EscapedNarrative::of_typed("transferred from the district hospital"),
        stated: StatedTime::NotStated,
        recording_started: "2026-11-02T22:00:00Z",
        recording_ended: "2026-11-02T22:04:00Z",
        addendum_to: None,
    };
    let hlc = db::next_hlc(&c, ORIGIN).await.unwrap();
    seal_sign_submit(
        &mut c,
        &node_sk,
        note_body(Uuid::now_v7(), chart, &node_kid, hlc, &a),
        None,
        None,
    )
    .await
    .unwrap();

    let list = list_patient_notes(&c, chart).await.unwrap();
    assert_eq!(list.notes.len(), 1);
    assert_eq!(list.notes[0].grade, NoteGrade::WriterNotKnown);
    assert_eq!(grade_words(&list.notes[0].grade), "writer not known");
}

/// I2: a part signed under "Don't know" is `{signer, attested}` with no writer, and must never
/// read like the signer's own authored note: its byline says the writer is not known.
#[tokio::test]
async fn an_unknown_writers_part_reads_writer_not_known_signed_by() {
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
    sign_on(&mut c, &node_sk, &author, chart, "found on the floor").await;

    let note = &list_patient_notes(&c, chart).await.unwrap().notes[0];
    assert_eq!(note.writer, PartWriter::NotKnown);
    assert_eq!(
        byline_words(&note.grade, &note.writer).shown,
        format!("writer not known · signed by {}", common::NOTE_AUTHOR)
    );
}

/// The window reads notes as `cairn_window`, not as the schema owner (ruling R11). The vouch
/// predicate is closed to that role, so the read must reach it through db/065's definer door;
/// calling `cairn_attestation_vouched` directly was a 42501 under the window's login.
#[tokio::test]
async fn the_window_role_reads_a_charts_notes() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let (mut owner, window) = common::owner_and_window(&base).await;
    let (node_sk, _, human_sk, human_kid, chart) = common::note_setup(&owner).await;
    let author = AuthorParams {
        human_sk: &human_sk,
        human_kid: &human_kid,
    };
    sign_on(
        &mut owner,
        &node_sk,
        &author,
        chart,
        "seen on the ward round",
    )
    .await;

    let list = list_patient_notes(&window, chart).await;
    common::assert_not_denied("list_patient_notes", &list);
    let list = list.unwrap();
    assert_eq!(texts(&list), vec!["seen on the ward round"]);
    assert_eq!(
        list.notes[0].grade,
        NoteGrade::Signed {
            by: common::NOTE_AUTHOR.into()
        }
    );
}

/// Task 4 review, Important #1: the floor reads a ref's event as `::uuid`, which admits spellings
/// Rust's `Uuid` parser does not (hyphens after every four digits). One such addendum, from a peer
/// or a raw-SQL client, must not make the chart's whole note list unreadable: the read takes the
/// id in Postgres's canonical form.
#[tokio::test]
async fn an_addendum_ref_in_a_non_canonical_spelling_still_reads() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (node_sk, node_kid, _, _, chart) = common::note_setup(&c).await;
    let note = |addendum_to| NoteAssertion {
        narrative: EscapedNarrative::of_typed("seen"),
        stated: StatedTime::NotStated,
        recording_started: "2026-11-02T22:00:00Z",
        recording_ended: "2026-11-02T22:04:00Z",
        addendum_to,
    };
    let first = Uuid::now_v7();
    let hlc = db::next_hlc(&c, ORIGIN).await.unwrap();
    let body = note_body(first, chart, &node_kid, hlc, &note(None));
    seal_sign_submit(&mut c, &node_sk, body, None, None)
        .await
        .unwrap();
    // `a0ee-bc99-…`: Postgres reads it as a uuid, `Uuid::parse_str` refuses it.
    let simple = first.simple().to_string();
    let odd: String = simple
        .as_bytes()
        .chunks(4)
        .map(|g| std::str::from_utf8(g).unwrap())
        .collect::<Vec<_>>()
        .join("-");
    assert!(odd.parse::<Uuid>().is_err(), "the spelling Rust refuses");
    let hlc = db::next_hlc(&c, ORIGIN).await.unwrap();
    let body = note_body(Uuid::now_v7(), chart, &node_kid, hlc, &note(Some(&odd)));
    seal_sign_submit(&mut c, &node_sk, body, None, None)
        .await
        .expect("the floor admits the spelling");

    let list = list_patient_notes(&c, chart)
        .await
        .expect("one odd ref must not break the chart's notes");
    assert_eq!(list.notes.len(), 2);
    let addenda: Vec<_> = list.notes.iter().filter_map(|n| n.addendum_to).collect();
    assert_eq!(addenda, [first], "the ref reads as the canonical id");
}
