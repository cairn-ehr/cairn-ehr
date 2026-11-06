//! Sign-in slice 3b-2b, Task 4 — the audited discard of a draft progress note
//! (`cairn_node::note::discard::discard_draft`).
//!
//! A discard is Sign's twin: in ONE transaction it saves the text on screen, asks db/069's door
//! to consume the draft, and submits a `clinical.note-discard.asserted` signed by the discarder,
//! whose `t_effective` is the instant the door minted under the draft's lock. These tests pin the
//! event it writes, that it carries the text on screen (Review Focus 4), that a colleague's
//! draft is refused and kept (Review Focus 3), that an all-blank draft is deleted with no event,
//! that a discarded draft answers `discarded` to a Sign, a save and a take, that a discard racing a
//! Sign has exactly one winner (Review Focus 1), that the instant follows a save that held the
//! lock, and that a raw door call with no event behind it is refused at COMMIT (Review Focus 2).
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard` taken BEFORE connecting.
//! Key material is minted at runtime (house rule 6).
mod common;

use cairn_event::generate_key;
use cairn_event::note_discard::NOTE_DISCARD_EVENT_TYPE;
use cairn_event::SigningKey;
use cairn_node::db;
use cairn_node::medication::AuthorParams;
use cairn_node::note::discard::{discard_draft, DiscardOutcome, DiscardRefusal};
use cairn_node::note::draft::{open_drafts, save_part, take_draft, SaveOutcome, TakeOutcome};
use cairn_node::note::sign::{sign_note, SignNoteInput, SignRefusal};
use chrono::{DateTime, Utc};
use tokio_postgres::Client;
use uuid::Uuid;

const ORIGIN: &str = "note-discard-test-node";
const TERMINAL: &str = "ward-3-pc";

/// What the window sends with Discard (the same input as Sign's), as the fixture's clinician.
fn input(draft: Uuid, patient: Uuid, narrative: &str) -> SignNoteInput<'_> {
    SignNoteInput {
        draft,
        patient,
        terminal: TERMINAL,
        writer: Some("notewriter"),
        narrative,
        stated: None,
        node_origin: ORIGIN,
        addendum_to: None,
        also_vouches: &[],
    }
}

async fn save_as(c: &Client, draft: Uuid, chart: Uuid, writer: Option<&str>, text: &str) {
    let saved = save_part(c, draft, chart, TERMINAL, writer, text, None, None)
        .await
        .unwrap();
    assert_eq!(saved, SaveOutcome::Saved, "{writer:?} saves {text:?}");
}

/// Enrol `Nurse A` (`nurse-a`) with a fresh key, as `note_sign_parts.rs` does.
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

async fn count(c: &Client, sql: &str, chart: Uuid) -> i64 {
    c.query_one(sql, &[&chart.to_string()])
        .await
        .unwrap()
        .get(0)
}

async fn discard_count(c: &Client, chart: Uuid) -> i64 {
    count(
        c,
        "SELECT count(*) FROM event_log WHERE event_type = 'clinical.note-discard.asserted' \
           AND patient_id = $1::text::uuid",
        chart,
    )
    .await
}

async fn note_count(c: &Client, chart: Uuid) -> i64 {
    count(
        c,
        "SELECT count(*) FROM clinical_note WHERE patient_id = $1::text::uuid",
        chart,
    )
    .await
}

/// One discard event as stored: type, signer, clear payload, envelope time, and the spend's
/// `spent_at` (the door's `discarded_at`).
struct Stored {
    event_type: String,
    signer: String,
    payload: serde_json::Value,
    t_effective: DateTime<Utc>,
    spent_at: DateTime<Utc>,
}

async fn stored(c: &Client, event: Uuid) -> Stored {
    let r = c
        .query_one(
            "SELECT e.event_type, e.signer_key_id, cairn_clear_payload(e)::text, e.t_effective, \
                    s.spent_at \
               FROM event_log e JOIN note_draft_spent s ON s.event_id = e.event_id \
              WHERE e.event_id = $1::text::uuid",
            &[&event.to_string()],
        )
        .await
        .unwrap();
    Stored {
        event_type: r.get(0),
        signer: r.get(1),
        payload: serde_json::from_str(&r.get::<_, String>(2)).unwrap(),
        t_effective: r.get(3),
        spent_at: r.get(4),
    }
}

/// The event a `Recorded` discard names.
fn recorded(out: anyhow::Result<Result<DiscardOutcome, DiscardRefusal>>) -> Uuid {
    match out.unwrap() {
        Ok(DiscardOutcome::Recorded(c)) => {
            assert_eq!(c.event_id, c.value, "one event, named twice");
            c.value
        }
        other => panic!("expected Recorded, got {other:?}"),
    }
}

#[tokio::test]
async fn discarding_own_draft_records_one_signed_event() {
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
    // A part by a writer not known, then the discarder's own: both are the discarder's to throw.
    save_as(&c, draft, chart, None, "found on the floor").await;

    let event =
        recorded(discard_draft(&mut c, &node_sk, &input(draft, chart, "BP <90"), &author).await);

    let s = stored(&c, event).await;
    assert_eq!(s.event_type, NOTE_DISCARD_EVENT_TYPE);
    assert_eq!(s.signer, human_kid, "signed by the discarder");
    let parts = s.payload["parts"].as_array().unwrap();
    assert_eq!(parts.len(), 2, "{parts:?}");
    assert_eq!(parts[0]["writer"], "not-known");
    assert_eq!(parts[0]["narrative"], "found on the floor");
    assert_eq!(parts[1]["writer"], "discarder");
    assert_eq!(
        parts[1]["narrative"], "BP \\<90",
        "escaped once, as a note is"
    );
    assert_eq!(parts[1]["stated_time"], serde_json::Value::Null);
    // Controller ruling 4: the envelope time is the door's discarded_at, stored verbatim.
    assert_eq!(s.t_effective, s.spent_at);
    assert!(
        open_drafts(&c, chart).await.unwrap().is_empty(),
        "the draft is gone"
    );
    assert_eq!(note_count(&c, chart).await, 0, "a discard is never a note");
}

/// Review Focus 4: Discard pressed during the debounce carries what is on screen, not the last
/// autosave.
#[tokio::test]
async fn discard_carries_the_text_on_screen() {
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
    save_as(&c, draft, chart, Some("notewriter"), "a").await;

    let event =
        recorded(discard_draft(&mut c, &node_sk, &input(draft, chart, "ab"), &author).await);

    let s = stored(&c, event).await;
    assert_eq!(s.payload["parts"][0]["narrative"], "ab");
    assert_eq!(s.payload["parts"].as_array().unwrap().len(), 1);
}

/// Review Focus 3: Nurse A typed; Dr Ada (a different key) presses Discard. Refused, and nothing
/// is written — not even Dr Ada's own save of the text on screen.
#[tokio::test]
async fn a_colleagues_draft_is_refused_not_yours_and_kept() {
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

    let out = discard_draft(&mut c, &node_sk, &input(draft, chart, "mine"), &author)
        .await
        .unwrap();

    assert!(matches!(out, Err(DiscardRefusal::NotYours)), "{out:?}");
    let drafts = open_drafts(&c, chart).await.unwrap();
    assert_eq!(drafts.len(), 1, "the draft is kept");
    assert_eq!(
        drafts[0].parts.len(),
        1,
        "the refused discard's save rolled back"
    );
    assert_eq!(drafts[0].parts[0].narrative, "obs stable");
    assert_eq!(discard_count(&c, chart).await, 0);
}

/// Design §3.5: nothing to audit, so no event and no spend — but the deletion commits.
#[tokio::test]
async fn an_all_blank_draft_is_empty_and_gone() {
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
    save_as(&c, draft, chart, Some("notewriter"), "x").await;

    let out = discard_draft(&mut c, &node_sk, &input(draft, chart, "  \n "), &author)
        .await
        .unwrap();

    assert!(matches!(out, Ok(DiscardOutcome::Empty)), "{out:?}");
    assert!(
        open_drafts(&c, chart).await.unwrap().is_empty(),
        "deleted, and it stuck"
    );
    assert_eq!(discard_count(&c, chart).await, 0, "no event");
    let spends: i64 = c
        .query_one(
            "SELECT count(*) FROM note_draft_spent WHERE draft_id = $1::text::uuid",
            &[&draft.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(spends, 0, "no spend recorded");
}

/// After a discard, a Sign, a second discard, a late save and a take all answer `discarded`,
/// never `spent` (which the window words as "already signed").
#[tokio::test]
async fn sign_after_discard_is_refused_discarded() {
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
    save_as(&c, draft, chart, Some("notewriter"), "abdomen soft").await;
    recorded(
        discard_draft(
            &mut c,
            &node_sk,
            &input(draft, chart, "abdomen soft"),
            &author,
        )
        .await,
    );

    let sign = sign_note(
        &mut c,
        &node_sk,
        &input(draft, chart, "abdomen soft"),
        &author,
    )
    .await
    .unwrap();
    assert!(matches!(sign, Err(SignRefusal::Discarded)), "{sign:?}");
    let again = discard_draft(
        &mut c,
        &node_sk,
        &input(draft, chart, "abdomen soft"),
        &author,
    )
    .await
    .unwrap();
    assert!(matches!(again, Err(DiscardRefusal::Discarded)), "{again:?}");
    let save = save_part(&c, draft, chart, TERMINAL, None, "late", None, None)
        .await
        .unwrap();
    assert_eq!(save, SaveOutcome::Discarded);
    let take = take_draft(&c, draft, chart, "pc-2").await.unwrap();
    assert_eq!(take, TakeOutcome::Discarded);
    assert_eq!(note_count(&c, chart).await, 0);
    assert_eq!(discard_count(&c, chart).await, 1);
}

/// Review Focus 1: a Discard and a Sign of one draft on two connections at once. Exactly one
/// wins; the loser is refused by the winner's word and writes nothing.
#[tokio::test]
async fn discard_racing_sign_one_wins() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let mut other = db::connect(&base).await.unwrap();
    let (node_sk, _, human_sk, human_kid, chart) = common::note_setup(&c).await;
    let author = AuthorParams {
        human_sk: &human_sk,
        human_kid: &human_kid,
    };
    let draft = Uuid::now_v7();
    save_as(&c, draft, chart, Some("notewriter"), "seen").await;
    let sent = input(draft, chart, "seen");

    let (discarded, signed) = tokio::join!(
        discard_draft(&mut c, &node_sk, &sent, &author),
        sign_note(&mut other, &node_sk, &sent, &author)
    );

    match (discarded.unwrap(), signed.unwrap()) {
        (Ok(DiscardOutcome::Recorded(_)), Err(SignRefusal::Discarded)) => {
            assert_eq!(
                (discard_count(&c, chart).await, note_count(&c, chart).await),
                (1, 0)
            );
        }
        (Err(DiscardRefusal::Spent), Ok(_)) => {
            assert_eq!(
                (discard_count(&c, chart).await, note_count(&c, chart).await),
                (0, 1)
            );
        }
        other => panic!("exactly one must win: {other:?}"),
    }
}

/// Controller ruling 4: the discard instant is minted AFTER the draft's lock, so a save that held
/// the lock while the discard waited never stamps a keystroke later than the discard.
#[tokio::test]
async fn discarded_at_follows_a_save_that_held_the_lock() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let mut other = db::connect(&base).await.unwrap();
    let (node_sk, _, human_sk, human_kid, chart) = common::note_setup(&c).await;
    let author = AuthorParams {
        human_sk: &human_sk,
        human_kid: &human_kid,
    };
    let draft = Uuid::now_v7();
    save_as(&c, draft, chart, Some("notewriter"), "a").await;
    // Connection A holds the draft's lock (its save took it) until it commits.
    let holder = other.transaction().await.unwrap();
    let held = save_part(
        &holder,
        draft,
        chart,
        TERMINAL,
        Some("notewriter"),
        "ab",
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(held, SaveOutcome::Saved);

    // B's discard queues behind the lock; A then saves again (a keystroke stamped while B waits)
    // and commits. B sends the same text, so its own save stamps nothing new.
    let sent = input(draft, chart, "abc");
    let (out, ()) = tokio::join!(discard_draft(&mut c, &node_sk, &sent, &author), async {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let again = save_part(
            &holder,
            draft,
            chart,
            TERMINAL,
            Some("notewriter"),
            "abc",
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(again, SaveOutcome::Saved);
        holder.commit().await.unwrap();
    });

    let s = stored(&c, recorded(out)).await;
    let ended: DateTime<Utc> = s.payload["parts"][0]["recording_ended"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!(
        s.t_effective >= ended,
        "discarded at {} before the last keystroke {ended}",
        s.t_effective
    );
}

/// Review Focus 2, from Rust: the door's yes is not a discard. A raw call naming the
/// discarder's key, with no event behind it, consumes the draft — and a TRUE commit refuses it,
/// so the draft is still there.
#[tokio::test]
async fn a_raw_door_call_without_its_event_is_refused_at_commit() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (_, _, _, human_kid, chart) = common::note_setup(&c).await;
    let draft = Uuid::now_v7();
    save_as(&c, draft, chart, Some("notewriter"), "seen").await;

    let tx = c.transaction().await.unwrap();
    let word: String = tx
        .query_one(
            "SELECT outcome FROM cairn_note_draft_discard($1::text::uuid, $2::text::uuid, $3, \
                    $4::text::uuid, $5)",
            &[
                &draft.to_string(),
                &chart.to_string(),
                &TERMINAL,
                &Uuid::now_v7().to_string(),
                &human_kid,
            ],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(word, "ok", "the door says yes");
    tx.commit()
        .await
        .expect_err("no event signed by that key: the COMMIT refuses");

    assert_eq!(
        open_drafts(&c, chart).await.unwrap().len(),
        1,
        "the draft is still there"
    );
}

/// PR #984 review: `discarded_at_follows_a_save_that_held_the_lock` cannot tell WHERE db/069 reads
/// its clock, because `discard_draft` saves first and that save takes the same lock. A raw door
/// call takes nothing before the door does, so here only the door's own lock can make it wait: the
/// instant it mints (signed into the event's `t_effective`) must come after the holder lets go —
/// a clock read before `pg_advisory_xact_lock` would be stamped ~300 ms earlier.
#[tokio::test]
async fn a_raw_door_call_reads_its_clock_after_the_drafts_lock() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let mut other = db::connect(&base).await.unwrap();
    let (_, _, _, human_kid, chart) = common::note_setup(&c).await;
    let draft = Uuid::now_v7();
    save_as(&c, draft, chart, Some("notewriter"), "seen").await;

    // A holds the draft's lock (the key every draft door takes, db/066) until it commits.
    let holder = other.transaction().await.unwrap();
    holder
        .execute(
            "SELECT pg_advisory_xact_lock(hashtextextended('note_draft:' || $1, 0))",
            &[&draft.to_string()],
        )
        .await
        .unwrap();

    let tx = c.transaction().await.unwrap();
    let (minted, released) = tokio::join!(
        async {
            let row = tx
                .query_one(
                    "SELECT max(discarded_at) FROM cairn_note_draft_discard($1::text::uuid, \
                            $2::text::uuid, $3, $4::text::uuid, $5)",
                    &[
                        &draft.to_string(),
                        &chart.to_string(),
                        &TERMINAL,
                        &Uuid::now_v7().to_string(),
                        &human_kid,
                    ],
                )
                .await
                .unwrap();
            row.get::<_, DateTime<Utc>>(0)
        },
        async {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            let at: DateTime<Utc> = holder
                .query_one("SELECT clock_timestamp()", &[])
                .await
                .unwrap()
                .get(0);
            holder.commit().await.unwrap();
            at
        }
    );
    assert!(
        minted > released,
        "the discard instant {minted} must follow the lock's release {released}"
    );
    // No event was signed: the COMMIT would refuse it. Nothing is kept.
    tx.rollback().await.unwrap();
}
