//! Sign-in slice 3a, Task 4 — signing a progress note (`cairn_node::note::sign::sign_note`).
//!
//! A note is signed in ONE transaction that saves the text sent with Sign into its draft, spends
//! the draft, builds the body from the spent row, and submits it through the strict door. These
//! tests pin what a clinician relies on: the signed note is the text they sent (not the last
//! autosave), a draft is spent exactly once, a refused sign leaves their draft untouched, and an
//! unstated time is recorded honestly as "writing began" (principle 4).
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard` taken BEFORE connecting.
//! Key material is minted at runtime (house rule 6).
mod common;

use cairn_event::generate_key;
use cairn_event::SigningKey;
use cairn_node::db;
use cairn_node::medication::AuthorParams;
use cairn_node::note::draft::{open_drafts, save_draft, SaveOutcome};
use cairn_node::note::read::list_patient_notes;
use cairn_node::note::sign::{sign_note, SignNoteInput, SignRefusal};
use cairn_node::note::view::NoteGrade;
use tokio_postgres::Client;
use uuid::Uuid;

const ORIGIN: &str = "note-sign-test-node";
const TERMINAL: &str = "ward-3-pc";

/// The input the window sends with Sign.
fn input(draft: Uuid, patient: Uuid, narrative: &str) -> SignNoteInput<'_> {
    SignNoteInput {
        draft,
        patient,
        terminal: TERMINAL,
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

/// Autosave `text` into a new draft on `chart`, as the window's autosave would.
async fn autosave(c: &Client, chart: Uuid, text: &str) -> Uuid {
    let draft = Uuid::now_v7();
    let saved = save_draft(c, draft, chart, TERMINAL, text, None)
        .await
        .unwrap();
    assert_eq!(saved, SaveOutcome::Saved);
    draft
}

#[tokio::test]
async fn a_signed_note_is_attested_and_spends_its_draft() {
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
    let draft = autosave(&c, chart, "chest clear").await;

    let signed = sign_note(
        &mut c,
        &node_sk,
        &input(draft, chart, "chest clear"),
        &author,
    )
    .await
    .unwrap()
    .expect("a typed note on its own chart signs");

    let projected: i64 = c
        .query_one(
            "SELECT count(*) FROM clinical_note WHERE event_id = $1::text::uuid",
            &[&signed.event_id.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(projected, 1, "the signed event is the projected note");
    assert_eq!(signed.value, signed.event_id);
    assert!(
        open_drafts(&c, chart).await.unwrap().is_empty(),
        "signing spends the draft"
    );
    let list = list_patient_notes(&c, chart).await.unwrap();
    assert_eq!(list.notes.len(), 1);
    assert_eq!(list.notes[0].event_id, signed.event_id);
    assert_eq!(list.notes[0].text, "chest clear");
    assert_eq!(
        list.notes[0].grade,
        NoteGrade::Signed {
            by: common::NOTE_AUTHOR.into()
        }
    );
}

/// Review Focus 4: Sign races the last keystroke. The note is the text sent WITH Sign — here
/// text that looks like markup, which must sign as typed and read back as typed.
#[tokio::test]
async fn sign_takes_the_text_sent_with_it_not_the_last_autosave() {
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
    let draft = autosave(&c, chart, "BP <9").await;
    let typed = "BP <90, [L] arm (R)\n[ref]: discharge letter \\";

    sign_note(&mut c, &node_sk, &input(draft, chart, typed), &author)
        .await
        .unwrap()
        .expect("signs");

    let stored: String = c
        .query_one(
            "SELECT narrative FROM clinical_note WHERE patient_id = $1::text::uuid",
            &[&chart.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        stored,
        cairn_event::note::escape_note_markup(typed),
        "the signed narrative is the sent text, escaped once"
    );
    let list = list_patient_notes(&c, chart).await.unwrap();
    assert_eq!(list.notes[0].text, typed, "read back exactly as typed");
}

#[tokio::test]
async fn a_second_sign_of_the_same_draft_is_refused_spent_and_writes_one_note() {
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
    let draft = autosave(&c, chart, "abdomen soft").await;

    sign_note(
        &mut c,
        &node_sk,
        &input(draft, chart, "abdomen soft"),
        &author,
    )
    .await
    .unwrap()
    .expect("the first sign signs");
    let again = sign_note(
        &mut c,
        &node_sk,
        &input(draft, chart, "abdomen soft"),
        &author,
    )
    .await
    .unwrap();

    assert!(matches!(again, Err(SignRefusal::Spent)), "{again:?}");
    assert_eq!(note_count(&c, chart).await, 1, "one note, never two");
}

/// PR #931 review: the spend door names only the draft (db/065), so what keeps another
/// terminal — or a stale draft id on another chart — from signing a draft is that `sign_note`
/// SAVES before it spends, and the save answers `elsewhere`. This pins that ordering: either
/// sign is refused, the draft and its text are untouched, and no note is written anywhere.
#[tokio::test]
async fn a_sign_from_another_terminal_or_chart_is_refused_and_leaves_the_draft() {
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
    let draft = autosave(&c, chart, "held at the ward PC").await;
    let other_chart = Uuid::now_v7();

    let mut other_terminal = input(draft, chart, "overwritten elsewhere");
    other_terminal.terminal = "theatre-pc";
    for (who, attempt) in [
        ("another terminal", other_terminal),
        (
            "another chart",
            input(draft, other_chart, "on the wrong chart"),
        ),
    ] {
        let refused = sign_note(&mut c, &node_sk, &attempt, &author)
            .await
            .unwrap();
        assert!(
            matches!(refused, Err(SignRefusal::Elsewhere)),
            "{who}: {refused:?}"
        );
    }

    let drafts = open_drafts(&c, chart).await.unwrap();
    assert_eq!(drafts.len(), 1, "the draft is still open");
    assert_eq!(drafts[0].draft_id, draft);
    assert_eq!(drafts[0].terminal_id, TERMINAL);
    assert_eq!(
        drafts[0].narrative, "held at the ward PC",
        "its text is untouched"
    );
    assert_eq!(note_count(&c, chart).await, 0);
    assert_eq!(note_count(&c, other_chart).await, 0);
}

/// The node's HLC, read raw: the first thing a sign that reached the database would move.
async fn hlc(c: &Client) -> (i64, i32) {
    let r = c
        .query_one("SELECT hlc_wall, hlc_counter FROM hlc_state", &[])
        .await
        .unwrap();
    (r.get(0), r.get(1))
}

#[tokio::test]
async fn an_empty_note_is_refused_before_the_database() {
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
    let before = hlc(&c).await;

    let out = sign_note(
        &mut c,
        &node_sk,
        &input(Uuid::now_v7(), chart, " \n\t "),
        &author,
    )
    .await
    .unwrap();

    assert!(matches!(out, Err(SignRefusal::Empty)), "{out:?}");
    assert_eq!(hlc(&c).await, before, "no HLC was minted");
    assert!(
        open_drafts(&c, chart).await.unwrap().is_empty(),
        "no draft was saved"
    );
    assert_eq!(note_count(&c, chart).await, 0);
}

/// A sign the floor refuses rolls back WHOLE: the draft keeps its autosaved text and is still
/// unspent, so the clinician loses nothing and can sign again. The refusal is a real one —
/// the author's key was never enrolled, so `submit_event` refuses its signature ("not an
/// enrolled, non-revoked actor"), as it would for a key revoked between unlock and Sign.
#[tokio::test]
async fn a_failed_submit_leaves_the_draft_in_place() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (node_sk, _, human_sk, human_kid, chart) = common::note_setup(&c).await;
    let (stranger_sk, stranger_kid): (SigningKey, String) = generate_key().unwrap();
    let stranger = AuthorParams {
        human_sk: &stranger_sk,
        human_kid: &stranger_kid,
    };
    let draft = autosave(&c, chart, "first text").await;

    let refused = sign_note(
        &mut c,
        &node_sk,
        &input(draft, chart, "second text"),
        &stranger,
    )
    .await;

    let e = refused.expect_err("the floor refuses an unenrolled signer");
    assert!(
        format!("{e:#}").contains("not an enrolled"),
        "the refusal is the floor's: {e:#}"
    );
    let drafts = open_drafts(&c, chart).await.unwrap();
    assert_eq!(drafts.len(), 1, "the draft survives");
    assert_eq!(drafts[0].draft_id, draft);
    assert_eq!(
        drafts[0].narrative, "first text",
        "the sign's own save rolled back with it"
    );
    assert_eq!(note_count(&c, chart).await, 0);

    // And it is still signable by its real author.
    let author = AuthorParams {
        human_sk: &human_sk,
        human_kid: &human_kid,
    };
    sign_note(
        &mut c,
        &node_sk,
        &input(draft, chart, "second text"),
        &author,
    )
    .await
    .unwrap()
    .expect("the surviving draft signs");
    assert_eq!(note_count(&c, chart).await, 1);
}

/// Principle 4: no stated time is recorded as "writing began", never a fabricated time. The
/// envelope's t_effective is when typing started, and the note's recording_ended is the
/// draft's last REAL change — a Sign with unchanged text does not move it (ruling R1).
#[tokio::test]
async fn no_stated_time_signs_recording_started_as_t_effective() {
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
    let draft = autosave(&c, chart, "reviewed overnight").await;
    let saved = open_drafts(&c, chart).await.unwrap().remove(0);
    assert_eq!(saved.stated_time, None);

    sign_note(
        &mut c,
        &node_sk,
        &input(draft, chart, "reviewed overnight"),
        &author,
    )
    .await
    .unwrap()
    .expect("signs");

    let r = c
        .query_one(
            "SELECT effective_basis, t_effective, recording_started, recording_ended \
               FROM clinical_note WHERE patient_id = $1::text::uuid",
            &[&chart.to_string()],
        )
        .await
        .unwrap();
    let basis: String = r.get(0);
    let t_effective: chrono::DateTime<chrono::Utc> = r.get(1);
    let started: chrono::DateTime<chrono::Utc> = r.get(2);
    let ended: chrono::DateTime<chrono::Utc> = r.get(3);
    assert_eq!(basis, "recording-started");
    assert_eq!(t_effective, saved.recording_started);
    assert_eq!(started, saved.recording_started);
    assert_eq!(
        ended, saved.last_keystroke,
        "an unchanged text keeps its last keystroke"
    );
    let row = &list_patient_notes(&c, chart).await.unwrap().notes[0];
    assert_eq!(row.effective_basis, "recording-started");
    assert_eq!(row.t_effective, saved.recording_started);
    assert_eq!(row.recording_started, saved.recording_started);
}

/// A stated time is the note's t_effective, verbatim, and says so.
#[tokio::test]
async fn a_stated_time_signs_as_t_effective() {
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
    let draft = autosave(&c, chart, "seen at 03:00").await;
    let stated: chrono::DateTime<chrono::Utc> = "2026-11-03T03:00:00Z".parse().unwrap();
    let mut sent = input(draft, chart, "seen at 03:00");
    sent.stated = Some(stated);

    sign_note(&mut c, &node_sk, &sent, &author)
        .await
        .unwrap()
        .expect("signs");

    let row = &list_patient_notes(&c, chart).await.unwrap().notes[0];
    assert_eq!(row.effective_basis, "stated");
    assert_eq!(row.t_effective, stated);
}

/// Final review I1: a backward wall-clock step (an NTP step after a long offline spell) must
/// never make an honest note unsignable. The step is simulated by pushing the draft's times an
/// hour into the future, as the owner. Sign's own save of the changed text then stamps
/// `last_keystroke` no earlier than `recording_started`, so floor rule 8 (`recording_started`
/// not after `recording_ended`) passes and the note signs.
#[tokio::test]
async fn a_backward_clock_step_does_not_make_a_draft_unsignable() {
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
    let draft = autosave(&c, chart, "written before the step").await;
    c.execute(
        "UPDATE note_draft SET recording_started = clock_timestamp() + interval '1 hour', \
                last_keystroke = clock_timestamp() + interval '1 hour' \
          WHERE draft_id = $1::text::uuid",
        &[&draft.to_string()],
    )
    .await
    .unwrap();

    sign_note(
        &mut c,
        &node_sk,
        &input(draft, chart, "written after the step"),
        &author,
    )
    .await
    .unwrap()
    .expect("a clock step back must not refuse an honest note");

    let r = c
        .query_one(
            "SELECT recording_started, recording_ended FROM clinical_note \
              WHERE patient_id = $1::text::uuid",
            &[&chart.to_string()],
        )
        .await
        .unwrap();
    let started: chrono::DateTime<chrono::Utc> = r.get(0);
    let ended: chrono::DateTime<chrono::Utc> = r.get(1);
    assert_eq!(
        ended, started,
        "the end is held at the start, never before it"
    );
}
